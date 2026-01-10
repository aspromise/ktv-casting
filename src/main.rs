use crate::dlna_controller::{DlnaController, generate_didl_metadata};
use crate::media_server::start_media_server;
use local_ip_address::local_ip;
use std::path::Path;
use tokio::time::{Duration, sleep};

mod dlna_controller;
mod media_server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== KTV投屏DLNA应用启动 ===");

    // 启动媒体服务器 - 使用spawn_blocking避免Send trait问题
    let server_port = 8080;
    let server_handle = tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            if let Err(e) = start_media_server("0.0.0.0", server_port, "./test_videos").await {
                eprintln!("媒体服务器启动失败: {}", e);
            }
        });
    });

    // 等待服务器启动
    sleep(Duration::from_secs(2)).await;

    // 获取本地IP地址
    let local_ip = local_ip()?;
    println!("本地IP地址: {}", local_ip);

    // 创建DLNA控制器
    let controller = DlnaController::new();

    // 发现DLNA设备
    println!("开始搜索DLNA设备...");
    let devices = controller.discover_devices().await?;

    if devices.is_empty() {
        println!("未发现任何DLNA设备，请确保设备在同一网络中并已开启DLNA功能");
        return Ok(());
    }

    println!("发现 {} 个DLNA设备", devices.len());

    // 选择第一个设备进行测试
    let device = &devices[0];
    println!("选择设备: {}", device.friendly_name);

    // 测试媒体文件
    let test_video = "test_videos/12.mp4";
    if !Path::new(test_video).exists() {
        println!("测试视频文件不存在: {}", test_video);
        return Ok(());
    }

    // 生成元数据
    // let metadata = generate_didl_metadata("测试视频", "video/mp4", Some("0:03:30"));
    let metadata = "".to_string();
    // 设置AVTransport URI
    println!("设置媒体URI...");
    if let Err(e) = controller
        .set_avtransport_uri(
            device,
            &format!("/{}", test_video),
            &metadata,
            local_ip,
            server_port,
        )
        .await
    {
        eprintln!("设置媒体URI失败: {}", e);
    }

    // 等待设备准备
    sleep(Duration::from_secs(2)).await;

    // 开始播放
    println!("开始播放...");
    if let Err(e) = controller.play(device).await {
        eprintln!("播放失败: {}", e);
    }

    // 播放10秒
    println!("播放10秒...");
    sleep(Duration::from_secs(10)).await;

    // 暂停
    println!("暂停播放...");
    if let Err(e) = controller.pause(device).await {
        eprintln!("暂停失败: {}", e);
    }

    // 等待2秒
    sleep(Duration::from_secs(2)).await;

    // 恢复播放
    println!("恢复播放...");
    if let Err(e) = controller.play(device).await {
        eprintln!("恢复播放失败: {}", e);
    }

    // 播放5秒
    println!("播放5秒...");
    sleep(Duration::from_secs(5)).await;

    // 停止
    println!("停止播放...");
    if let Err(e) = controller.stop(device).await {
        eprintln!("停止失败: {}", e);
    }

    // 获取设备状态
    println!("获取设备状态...");
    if let Err(e) = controller.get_transport_info(device).await {
        eprintln!("获取传输信息失败: {}", e);
    }

    if let Err(e) = controller.get_position_info(device).await {
        eprintln!("获取位置信息失败: {}", e);
    }

    println!("=== 测试完成 ===");
    println!("按Ctrl+C退出...");

    // 等待用户中断
    tokio::signal::ctrl_c().await?;

    // 关闭服务器
    server_handle.abort();

    println!("应用已退出");
    Ok(())
}
