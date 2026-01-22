use crate::bilibili_parser::get_bilibili_direct_link;
use crate::dlna_controller::DlnaController;
use actix_web::{App, HttpResponse, HttpServer, get, web};
use anyhow::{Context, Result, anyhow, bail};
use futures_util::StreamExt;
use local_ip_address::local_ip;
use playlist_manager::PlaylistManager;
use reqwest::Client;
use tokio::time::sleep;
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use url::Url;

mod bilibili_parser;
mod dlna_controller;
mod media_server;
mod playlist_manager;

#[tokio::main]
async fn main() -> Result<()> {
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
        eprintln!("错误：没有找到房间号");
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
        .tls_backend_rustls()
        .build()
        .expect("Failed to create client");

    let client_data = web::Data::new(client);

    // 2. 配置 HttpServer，运行
    let server = HttpServer::new(move || {
        App::new()
            .app_data(client_data.clone())
            .service(proxy_handler)
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
                        eprintln!("设置AVTransport URI失败: {}，500ms后重试", error_msg);
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
                        eprintln!("next失败: {}，500ms后重试", error_msg);
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
                        eprintln!("play失败: {}，500ms后重试", error_msg);
                        sleep(Duration::from_millis(500)).await;
                    }
                }
            }
        })
    });

    tokio::spawn(async move {
        let controller = DlnaController::new();
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        let mut remaining_secs: u32;
        let mut total_secs: u32;
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
                        eprintln!("get_secs失败: {}，500ms后重试", error_msg);
                        sleep(Duration::from_millis(500)).await;
                    }
                }
            }
            if remaining_secs <= 2 && total_secs > 0 {
                eprintln!("剩余时间{}秒，总时间{}秒，准备切歌", remaining_secs, total_secs);
                // 重试next_song
                loop {
                    match playlist_manager.next_song().await {
                        Ok(_) => break,
                        Err(e) => {
                            let error_msg = format!("{}", e);
                            eprintln!("next_song失败: {}，500ms后重试", error_msg);
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

#[get("/{url:.*}")]
async fn proxy_handler(
    path: web::Path<(String,)>,
    client: web::Data<reqwest::Client>,
) -> Result<HttpResponse, actix_web::Error> {
    let (origin_url,) = path.into_inner();
    let bv_id = &origin_url[..origin_url.find('?').unwrap_or(origin_url.len())];
    let page: Option<u32> = if let Some(pos) = origin_url.find("?page=") {
        origin_url[pos + 6..].parse().ok()
    } else {
        None
    };
    let target_url = get_bilibili_direct_link(&bv_id, page)
        .await
        .map_err(actix_web::error::ErrorInternalServerError)?;

    let response = client
        .get(&target_url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/118.0.0.0 Safari/537.36")
        .header("Referer", "https://www.bilibili.com/") // 加上这个通常更稳
        .send()
        .await
        .map_err(actix_web::error::ErrorInternalServerError)?;

    let status_u16 = response.status().as_u16();
    let mut client_resp = HttpResponse::build(
        actix_web::http::StatusCode::from_u16(status_u16)
            .unwrap_or(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR),
    );

    for (name, value) in response.headers().iter() {
        let name_str = name.as_str();
        if name_str != "connection"
            && name_str != "content-encoding"
            && name_str != "transfer-encoding"
        {
            client_resp.insert_header((name_str, value.as_bytes()));
        }
    }

    let body_stream = response
        .bytes_stream()
        .map(|item| item.map_err(|e| std::io::Error::other(e)));

    Ok(client_resp.streaming(body_stream))
}
