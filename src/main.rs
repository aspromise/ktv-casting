use crate::dlna_controller::DlnaController;
use actix_web::{App, HttpServer, web};
use anyhow::{Context, Result, bail};
use regex::Regex;
use local_ip_address::local_ip;
use log::{debug, error, info};
use playlist_manager::PlaylistManager;
use reqwest::Client;
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::sleep;
use url::{Position, Url};
use crate::utils::retry_until_success;

mod bilibili_parser;
mod dlna_controller;
mod media_server;
mod mp4_util;
mod playlist_manager;
mod utils;

pub struct SharedState {
    pub duration_cache: Arc<Mutex<std::collections::HashMap<String, u32>>>,
}

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("RUST_LOG").is_err() {
        unsafe {
            std::env::set_var("RUST_LOG", "INFO");
        }
    }
    env_logger::init();

    println!("=== KTV投屏DLNA应用启动 ===");
    println!("输入房间链接，如 http://127.0.0.1:1145/102 或 https://ktv.example.com/102");
    let mut input = String::new();
    io::stdin().read_line(&mut input).expect("无法读取输入");
    let url_str = input.trim();
    let mut normalized_url = url_str.to_string();
    if !normalized_url.contains("://") && !normalized_url.is_empty() {
        normalized_url = format!("http://{}", normalized_url);
    }
    // ② 使用 url crate 解析并提取 base URL 与 room_id
    let parsed_url = Url::parse(&normalized_url).with_context(|| "无法解析 URL")?;

    let base_url = parsed_url[..Position::AfterPort].to_string();
    info!("Base URL: {}", base_url);

    // ③ 从路径中取最后一段（非空）作为 room_id
    // 尝试从不同的地方获取 room_id
    let room_id_result: Option<String> = {
        // 优先尝试从查询参数 ?roomId=123 中获取
        let query_room_id = parsed_url.query_pairs()
            .find(|(key, _)| key == "roomId")
            .map(|(_, value)| value.into_owned());

        if query_room_id.is_some() {
            query_room_id
        } else {
            // 如果 Query 里没有，回退到原来的路径末尾逻辑
            parsed_url.path_segments()
                .and_then(|s| s.filter(|seg| !seg.is_empty()).last())
                .map(|s| s.to_string())
        }
    };
    // 检查结果
    let room_str = room_id_result.with_context(|| "URL 中未找到房间号 (roomId 参数或路径末尾)")?;
    let re = Regex::new(r"^[a-zA-Z0-9_-]{1,20}$")?;

    if !re.is_match(&room_str) {
        bail!("无效的房间号格式: '{}' (仅限 1-20 位字母、数字、下划线或连字符)", room_str);
    }
    let room_id: String = room_str.to_string();
    info!("Parsed room_id: {}", room_id);

    // 询问用户昵称（可选）
    println!("输入您的昵称（直接回车使用默认值 'ktv-casting'）：");
    input.clear();
    io::stdin().read_line(&mut input).expect("无法读取输入");
    let nickname = input.trim().to_string();
    let nickname = if nickname.is_empty() { None } else { Some(nickname) };

    let server_port = 8080;
    let playlist_manager = Arc::new(PlaylistManager::new(&base_url, room_id.clone(), nickname.clone()));

    let duration_cache = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let shared_state = web::Data::new(SharedState {
        duration_cache: duration_cache.clone(),
    });

    // 1. 创建 Reqwest Client
    let client = Client::builder()
        .use_rustls_tls()
        .build()
        .expect("Failed to create client");

    let client_data = web::Data::new(client);

    // 2. 配置 HttpServer，运行
    let server = HttpServer::new(move || {
        App::new()
            .app_data(client_data.clone())
            .app_data(shared_state.clone())
            .service(media_server::proxy_handler)
    })
    .bind(("0.0.0.0", server_port))?
    .run();

    let local_ip = local_ip()?;
    let controller = DlnaController::new();
    let devices = controller.discover_devices().await?;
    if devices.is_empty() {
        bail!("No DLNA Devices");
    }
    println!("发现以下DLNA设备：");
    println!("编号: 设备名称 at 设备地址");
    for (i, device) in devices.iter().enumerate() {
        println!("{}: {} at {}", i, device.friendly_name, device.location);
    }
    println!("输入设备编号：");
    input.clear();
    io::stdin().read_line(&mut input).expect("读取编号失败");
    let device_num: usize = input.trim().parse()?;
    if device_num > devices.len() {
        bail!("编号有误");
    }
    let device = devices[device_num].clone(); // clone owned copy
    let device_cloned = device.clone();

    // 设置歌曲变化回调（需要克隆controller和device）
    let controller_for_callback = controller.clone();
    let device_for_callback = device.clone();
    let callback_pm = playlist_manager.clone();

    callback_pm.set_on_song_change(move |url| {
        let controller = controller_for_callback.clone();
        let device = device_for_callback.clone();
        tokio::spawn(async move {
            // 停止当前播放
            retry_until_success("停止播放", 500, || async {
                controller.stop(&device).await.map_err(|e| e.to_string())
            }).await.ok();

            // 设置AVTransport URI
            retry_until_success("设置AVTransport URI", 500, || async {
                controller
                    .set_avtransport_uri(&device, &url, "", local_ip, server_port)
                    .await
                    .map_err(|e| e.to_string())
            }).await.ok();

            // 播放
            retry_until_success("播放", 500, || async {
                controller.play(&device).await.map_err(|e| e.to_string())
            }).await.ok();
        });
    }).await;

    // 启动WebSocket监听（需要克隆playlist_manager）
    let pm_ws = playlist_manager.clone();
    match pm_ws.start_websocket_listener().await {
        Ok(_) => info!("WebSocket监听已启动"),
        Err(e) => {
            error!("WebSocket连接失败: {}，将退回到轮询模式", e);
            // 如果WebSocket连接失败，退回到轮询模式
            playlist_manager.start_periodic_update_legacy();
        }
    }
    let pm_for_monitor = playlist_manager.clone();
    tokio::spawn(async move {
        let controller = DlnaController::new();
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        let mut current_secs: u32 = 0;
        let mut total_secs: u32 = 0;
        loop {
            interval.tick().await;

            // 首先尝试从缓存中获取总长度
            let mut cached_total = 0;
            if let Some(playing) = pm_for_monitor.get_song_playing().await {
                let cache = duration_cache.lock().await;
                if let Some(&d) = cache.get(&playing) {
                    cached_total = d;
                }
            }

            // 使用重试逻辑获取播放进度
            let result = retry_until_success("获取播放进度", 500, || async {
                controller.get_secs(&device_cloned).await.map_err(|e| e.to_string())
            }).await;

            match result {
                Ok((current, _)) => {
                    current_secs = current;

                    // 如果从缓存拿到了长度，
                    if cached_total > 0 {
                        total_secs = cached_total;
                        info!("使用缓存的视频时长: {}s", total_secs);
                    }

                    let remaining_secs = total_secs.saturating_sub(current_secs);

                    info!(
                        "获取播放进度成功，当前时间{}秒，总时间{}秒，剩余时间{}秒",
                        current_secs, total_secs, remaining_secs
                    );

                    if remaining_secs <= 2 && total_secs > 0 {
                        info!(
                            "剩余时间{}秒，总时间{}秒，准备切歌",
                            remaining_secs, total_secs
                        );
                        // 重试next_song
                        retry_until_success("下一首歌曲", 500, || async {
                            pm_for_monitor.next_song().await.map_err(|e| e.to_string())
                        }).await.ok();
                        sleep(Duration::from_secs(5)).await;
                    }
                }
                Err(e) => {
                    error!("获取播放进度失败: {}", e);
                }
            }
        }
    });
    server.await?;

    println!("应用已退出");
    Ok(())
}
