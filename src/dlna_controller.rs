use chrono::{NaiveTime, Timelike};
use futures::future::try_join_all;
use futures::stream::StreamExt;
use rupnp::Device;
use rupnp::http::Uri;
use rupnp::ssdp::{SearchTarget, URN};
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

// AVTransport服务URN
const AV_TRANSPORT: URN = URN::service("schemas-upnp-org", "AVTransport", 1);

// DLNA设备信息
#[derive(Debug, Clone)]
pub struct DlnaDevice {
    pub device: Device,
    pub friendly_name: String,
    pub location: String,
    pub services: Vec<URN>,
}

#[derive(Clone)]
pub struct DlnaController;

impl DlnaController {
    pub fn new() -> Self {
        Self
    }

    // 发现网络中的DLNA渲染器设备
    pub async fn discover_devices(&self) -> Result<Vec<DlnaDevice>, rupnp::Error> {
        println!("正在搜索DLNA设备...");

        // 使用正确的SearchTarget构造方法 - 搜索AVTransport服务
        let search_target = SearchTarget::URN(AV_TRANSPORT);
        let devices_stream = rupnp::discover(&search_target, Duration::from_secs(5), None).await?;

        // 将Stream转换为Vec
        let devices: Vec<Result<Device, rupnp::Error>> = devices_stream.collect().await;

        let mut dlna_devices = Vec::new();

        for device_result in devices {
            match device_result {
                Ok(device) => {
                    // 检查是否是媒体渲染器设备
                    let device_type_str = device.device_type().to_string();
                    if device_type_str.contains("MediaRenderer") {
                        let friendly_name = device.friendly_name().to_string();
                        let location = device.url().to_string();

                        // 检查设备是否支持AVTransport服务
                        let services: Vec<URN> = device
                            .services()
                            .iter()
                            .map(|s| s.service_type().clone())
                            .collect();

                        println!("发现设备: {} (位置: {})", friendly_name, location);
                        println!("支持的服务: {:?}", services);

                        dlna_devices.push(DlnaDevice {
                            device,
                            friendly_name,
                            location,
                            services,
                        });
                    }
                }
                Err(e) => {
                    println!("设备发现错误: {}", e);
                }
            }
        }

        Ok(dlna_devices)
    }

    pub async fn get_devices_from_urls(
        &self,
        urls: &Vec<&'static str>,
    ) -> Result<Vec<DlnaDevice>, rupnp::Error> {
        let devices = try_join_all(urls.iter().map(|url| {
            let uri = Uri::from_static(url);
            Device::from_url(uri)
        }))
        .await?;

        let dlna_devices: Vec<DlnaDevice> = devices
            .into_iter()
            .map(|device| DlnaDevice {
                device: device.clone(),
                friendly_name: device.friendly_name().to_string(),
                location: device.url().to_string(),
                services: device
                    .services()
                    .iter()
                    .map(|s| s.service_type().clone())
                    .collect(),
            })
            .collect();
        Ok(dlna_devices)
    }

    // 获取设备的AVTransport服务
    fn get_avtransport_service<'a>(&'a self, device: &'a DlnaDevice) -> Option<&'a rupnp::Service> {
        device
            .device
            .services()
            .iter()
            .find(|s| *s.service_type() == AV_TRANSPORT)
    }

    // 设置AVTransport URI（发送媒体URL给设备）
    pub async fn set_avtransport_uri(
        &self,
        device: &DlnaDevice,
        current_uri: &str,
        current_uri_metadata: &str,
        server_ip: IpAddr,
        server_port: u16,
    ) -> Result<(), rupnp::Error> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or(rupnp::Error::ParseError("设备不支持AVTransport服务"))?;

        // 构建完整的媒体URL
        let media_url = format!("http://{}:{}/{}", server_ip, server_port, current_uri);
        // let media_url = "https://cn-jsnt-ct-01-06.bilivideo.com/upgcxcode/95/66/65166695/65166695-1-208.mp4?e=ig8euxZM2rNcNbN3hwdVhwdlhb4VhwdVhoNvNC8BqJIzNbfq9rVEuxTEnE8L5F6VnEsSTx0vkX8fqJeYTj_lta53NCM=&platform=html5&oi=1696788563&trid=0000552b5f27ec06482cbd0f902c89beadeT&mid=483794508&nbs=1&os=bcache&uipk=5&deadline=1768072065&gen=playurlv3&og=hw&upsig=40d24fb953240187eb8a621ba81a3085&uparams=e,platform,oi,trid,mid,nbs,os,uipk,deadline,gen,og&cdnid=4284&bvc=vod&nettype=0&bw=2247418&agrr=0&buvid=&build=0&dl=0&f=T_0_0&mobi_app=&orderid=0,1".to_string();

        println!("设置媒体URI: {}", media_url);
        println!("元数据: {}", current_uri_metadata);

        // 准备SOAP请求参数
        let action = "SetAVTransportURI";
        let args_str = format!(
            r#"
            <InstanceID>0</InstanceID>
            <CurrentURI>{}</CurrentURI>
            <CurrentURIMetaData>{}</CurrentURIMetaData>
            "#,
            media_url, current_uri_metadata
        );

        // 发送SOAP请求 - 使用device.url()而不是service.url()
        let device_url = device.device.url();
        let response = avtransport.action(device_url, action, &args_str).await?;

        println!("SetAVTransportURI响应: {:?}", response);

        Ok(())
    }

    // 设置下一个AVTransport URI（用于播放列表）
    pub async fn set_next_avtransport_uri(
        &self,
        device: &DlnaDevice,
        next_uri: &str,
        next_uri_metadata: &str,
        server_ip: IpAddr,
        server_port: u16,
    ) -> Result<(), rupnp::Error> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or(rupnp::Error::ParseError("设备不支持AVTransport服务"))?;

        let action = "SetNextAVTransportURI";
        let media_url = format!("http://{}:{}/{}", server_ip, server_port, next_uri);
        let args_str = format!(
            r#"
            <InstanceID>0</InstanceID>
            <NextURI>{}</NextURI>
            <NextURIMetaData>{}</NextURIMetaData>
            "#,
            media_url, next_uri_metadata
        );

        let device_url = device.device.url();
        let response = avtransport.action(device_url, action, &args_str).await?;

        println!("SetNextAVTransportURI响应: {:?}", response);

        Ok(())
    }

    // 播放媒体
    pub async fn play(&self, device: &DlnaDevice) -> Result<(), rupnp::Error> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or(rupnp::Error::ParseError("设备不支持AVTransport服务"))?;

        let action = "Play";
        let args_str = r#"
            <InstanceID>0</InstanceID>
            <Speed>1</Speed>
            "#;

        let device_url = device.device.url();
        let response = avtransport.action(device_url, action, &args_str).await?;
        println!("Play响应: {:?}", response);

        Ok(())
    }

    // 暂停播放
    pub async fn pause(&self, device: &DlnaDevice) -> Result<(), rupnp::Error> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or(rupnp::Error::ParseError("设备不支持AVTransport服务"))?;

        let action = "Pause";
        let args_str = "<InstanceID>0</InstanceID>";

        let device_url = device.device.url();
        let response = avtransport.action(device_url, action, &args_str).await?;
        println!("Pause响应: {:?}", response);

        Ok(())
    }

    // 停止播放
    pub async fn stop(&self, device: &DlnaDevice) -> Result<(), rupnp::Error> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or(rupnp::Error::ParseError("设备不支持AVTransport服务"))?;

        let action = "Stop";
        let args_str = "<InstanceID>0</InstanceID>";

        let device_url = device.device.url();
        let response = avtransport.action(device_url, action, &args_str).await?;
        println!("Stop响应: {:?}", response);

        Ok(())
    }

    // 下一首
    pub async fn next(&self, device: &DlnaDevice) -> Result<(), rupnp::Error> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or(rupnp::Error::ParseError("设备不支持AVTransport服务"))?;

        let action = "Next";
        let args_str = "<InstanceID>0</InstanceID>";

        let device_url = device.device.url();
        let response = avtransport.action(device_url, action, &args_str).await?;
        println!("Next响应: {:?}", response);

        Ok(())
    }

    // 获取传输信息
    pub async fn get_transport_info(
        &self,
        device: &DlnaDevice,
    ) -> Result<(), rupnp::Error> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or(rupnp::Error::ParseError("设备不支持AVTransport服务"))?;

        let action = "GetTransportInfo";
        let args_str = "<InstanceID>0</InstanceID>";

        let device_url = device.device.url();
        let response = avtransport.action(device_url, action, &args_str).await?;
        println!("传输信息: {:?}", response);

        Ok(())
    }

    // 获取位置信息
    pub async fn get_position_info(
        &self,
        device: &DlnaDevice,
    ) -> Result<HashMap<String, String>, rupnp::Error> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or(rupnp::Error::ParseError("设备不支持AVTransport服务"))?;

        let action = "GetPositionInfo";
        let args_str = "<InstanceID>0</InstanceID>";

        let device_url = device.device.url();
        let response = avtransport.action(device_url, action, &args_str).await?;

        // 解析响应
        let mut result = HashMap::new();
        for (key, value) in response {
            result.insert(key, value);
        }

        Ok(result)
    }

    // 获取当前播放位置（秒）
    pub async fn get_secs(&self, device: &DlnaDevice) -> Result<(u32, u32), rupnp::Error> {
        let position_info = self.get_position_info(device).await?;
        
        // 获取相对时间
        let default_time = "00:00:00".to_string();
        let rel_time = position_info.get("RelTime").unwrap_or(&default_time);
        let duration = position_info.get("TrackDuration").unwrap_or(&default_time);
        eprintln!("get_secs() : RelTime: {}, TrackDuration: {}", rel_time, duration);
        
        let track_duration = NaiveTime::parse_from_str(duration, "%H:%M:%S").map_err(|e| {
            rupnp::Error::ParseError("无法解析TrackDuration")
        })?;
        let current_time = NaiveTime::parse_from_str(rel_time, "%H:%M:%S").map_err(|e| {
            rupnp::Error::ParseError("无法解析RelTime")
        })?;

        let remaining_time = track_duration - current_time;

        let remaining_secs = remaining_time.num_seconds() as u32;
        let total_secs = track_duration.second();
        
        Ok((remaining_secs, total_secs))
    }

    // 设置渲染器音量
    pub async fn set_volume(
        &self,
        device: &DlnaDevice,
        volume: u32,
    ) -> Result<(), rupnp::Error> {
        let rendering_control = device
            .device
            .services()
            .iter()
            .find(|s| *s.service_type() == URN::service("schemas-upnp-org", "RenderingControl", 1))
            .ok_or(rupnp::Error::ParseError("设备不支持RenderingControl服务"))?;

        let action = "SetVolume";
        let args_str = format!(
            r#"
            <InstanceID>0</InstanceID>
            <Channel>Master</Channel>
            <DesiredVolume>{}</DesiredVolume>
            "#,
            volume
        );

        let device_url = device.device.url();
        let response = rendering_control.action(device_url, action, &args_str).await?;
        println!("SetVolume响应: {:?}", response);

        Ok(())
    }

    // 获取渲染器音量
    pub async fn get_volume(&self, device: &DlnaDevice) -> Result<u32, rupnp::Error> {
        let rendering_control = device
            .device
            .services()
            .iter()
            .find(|s| *s.service_type() == URN::service("schemas-upnp-org", "RenderingControl", 1))
            .ok_or(rupnp::Error::ParseError("设备不支持RenderingControl服务"))?;

        let action = "GetVolume";
        let args_str = r#"
            <InstanceID>0</InstanceID>
            <Channel>Master</Channel>
            "#;

        let device_url = device.device.url();
        let response = rendering_control.action(device_url, action, &args_str).await?;
        
        // 解析音量值
        let default_volume = "0".to_string();
        let volume_str = response.get("CurrentVolume").unwrap_or(&default_volume);
        let volume: u32 = volume_str.parse().unwrap_or(0);
        
        Ok(volume)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_set_next_avtransport_uri() {
        let controller = DlnaController::new();
        
        // 发现DLNA设备
        let devices = controller.discover_devices().await;
        match devices {
            Ok(devices) => {
                if devices.is_empty() {
                    println!("未发现DLNA设备，跳过测试");
                    return;
                }
                
                // 使用第一个设备
                let device = &devices[0];
                println!("使用设备: {}", device.friendly_name);
                
                // 测试设置下一首媒体URI
                let result = controller.set_next_avtransport_uri(
                    device,
                    "/media/test_next.mp4",
                    "",
                    "127.0.0.1".parse().unwrap(),
                    8080,
                ).await;
                
                match result {
                    Ok(_) => println!("设置下一首媒体URI成功"),
                    Err(e) => println!("设置下一首媒体URI失败: {}", e),
                }
            }
            Err(e) => {
                println!("设备发现失败: {}", e);
            }
        }
    }
}
