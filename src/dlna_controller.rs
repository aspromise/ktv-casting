use futures::stream::StreamExt;
use reqwest::Client;
use rupnp::Device;
use rupnp::ssdp::{SearchTarget, URN};
use std::net::IpAddr;
use std::time::Duration;
use std::collections::HashMap;
use chrono::NaiveTime;

// AVTransport服务URN
const AV_TRANSPORT: URN = URN::service("schemas-upnp-org", "AVTransport", 1);
const RENDERING_CONTROL: URN = URN::service("schemas-upnp-org", "RenderingControl", 1);

// DLNA设备信息
#[derive(Debug, Clone)]
pub struct DlnaDevice {
    pub device: Device,
    pub friendly_name: String,
    pub location: String,
    pub services: Vec<URN>,
}

// DLNA控制器
pub struct DlnaController {
    client: Client,
}

impl DlnaController {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
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
    ) -> Result<(), Box<dyn std::error::Error>> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or("设备不支持AVTransport服务")?;

        // 构建完整的媒体URL
        let media_url = format!("http://{}:{}{}", server_ip, server_port, current_uri);
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

    // 设置下一首媒体URI
    pub async fn set_next_avtransport_uri(
        &self,
        device: &DlnaDevice,
        next_uri: &str,
        next_uri_metadata: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or("设备不支持AVTransport服务")?;

        let action = "SetNextAVTransportURI";
        let args_str = format!(
            r#"
            <InstanceID>0</InstanceID>
            <NextURI>{}</NextURI>
            <NextURIMetaData>{}</NextURIMetaData>
            "#,
            next_uri, next_uri_metadata
        );

        let device_url = device.device.url();
        let response = avtransport.action(device_url, action, &args_str).await?;

        println!("SetNextAVTransportURI响应: {:?}", response);

        Ok(())
    }

    // 播放媒体
    pub async fn play(&self, device: &DlnaDevice) -> Result<(), Box<dyn std::error::Error>> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or("设备不支持AVTransport服务")?;

        let action = "Play";
        let args_str = r#"
            <InstanceID>0</InstanceID>
            <Speed>1</Speed>
            "#;

        let device_url = device.device.url();
        let response = avtransport
            .action(device_url, action, &args_str)
            .await?;
        println!("Play响应: {:?}", response);

        Ok(())
    }

    // 暂停播放
    pub async fn pause(&self, device: &DlnaDevice) -> Result<(), Box<dyn std::error::Error>> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or("设备不支持AVTransport服务")?;

        let action = "Pause";
        let args_str = "<InstanceID>0</InstanceID>";

        let device_url = device.device.url();
        let response = avtransport.action(device_url, action, &args_str).await?;
        println!("Pause响应: {:?}", response);

        Ok(())
    }

    // 停止播放
    pub async fn stop(&self, device: &DlnaDevice) -> Result<(), Box<dyn std::error::Error>> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or("设备不支持AVTransport服务")?;

        let action = "Stop";
        let args_str = "<InstanceID>0</InstanceID>";

        let device_url = device.device.url();
        let response = avtransport.action(device_url, action, &args_str).await?;
        println!("Stop响应: {:?}", response);

        Ok(())
    }

    // 切换到下一首媒体
    pub async fn next(&self, device: &DlnaDevice) -> Result<(), Box<dyn std::error::Error>> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or("设备不支持AVTransport服务")?;

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
    ) -> Result<(), Box<dyn std::error::Error>> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or("设备不支持AVTransport服务")?;

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
    ) -> Result<HashMap<String, String>, Box<dyn std::error::Error>> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or("设备不支持AVTransport服务")?;

        let action = "GetPositionInfo";
        let args_str = "<InstanceID>0</InstanceID>";

        let device_url = device.device.url();
        let response: HashMap<String, String> = avtransport.action(device_url, action, &args_str).await?;
        println!("位置信息: {:?}", response);

        Ok(response)
    }

    pub async fn get_remaining_time(
        &self,
        device: &DlnaDevice,
    ) -> Result<u32, Box<dyn std::error::Error>> {
        let position_info = self.get_position_info(device).await?;
        let track_duration = position_info
            .get("TrackDuration")
            .ok_or("无法获取TrackDuration")?;
        let current_time = position_info
            .get("RelTime")
            .ok_or("无法获取RelTime")?;

        let track_duration = NaiveTime::parse_from_str(track_duration, "%H:%M:%S")?;
        let current_time = NaiveTime::parse_from_str(current_time, "%H:%M:%S")?;

        let remaining_time = track_duration - current_time;
        Ok(remaining_time.num_seconds() as u32)
    }
}

