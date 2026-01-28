use crate::dlna_controller::DlnaController;
use actix_web::{App, HttpServer, web};
use anyhow::{Context, Result, bail};
use local_ip_address::local_ip;
use log::{error, info, warn};
use playlist_manager::PlaylistManager;
use reqwest::Client;
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::sleep;
use url::{Position, Url};

mod bilibili_parser;
mod dlna_controller;
mod media_server;
mod mp4_util;
mod playlist_manager;

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
    let segments: Vec<&str> = parsed_url
        .path_segments()
        .map(|s| s.filter(|seg| !seg.is_empty()).collect())
        .unwrap_or_default();

    if segments.is_empty() {
        error!("错误：没有找到房间号");
        bail!("No room id")
    }

    let room_str = segments.last().unwrap();
    let room_id: u64 = room_str
        .parse::<u64>()
        .with_context(|| format!("Error parsing room_str {}", room_str))?;
    info!("Parsed room_id: {}", room_id);

    let server_port = 8080;
    let playlist: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
    let mut playlist_manager = PlaylistManager::new(&base_url, room_id, playlist.clone());

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
    playlist_manager.start_periodic_update(move |url| {
        let controller = controller.clone();
        let device = device.clone();
        Box::pin(async move {
            loop {
                match controller.stop(&device).await {
                    Ok(_) => {
                        info!("成功停止播放");
                        break;
                    }
                    Err(e) => {
                        let error_msg = format!("{}", e);
                        let error_code: Option<u32> = error_msg
                            .split(|c: char| !c.is_numeric())
                            .find(|s| s.len() == 3)
                            .and_then(|s| s.parse().ok());
                        if let Some(code) = error_code
                            && code / 100 == 2
                        {
                            // 2xx错误码视为成功
                            info!("停止播放返回错误码{}，视为成功", code);
                            break;
                        }

                        warn!("停止播放失败: {}，500ms后重试", error_msg);
                        sleep(Duration::from_millis(500)).await;
                    }
                }
            }

            loop {
                match controller
                    .set_avtransport_uri(&device, &url, "", local_ip, server_port)
                    .await
                {
                    Ok(_) => {
                        info!("成功设置AVTransport URI为 {}", url);
                        break;
                    }
                    Err(e) => {
                        let error_msg = format!("{}", e);
                        let error_code: Option<u32> = error_msg
                            .split(|c: char| !c.is_numeric())
                            .find(|s| s.len() == 3)
                            .and_then(|s| s.parse().ok());
                        if let Some(code) = error_code
                            && code / 100 == 2
                        {
                            // 2xx错误码视为成功
                            info!("设置AVTransport URI返回错误码{}，视为成功", code);
                            break;
                        }

                        warn!("设置AVTransport URI失败: {}，500ms后重试", error_msg);
                        sleep(Duration::from_millis(500)).await;
                    }
                }
            }
            // // 重试设置AVTransport URI
            // loop {
            //     match controller
            //         .set_next_avtransport_uri(&device, &url, "", local_ip, server_port)
            //         .await
            //     {
            //         Ok(_) => break,
            //         Err(e) => {
            //             let error_msg = format!("{}", e);
            //             let error_code: Option<u32> = error_msg
            //                 .split(|c: char| !c.is_numeric())
            //                 .find(|s| s.len() == 3)
            //                 .and_then(|s| s.parse().ok());
            //             if let Some(code) = error_code {
            //                 if code / 100 == 2 {
            //                     // 2xx错误码视为成功
            //                     info!("设置AVTransport URI返回错误码{}，视为成功", code);
            //                     break
            //                 }
            //             }
            //             warn!("设置AVTransport URI失败: {}，500ms后重试", error_msg);
            //             sleep(Duration::from_millis(500)).await;
            //         }
            //     }
            // }

            // // 重试next
            // loop {
            //     match controller.next(&device).await {
            //         Ok(_) => break,
            //         Err(e) => {
            //             let error_msg = format!("{}", e);
            //             let error_code: Option<u32> = error_msg
            //                 .split(|c: char| !c.is_numeric())
            //                 .find(|s| s.len() == 3)
            //                 .and_then(|s| s.parse().ok());
            //             if let Some(code) = error_code {
            //                 if code / 100 == 2 {
            //                     // 2xx错误码视为成功
            //                     info!("设置AVTransport URI返回错误码{}，视为成功", code);
            //                     break;
            //                 }
            //             }
            //             warn!("next失败: {}，500ms后重试", error_msg);
            //             sleep(Duration::from_millis(500)).await;
            //         }
            //     }
            // }

            // 重试play
            loop {
                match controller.play(&device).await {
                    Ok(_) => {
                        info!("成功开始播放");
                        break;
                    }
                    Err(e) => {
                        let error_msg = format!("{}", e);
                        let error_code: Option<u32> = error_msg
                            .split(|c: char| !c.is_numeric())
                            .find(|s| s.len() == 3)
                            .and_then(|s| s.parse().ok());
                        if let Some(code) = error_code
                            && code / 100 == 2
                        {
                            // 2xx错误码视为成功
                            info!("播放返回错误码{}，视为成功", code);
                            break;
                        }

                        warn!("play失败: {}，500ms后重试", error_msg);
                        sleep(Duration::from_millis(500)).await;
                    }
                }
            }
        })
    });

    tokio::spawn(async move {
        let controller = DlnaController::new();
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        let mut current_secs: u32 = 0;
        let mut total_secs: u32 = 0;
        loop {
            interval.tick().await;

            // 首先尝试从缓存中获取总长度
            let mut cached_total = 0;
            if let Some(playing) = playlist_manager.get_song_playing().await {
                let cache = duration_cache.lock().await;
                if let Some(&d) = cache.get(&playing) {
                    cached_total = d;
                }
            }

            // 重试get_secs
            loop {
                match controller.get_secs(&device_cloned).await {
                    Ok(result) => {
                        (current_secs, _) = result;

                        // 如果从缓存拿到了长度，
                        if cached_total > 0 {
                            total_secs = cached_total;
                            info!("使用缓存的视频时长: {}s", total_secs);
                        }

                        let remaining_secs = if total_secs > current_secs {
                            total_secs - current_secs
                        } else {
                            0
                        };

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
                            loop {
                                match playlist_manager.next_song().await {
                                    Ok(_) => break,
                                    Err(e) => {
                                        let error_msg = e.to_string();
                                        error!("next_song失败: {}，500ms后重试", error_msg);
                                        sleep(Duration::from_millis(500)).await;
                                    }
                                }
                            }
                            sleep(Duration::from_secs(5)).await;
                        }
                        break;
                    }
                    Err(e) => {
                        let error_msg = format!("{}", e);
                        let error_code: Option<u32> = error_msg
                            .split(|c: char| !c.is_numeric())
                            .find(|s| s.len() == 3)
                            .and_then(|s| s.parse().ok());
                        if let Some(code) = error_code
                            && code / 100 == 2
                        {
                            // 2xx错误码视为成功
                            info!("获取进度返回错误码{}，视为成功", code);
                            break;
                        }

                        warn!("get_secs失败: {}，500ms后重试", error_msg);
                        sleep(Duration::from_millis(500)).await;
                    }
                }
            }
        }
    });
    server.await?;

    println!("应用已退出");
    Ok(())
}
