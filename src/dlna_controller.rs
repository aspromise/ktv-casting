use futures::stream::StreamExt;
use reqwest::Client;
use rupnp::Device;
use rupnp::ssdp::{SearchTarget, URN};
use std::net::IpAddr;
use std::time::Duration;

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

    // 播放媒体
    pub async fn play(&self, device: &DlnaDevice) -> Result<(), Box<dyn std::error::Error>> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or("设备不支持AVTransport服务")?;

        let action = "Play";
        let args_str = "InstanceID=0&Speed=1";

        let device_url = device.device.url();
        let response = avtransport.action(device_url, action, &args_str).await?;
        println!("Play响应: {:?}", response);

        Ok(())
    }

    // 暂停播放
    pub async fn pause(&self, device: &DlnaDevice) -> Result<(), Box<dyn std::error::Error>> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or("设备不支持AVTransport服务")?;

        let action = "Pause";
        let args_str = "InstanceID=0";

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
        let args_str = "InstanceID=0";

        let device_url = device.device.url();
        let response = avtransport.action(device_url, action, &args_str).await?;
        println!("Stop响应: {:?}", response);

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
        let args_str = "InstanceID=0";

        let device_url = device.device.url();
        let response = avtransport.action(device_url, action, &args_str).await?;
        println!("传输信息: {:?}", response);

        Ok(())
    }

    // 获取位置信息
    pub async fn get_position_info(
        &self,
        device: &DlnaDevice,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or("设备不支持AVTransport服务")?;

        let action = "GetPositionInfo";
        let args_str = "InstanceID=0";

        let device_url = device.device.url();
        let response = avtransport.action(device_url, action, &args_str).await?;
        println!("位置信息: {:?}", response);

        Ok(())
    }
}

// 生成DIDL-Lite元数据
pub fn generate_didl_metadata(title: &str, mime_type: &str, duration: Option<&str>) -> String {
    let duration_str = duration.unwrap_or("0:00:00");

    format!(
        r#"&lt;DIDL-Lite xmlns:dc=&quot;http://purl.org/dc/elements/1.1/&quot; xmlns:upnp=&quot;urn:schemas-upnp-org:metadata-1-0/upnp/&quot; xmlns=&quot;urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/&quot;&gt;
    &lt;item id=&quot;1&quot; parentID=&quot;-1&quot; restricted=&quot;0&quot;&gt;
        &lt;dc:title&gt;{}&lt;/dc:title&gt;
        &lt;dc:creator&gt;Unknown&lt;/dc:creator&gt;
        &lt;upnp:class&gt;object.item.videoItem&lt;/upnp:class&gt;
        &lt;res protocolInfo=&quot;http-get:*:{}:DLNA.ORG_OP=01;DLNA.ORG_CI=0;DLNA.ORG_FLAGS=01700000000000000000000000000000&quot; duration=&quot;{}&quot;&gt;http://placeholder/url&lt;/res&gt;
    &lt;/item&gt;
&lt;/DIDL-Lite&gt;"#,
        title, mime_type, duration_str
    )
}
