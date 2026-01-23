use futures::StreamExt;
use rupnp::ssdp::SearchTarget;
use std::time::Duration;

#[tokio::main]
async fn main() {
    println!("正在发现DLNA设备...");
    
    let search_target = SearchTarget::All;
    match rupnp::discover(&search_target, Duration::from_secs(3), None).await {
        Ok(devices_stream) => {
            println!("发现设备流");
            
            let devices: Vec<_> = devices_stream.collect().await;
            
            for device_result in devices {
                match device_result {
                    Ok(device) => {
                        println!("\n设备: {}", device.friendly_name());
                        println!("位置: {}", device.url());
                        
                        // 打印所有服务信息
                        println!("\n支持的服务:");
                        for service in device.services() {
                            println!("\n服务Debug信息: {:#?}", service);
                        }
                    }
                    Err(e) => {
                        println!("设备错误: {}", e);
                    }
                }
            }
        }
        Err(e) => {
            println!("发现错误: {}", e);
        }
    }
}
