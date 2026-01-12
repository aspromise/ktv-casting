use crate::dlna_controller::DlnaController;
use crate::bilibili_parser::get_bilibili_direct_link;
use local_ip_address::local_ip;
use std::path::Path;
use tokio::time::{Duration, sleep};
use actix_web::{get, web, App, HttpServer, HttpResponse};
use futures_util::StreamExt;
use reqwest::Client;

mod dlna_controller;
mod media_server;
mod bilibili_parser;
mod playlist_manager;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== KTV投屏DLNA应用启动 ===");

    let server_port = 8080;

    // 1. 创建 Reqwest Client
    // 注：在 reqwest 0.12.x 中，use_rustls_tls() 是标准 API。
    // 如果你使用的是特定分支或旧版，请改回你原来的方法名。
    let client = Client::builder()
        .use_rustls_tls() 
        .build()
        .expect("Failed to create client");
    
    let client_data = web::Data::new(client);

    // 2. 配置 HttpServer，但先不运行
    let server = HttpServer::new(move || {
        App::new()
            .app_data(client_data.clone())
            .service(proxy_handler)
    })
    .bind(("0.0.0.0", server_port))?
    .run();

    let server_handle = server.handle();


    tokio::spawn(async move {
        // 等待一下确保服务器 Ready
        sleep(Duration::from_secs(1)).await;

        match run_dlna_logic(server_port).await {
            Ok(_) => println!("DLNA 任务执行完毕"),
            Err(e) => eprintln!("DLNA 任务出错: {}", e),
        }

        println!("正在关闭服务器...");
        // 发送停止信号
        server_handle.stop(true).await;
    });

    // 5. 【关键修改】在主线程运行服务器
    // run() 返回的 Server 是 !Send 的，但在 tokio::main 的根任务中可以直接 await
    server.await?;

    println!("应用已退出");
    Ok(())
}

// 将具体的业务逻辑提取出来，保持 main 清晰
async fn run_dlna_logic(server_port: u16) -> Result<(), Box<dyn std::error::Error>> {
    // 获取本地IP地址
    let local_ip = local_ip()?;
    println!("本地IP地址: {}", local_ip);

    // 创建DLNA控制器
    let controller = DlnaController::new();

    // 发现DLNA设备
    println!("开始搜索DLNA设备...");
    let devices = controller.discover_devices().await?;

    if devices.is_empty() {
        println!("未发现任何DLNA设备");
        return Ok(());
    }

    let device = &devices[0];
    println!("选择设备: {}", device.friendly_name);

    let test_video = "test_videos/12.mp4"; // 这里的路径仅仅为了生成 URI
    
    // 设置URI (会触发对 proxy_handler 的调用)
    // 你的 proxy_handler 实际上会忽略这个路径，去放 B 站视频，这是预期的
    controller.set_avtransport_uri(
            device,
            &format!("/{}", test_video),
            "", // metadata
            local_ip,
            server_port,
        )
        .await?;

    sleep(Duration::from_secs(2)).await;

    println!("开始播放...");
    controller.play(device).await?;

    println!("播放10秒...");
    sleep(Duration::from_secs(10)).await;

    println!("停止播放...");
    controller.stop(device).await?;
    
    Ok(())
}

#[get("/{url:.*}")]
async fn proxy_handler(
    path: web::Path<(String,)>,
    client: web::Data<reqwest::Client>,
) -> Result<HttpResponse, actix_web::Error> {
    let (_url_path,) = path.into_inner();
    let target_url = get_bilibili_direct_link("BV1LS4MzKE8y", Some(1)).await.unwrap();

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
            .unwrap_or(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR)
    );

    for (name, value) in response.headers().iter() {
        let name_str = name.as_str();
        if name_str != "connection" && name_str != "content-encoding" && name_str != "transfer-encoding" {
            client_resp.insert_header((name_str, value.as_bytes()));
        }
    }

    let body_stream = response.bytes_stream().map(|item| {
        item.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    });

    Ok(client_resp.streaming(body_stream))
}