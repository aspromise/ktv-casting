use crate::dlna_controller::DlnaController;
use actix_web::{App, HttpServer, web};
use anyhow::{Context, Result, anyhow, bail};
use env_logger;
use local_ip_address::local_ip;
use log::{error, info, warn};
use playlist_manager::PlaylistManager;
use reqwest::Client;
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::sleep;
use url::Url;

mod bilibili_parser;
mod dlna_controller;
mod media_server;
mod playlist_manager;

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("RUST_LOG").is_err() {
        unsafe {
            std::env::set_var("RUST_LOG", "INFO");
        }
    }
    env_logger::init();

    println!("=== KTV投屏DLNA应用启动 ===");
    println!("输入房间链接，如https://ktv.example.com/102");
    let mut input = String::new();
    io::stdin().read_line(&mut input).expect("无法读取输入");
    let url_str = input.trim();
    // ② 使用 url crate 解析并提取 base URL 与 room_id
    let parsed_url = Url::parse(&url_str).with_context(|| "无法解析 URL")?;

    let base_url = format!(
        "{}://{}",
        parsed_url.scheme(),
        parsed_url
            .host_str()
            .ok_or_else(|| { anyhow!("URL 没有主机") })?
    );

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

    let server_port = 8080;
    let playlist: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
    let mut playlist_manager = PlaylistManager::new(&base_url, room_id, playlist.clone());

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
            // 重试设置AVTransport URI
            loop {
                match controller
                    .set_next_avtransport_uri(&device, &url, "", local_ip, server_port)
                    .await
                {
                    Ok(_) => break,
                    Err(e) => {
                        let error_msg = format!("{}", e);
                        let error_code: Option<u32> = error_msg
                            .split(|c: char| !c.is_numeric())
                            .find(|s| s.len() == 3)
                            .and_then(|s| s.parse().ok());
                        if let Some(code) = error_code {
                            if code / 100 == 2 {
                                // 2xx错误码视为成功
                                info!("设置AVTransport URI返回错误码{}，视为成功", code);
                                break;
                            }
                        }
                        warn!("设置AVTransport URI失败: {}，500ms后重试", error_msg);
                        sleep(Duration::from_millis(500)).await;
                    }
                }
            }

            // 重试next
            loop {
                match controller.next(&device).await {
                    Ok(_) => break,
                    Err(e) => {
                        let error_msg = format!("{}", e);
                        let error_code: Option<u32> = error_msg
                            .split(|c: char| !c.is_numeric())
                            .find(|s| s.len() == 3)
                            .and_then(|s| s.parse().ok());
                        if let Some(code) = error_code {
                            if code / 100 == 2 {
                                // 2xx错误码视为成功
                                info!("设置AVTransport URI返回错误码{}，视为成功", code);
                                break;
                            }
                        }
                        warn!("next失败: {}，500ms后重试", error_msg);
                        sleep(Duration::from_millis(500)).await;
                    }
                }
            }

            // 重试play
            loop {
                match controller.play(&device).await {
                    Ok(_) => break,
                    Err(e) => {
                        let error_msg = format!("{}", e);
                        let error_code: Option<u32> = error_msg
                            .split(|c: char| !c.is_numeric())
                            .find(|s| s.len() == 3)
                            .and_then(|s| s.parse().ok());
                        if let Some(code) = error_code {
                            if code / 100 == 2 {
                                // 2xx错误码视为成功
                                info!("设置AVTransport URI返回错误码{}，视为成功", code);
                                break;
                            }
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
        let mut remaining_secs: u32 = 0;
        let mut total_secs: u32 = 0;
        loop {
            interval.tick().await;
            // 重试get_secs
            loop {

                match controller.get_secs(&device_cloned).await {
                    Ok(result) => {
                        (remaining_secs, total_secs) = result;
                        break;
                    }
                    Err(e) => {
                        let error_msg = format!("{}", e);
                        let error_code: Option<u32> = error_msg
                            .split(|c: char| !c.is_numeric())
                            .find(|s| s.len() == 3)
                            .and_then(|s| s.parse().ok());
                        if let Some(code) = error_code {
                            if code / 100 == 2 {
                                // 2xx错误码视为成功
                                info!("设置AVTransport URI返回错误码{}，视为成功", code);
                                break;
                            }
                        }
                        warn!("get_secs失败: {}，500ms后重试", error_msg);
                        sleep(Duration::from_millis(500)).await;
                    }
                }
            }
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
                            let error_msg = format!("{}", e);
                            error!("next_song失败: {}，500ms后重试", error_msg);
                            sleep(Duration::from_millis(500)).await;
                        }
                    }
                }
                sleep(Duration::from_secs(5)).await;
            }
        }
    });
    server.await?;

    println!("应用已退出");
    Ok(())
}
