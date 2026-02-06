use log::{debug, error, info, warn};
use reqwest::Client;
use serde_json::json;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::{sleep, Interval};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use futures_util::{SinkExt, StreamExt};
use crate::utils::extract_bv_id;

#[derive(Clone)]
pub struct PlaylistManager {
    url: String,
    room_id: String,
    nickname: String,
    hash: Arc<Mutex<Option<String>>>,
    song_playing: Arc<Mutex<Option<String>>>,
    on_song_change: Arc<Mutex<Option<Arc<dyn Fn(String) + Send + Sync>>>>,
    client: Client,
}

impl PlaylistManager {
    pub fn new(url: &str, room_id: String, nickname: Option<String>) -> Self {
        let client = Client::builder()
            .use_rustls_tls()
            .build()
            .expect("Failed to create HTTP client");
        
        Self {
            url: url.to_string(),
            room_id,
            nickname: nickname.unwrap_or_else(|| "ktv-casting".to_string()),
            hash: Arc::new(Mutex::new(None)),
            song_playing: Arc::new(Mutex::new(None)),
            on_song_change: Arc::new(Mutex::new(None)),
            client,
        }
    }

    /// 设置歌曲变化回调函数（异步版本）
    pub async fn set_on_song_change<F>(&self, callback: F)
    where
        F: Fn(String) + Send + Sync + 'static,
    {
        let mut on_song_change = self.on_song_change.lock().await;
        *on_song_change = Some(Arc::new(callback));
    }

    /// 启动WebSocket连接并监听（包含自动重连）
    pub async fn start_websocket_listener(self: Arc<Self>) -> Result<(), String> {
        let mut backoff = 1;
        
        loop {
            match Arc::clone(&self).connect_websocket_internal().await {
                Ok(_) => {
                    info!("WebSocket连接成功");
                    backoff = 1; // 重置退避
                }
                Err(e) => {
                    warn!("WebSocket连接失败: {}，{}秒后重试", e, backoff);
                    sleep(Duration::from_secs(backoff)).await;
                    backoff = (backoff * 2).min(60); // 指数退避，最大60秒
                    continue;
                }
            }

            // 连接成功后，等待断开后再重连
            break;
        }

        Ok(())
    }

    /// 内部连接方法（不包含重连逻辑）
    async fn connect_websocket_internal(self: Arc<Self>) -> Result<(), String> {
        // 从HTTP URL构建WebSocket URL
        // 例如：https://ktv.starfreedomx.top -> wss://ktv.starfreedomx.top
        let ws_protocol = if self.url.starts_with("https://") { "wss:" } else { "ws:" };
        
        // 提取主机部分（去除协议）
        let host_part = if self.url.starts_with("http://") {
            &self.url[7..] // 跳过 "http://"
        } else if self.url.starts_with("https://") {
            &self.url[8..] // 跳过 "https://"
        } else {
            &self.url
        };
        
        let ws_url = format!("{}//{}/api/ws?roomId={}&nickname={}", 
            ws_protocol, 
            host_part, 
            self.room_id,
            urlencoding::encode(&self.nickname)
        );
        
        info!("正在连接到WebSocket: {}", ws_url);

        let (ws_stream, _) = connect_async(&ws_url)
            .await
            .map_err(|e| format!("WebSocket连接失败: {}", e))?;

        info!("WebSocket连接成功，开始监听消息...");

        let self_for_init = self.clone();
        tokio::spawn(async move {
            info!("执行初次同步...");
            // 触发 HTTP 获取
            self_for_init.handle_update().await;
        });
        // 启动消息监听任务
        tokio::spawn(async move {
            Self::message_listener(self, ws_stream).await;
        });

        Ok(())
    }

    /// 消息监听循环
    async fn message_listener(
        self: Arc<Self>,
        mut ws_stream: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    ) {
        let mut ping_interval: Interval = tokio::time::interval(Duration::from_secs(30));
        let mut last_pong_time = std::time::Instant::now();

        loop {
            tokio::select! {
                msg = ws_stream.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            debug!("收到WebSocket消息: {}", text);
                            
                            // 处理心跳响应
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text)
                                && json.get("type").and_then(|t| t.as_str()) == Some("pong") {
                                    last_pong_time = std::time::Instant::now();
                                    debug!("收到pong响应");
                                    continue;
                                }

                            // 处理UPDATE消息
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text)
                                && let Some(msg_type) = json.get("type").and_then(|t| t.as_str())
                                && msg_type == "UPDATE"
                                && let Some(hash) = json.get("hash").and_then(|h| h.as_str()) {
                                let old_hash = self.hash.lock().await.clone();
                                    // 如果hash发生变化，获取当前播放的歌曲
                                    if old_hash.as_deref() != Some(&hash) {
                                        info!("检测到歌单更新，hash: {}", hash);
                                        self.handle_update().await;
                                }
                            }
                        }
                        Some(Ok(Message::Ping(data))) => {
                            debug!("收到ping，发送pong");
                            if ws_stream.send(Message::Pong(data)).await.is_err() {
                                warn!("发送pong失败");
                                break;
                            }
                        }
                        Some(Ok(Message::Pong(_))) => {
                            last_pong_time = std::time::Instant::now();
                            debug!("收到pong");
                        }
                        Some(Ok(Message::Close(_))) => {
                            info!("WebSocket连接已关闭");
                            break;
                        }
                        Some(Err(e)) => {
                            warn!("WebSocket错误: {}", e);
                            break;
                        }
                        None => {
                            info!("WebSocket流结束");
                            break;
                        }
                        _ => {}
                    }
                }
                _ = ping_interval.tick() => {
                    // 定时发送ping并检查连接状态
                    let now = std::time::Instant::now();
                    if now.duration_since(last_pong_time) > Duration::from_secs(60) {
                        warn!("超过60秒未收到pong，连接可能已断开");
                        break;
                    }

                    if ws_stream.send(Message::Ping(vec![])).await.is_err() {
                        warn!("发送ping失败，连接可能已断开");
                        break;
                    }
                    debug!("发送ping");
                }
            }
        }

        info!("WebSocket监听结束");
    }

    /// 处理UPDATE消息
    async fn handle_update(&self) {
        let old_hash = self.hash.lock().await.clone();

        let last_hash_for_request = old_hash.unwrap_or_else(|| "EMPTY_LIST_HASH".to_string());
        // 调用HTTP接口获取完整歌单信息
        if let Ok((Some(song_url), latest_hash)) = self.fetch_current_song_from_hash(&last_hash_for_request).await {
            let mut song_playing = self.song_playing.lock().await;
            let old_song = song_playing.clone();
            *song_playing = Some(song_url.clone());
            drop(song_playing);

            // 更新 Hash
            let mut hash_guard = self.hash.lock().await;
            *hash_guard = Some(latest_hash);
            drop(hash_guard);

            if old_song != Some(song_url.clone()) {
                info!("歌曲已切换为: {}", song_url);
                if let Some(callback) = self.on_song_change.lock().await.as_ref() {
                    callback(song_url);
                }
            }
        }
    }

    /// 根据hash获取当前播放的歌曲（通过HTTP接口）
    async fn fetch_current_song_from_hash(&self, hash: &str) -> Result<(Option<String>, String), String> {
        let url = format!(
            "{}/api/songListInfo?roomId={}&lastHash={}",
            self.url, self.room_id, hash
        );

        debug!("获取当前歌曲: {}", url);

        let resp = self.client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("发送请求失败: {}", e))?;
        
        if !resp.status().is_success() {
            return Err(format!("请求失败，状态码: {}", resp.status()));
        }

        let resp_json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("解析JSON失败: {}", e))?;

        let latest_hash: String = resp_json["hash"].as_str().map(|s| s.to_string()).unwrap_or_else(|| hash.to_string());

        if !resp_json["changed"].as_bool().unwrap_or(false) {
            return Ok((None, latest_hash));
        }

        // 提取正在演唱的歌曲
        let singing_url: Option<String> = resp_json["list"]["singing"]
            .as_object()
            .and_then(|_| resp_json["list"]["singing"]["url"].as_str().map(extract_bv_id))
            .or_else(|| {
                resp_json["list"]["sung"]
                    .as_array()
                    .and_then(|arr| arr.last())
                    .and_then(|last| last["url"].as_str())
                    .map(extract_bv_id)
            });



        Ok((singing_url, latest_hash))
    }

    /// 请求下一首歌曲（HTTP接口）
    pub async fn next_song(&self) -> Result<(), String> {
        let url = format!("{}/api/nextSong?roomId={}", self.url, self.room_id);
        let temp_hash = self
            .hash
            .lock()
            .await
            .clone()
            .unwrap_or_else(|| "EMPTY_LIST_HASH".to_string());
        
        let resp = self.client
            .post(&url)
            .json(&json!({"idArrayHash": temp_hash}))
            .send()
            .await
            .map_err(|e| format!("发送请求失败: {}", e))?;
        
        let resp_json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("解析JSON失败: {}", e))?;

        if !resp_json["success"].as_bool().unwrap_or(false) {
            return Err(format!("请求失败: {}", resp_json));
        }

        info!("成功请求下一首歌曲");
        Ok(())
    }

    /// 获取当前播放的歌曲
    pub async fn get_song_playing(&self) -> Option<String> {
        self.song_playing.lock().await.clone()
    }

    /// 获取当前hash
    pub async fn get_hash(&self) -> Option<String> {
        self.hash.lock().await.clone()
    }

    /// 遗留的轮询方法（当WebSocket不可用时使用）
    pub fn start_periodic_update_legacy(&self) {
        let self_clone = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(300));
            loop {
                interval.tick().await;
                self_clone.handle_update().await;
            }
        });
    }
}

