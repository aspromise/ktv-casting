use log::{debug, error, info};
use reqwest::Client;
use serde_json::json;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::sleep;
use crate::messages::Message;
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone)]
pub struct PlaylistManager {
    url: String,
    room_id: u64,
    hash: Arc<Mutex<Option<String>>>,
    playlist: Arc<Mutex<Vec<String>>>,
    song_playing: Arc<Mutex<Option<String>>>,
    tx: Option<UnboundedSender<Message>>,
}

impl PlaylistManager {
    pub fn new(url: &str, room_id: u64, playlist: Arc<Mutex<Vec<String>>>, tx: Option<UnboundedSender<Message>>) -> Self {
        Self {
            url: url.to_string(),
            room_id,
            hash: Arc::new(Mutex::new(None)),
            playlist,
            song_playing: Arc::new(Mutex::new(None)),
            tx,
        }
    }

    async fn fetch_playlist(&mut self) -> Result<Option<String>, String> {
        let client = Client::builder()
            .use_rustls_tls()
            .build()
            .map_err(|e| format!("创建HTTP客户端失败: {}", e))?;

        let hash_guard = self.hash.lock().await;
        let last_hash = hash_guard.clone().unwrap_or("EMPTY_LIST_HASH".to_string());
        drop(hash_guard); // 释放锁，避免长时间持有

        let url = format!(
            "{}/api/songListInfo?roomId={}&lastHash={}",
            self.url, self.room_id, last_hash
        );

        debug!("正在获取播放列表: {}", url);

        let resp = client
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
        let changed: bool = resp_json["changed"].as_bool().unwrap_or(false);

        if !changed {
            debug!("播放列表未改变，跳过更新");
            return Ok(self.song_playing.lock().await.clone());
        }

        // 获取新的 hash 值
        let new_hash = resp_json["hash"]
            .as_str()
            .unwrap_or("EMPTY_LIST_HASH")
            .to_string();

        let extract_bv_function = |url: &str| {
            // 提取 bilibili://video/ 后面的部分
            if let Some(start) = url.find("bilibili://video/") {
                let after_prefix = &url[start + "bilibili://video/".len()..];
                after_prefix.to_string().replace("?", "-").replace("=", "") // 替换问号和等号，避免DLNA设备不支持
            } else {
                url.to_string().replace("?", "-").replace("=", "")
            }
        };

        // 从 list 数组中提取待播歌单 URL
        let urls: Vec<String> = if let Some(list_array) = resp_json["list"].as_array() {
            list_array
                .iter()
                .filter(|item| {
                    item.get("state")
                        .is_none_or(|s| s.as_str().unwrap_or("") != "sung")
                })
                .filter_map(|item| item["url"].as_str())
                .map(extract_bv_function)
                .collect()
        } else {
            Vec::new()
        };

        // 从 list 数组中提取最后一条状态为 “sung” 的歌单 URL
        let sung_url: Option<String> = if let Some(list_array) = resp_json["list"].as_array() {
            list_array
                .iter()
                .rev() // 反转迭代器，从后往前找
                .find(|item| {
                    item.get("state")
                        .is_some_and(|s| s.as_str().unwrap_or("") == "sung")
                })
                .and_then(|item| item["url"].as_str()) // 把 &str 转为 Option<&str>
                .map(extract_bv_function) // 如果你需要把 url 处理成 bv 等
        } else {
            None
        };

        info!("获取到 {} 个URL，新的hash: {}", urls.len(), new_hash);

        // 打印每个URL用于调试
        for (i, url) in urls.iter().enumerate() {
            debug!("  {}. {}", i + 1, url);
        }

        // 更新播放列表
        let mut playlist = self.playlist.lock().await;
        playlist.clear();
        playlist.extend(urls);
        drop(playlist); // 释放锁，避免长时间持有

        // 更新当前歌曲
        let mut song_playing = self.song_playing.lock().await;
        *song_playing = sung_url.clone();
        drop(song_playing);

        // 更新 hash 值
        let mut hash = self.hash.lock().await;
        *hash = Some(new_hash);
        drop(hash); // 释放锁

        Ok(sung_url)
    }

    pub fn start_periodic_update<F>(&self, f_on_update: F)
    where
        F: Fn(String) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + 'static,
    {
        let mut self_clone = self.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(300));
            let mut song_playing: Option<String> = None;
            loop {
                interval.tick().await;
                match self_clone.fetch_playlist().await {
                    Err(e) => error!("定时更新播放列表失败: {}", e),
                    Ok(song_playing_new) => {
                        if song_playing_new != song_playing {
                            // 发送CurrentSong消息更新UI
                            if let Some(ref tx) = tx {
                                if let Some(ref song) = song_playing_new {
                                    // 从URL中提取歌曲名称（BV号）
                                    let song_name = song.split('-').next().unwrap_or(song);
                                    let _ = tx.send(Message::CurrentSong(song_name.to_string()));
                                } else {
                                    let _ = tx.send(Message::CurrentSong(String::from("无")));
                                }
                            }
                            
                            if let Some(url) = song_playing_new.clone() {
                                f_on_update(url).await; // await the future
                            }
                            song_playing = song_playing_new;
                        }
                    }
                }
            }
        });
    }

    pub async fn next_song(&mut self) -> Result<(), String> {
        let url = format!("{}/api/nextSong?roomId={}", self.url, self.room_id);
        let temp_hash = self
            .hash
            .lock()
            .await
            .clone()
            .unwrap_or_else(|| "EMPTY_LIST_HASH".to_string());
        let client = Client::builder()
            .use_rustls_tls()
            .build()
            .map_err(|e| format!("创建HTTP客户端失败: {}", e))?;
        let resp = client
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
        self.fetch_playlist().await?;

        Ok(())
    }
}
