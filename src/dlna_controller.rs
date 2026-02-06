use chrono::{NaiveTime, Timelike};
use futures::future::try_join_all;
use futures::stream::StreamExt;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use rupnp::Device;
use rupnp::http::Uri;
use rupnp::ssdp::{SearchTarget, URN};
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

fn extract_xml_tag_value(xml: &str, tag: &str) -> Option<String> {
    // 解析XML标签值，支持带命名空间属性的标签
    let start_pattern = format!("<{}", tag);
    let end_pattern = format!("</{}>", tag);

    // 找到开始标签
    if let Some(start_idx) = xml.find(&start_pattern) {
        // 找到开始标签的结束位置 >
        if let Some(tag_end_idx) = xml[start_idx..].find('>') {
            let value_start = start_idx + tag_end_idx + 1;

            // 找到对应的结束标签
            if let Some(value_end) = xml[value_start..].find(&end_pattern) {
                let value = &xml[value_start..value_start + value_end];
                return Some(value.trim().to_string());
            }
        }
    }

    None
}

fn xml_escape(s: &str) -> String {
    // Minimal XML escaping for element text nodes.
    // (Enough to keep SOAP XML well-formed when URLs contain & and friends.)
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn build_didl_lite_metadata(title: &str, media_url: &str, protocol_info: Option<&str>) -> String {
    // Build a minimal DIDL-Lite and then XML-escape it for embedding into <CurrentURIMetaData>.
    // Many renderers require at least: upnp:class + res@protocolInfo.
    // NOTE: avoid strict DLNA.ORG_PN profile binding; some renderers reject when profile ≠ actual.
    // Start permissive, then tighten if needed.
    let protocol = protocol_info.unwrap_or("http-get:*:video/mp4:*");

    // Important: the <res> inner URL should be XML-escaped *once* (so & -> &amp;).
    let res_url = xml_escape(media_url);

    let didl = format!(
        r#"<DIDL-Lite xmlns=\"urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:upnp=\"urn:schemas-upnp-org:metadata-1-0/upnp/\">
        <item id=\"0\" parentID=\"-1\" restricted=\"1\">
        <dc:title>{}</dc:title>
        <upnp:storageMedium>UNKNOWN</upnp:storageMedium>
        <upnp:writeStatus>UNKNOWN</upnp:writeStatus>
        <res protocolInfo=\"{}\">{}</res>
        <upnp:class>object.item.videoItem</upnp:class>
        </item>
        </DIDL-Lite>"#,
        xml_escape(title),
        protocol,
        res_url
    );

    // Embed metadata as escaped XML text nodes: <CurrentURIMetaData>&lt;DIDL-Lite ...&gt;...
    xml_escape(&didl)
}

fn build_soap_envelope(action: &str, args_xml: &str) -> String {
    // Keep the shape consistent with what most renderers accept (and close to your B站抓包).
    // Note: `rupnp` will build its own envelope too, but we log a best-effort equivalent
    // so you can diff with a packet capture.
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<s:Envelope s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/" xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
        <u:{action} xmlns:u="urn:schemas-upnp-org:service:AVTransport:1">{args}</u:{action}>
  </s:Body>
</s:Envelope>"#,
        action = action,
        args = args_xml
    )
}

fn device_location_uri(device: &DlnaDevice) -> Result<Uri, rupnp::Error> {
    device
        .location
        .parse::<Uri>()
        .map_err(|_| rupnp::Error::ParseError("无法解析设备location为Uri"))
}

fn log_upnp_action(service: &rupnp::Service, base_url: &Uri, action: &str, args_xml: &str) {
    // `service.action()` internally ends up using control_url(base_url).
    // We can't call the private control_url() here, so we log the base_url and also log
    // the serviceId/type to help confirm we matched the expected service.
    //
    // If you still see 204, compare this log with your抓包，重点确认：Host/port/path。
    let soap_action_header = format!("\"urn:schemas-upnp-org:service:AVTransport:1#{}\"", action);

    // Logged body is a best-effort “wire-like” payload for diffing.
    let envelope = build_soap_envelope(action, args_xml);

    log::debug!(
        "UPnP Action -> base_url={} service_id={} service_type={} SOAPAction={}",
        base_url,
        service.service_id(),
        service.service_type(),
        soap_action_header
    );
    log::debug!("UPnP Action body (approx) => {}", envelope);
}

/// Some renderers publish a `controlURL` like `_urn:schemas-upnp-org:service:AVTransport_control`
/// (missing the leading `/`). In practice the working endpoint is often `/_urn:...`.
///
/// `rupnp`'s internal URL replacement may produce the wrong path for such devices.
/// To make behavior explicit (and loggable), we send the SOAP request ourselves to:
/// `{scheme}://{host}:{port}/{control_path}`.
async fn avtransport_action_compat(
    service: &rupnp::Service,
    base_url: &Uri,
    action: &str,
    args_xml: &str,
) -> Result<HashMap<String, String>, rupnp::Error> {
    // 首先尝试使用 rupnp 原生的 action 方法（适用于Windows Media Player等标准设备）
    match service.action(base_url, action, args_xml).await {
        Ok(response) => {
            log::debug!("UPnP Action (native) succeeded");
            log::debug!("UPnP Action (native) response: {:?}", response);
            return Ok(response);
        }
        Err(e) => {
            log::warn!(
                "UPnP Action (native) failed: {}, trying compatibility mode",
                e
            );
        }
    }

    // 原生方法失败，尝试兼容性模式

    // 从 debug 输出中我们可以看到 service 的结构
    // 我们可以通过 Debug 表示式提取 control_endpoint 信息
    let service_debug = format!("{:?}", service);
    log::debug!("Service Debug info: {}", service_debug);

    let host = base_url
        .host()
        .ok_or(rupnp::Error::ParseError("base_url缺少host"))?
        .to_string();
    let scheme = base_url
        .scheme_str()
        .ok_or(rupnp::Error::ParseError("base_url缺少scheme"))?;
    let port = base_url
        .port_u16()
        .unwrap_or(if scheme == "https" { 443 } else { 80 });

    // 候选控制路径：优先使用 debug 中的 control_endpoint，并补充常见路径
    let mut possible_paths: Vec<String> = Vec::new();

    if let Some(path) = extract_control_endpoint_from_debug(&service_debug) {
        possible_paths.push(normalize_control_path(&path));
    }

    // 尝试从 debug 中解析出真实的控制路径（常见于 Windows UPnP Host）
    if let Some(start) = service_debug.find("/upnphost/udhisapi.dll?control=")
        && let Some(end) = service_debug[start..].find(", event_sub_endpoint")
    {
        let real_path = &service_debug[start..start + end];
        possible_paths.push(normalize_control_path(real_path));
    }

    // 通用回退路径
    possible_paths.extend(
        [
            "_urn:schemas-upnp-org:service:AVTransport_control",
            "AVTransport/control",
            "upnp/control/AVTransport",
            "control/AVTransport",
        ]
        .into_iter()
        .map(normalize_control_path), // 规范化路径,增加/
    );

    // 尝试匹配可能的路径模式
    for path in possible_paths {
        let final_url = if path.starts_with("http://") || path.starts_with("https://") {
            path
        } else {
            format!("{}://{}:{}{}", scheme, host, port, path)
        };

        let soap_action_header =
            format!("\"urn:schemas-upnp-org:service:AVTransport:1#{}\"", action);
        let body = build_soap_envelope(action, args_xml);

        log::debug!(
            "UPnP Action (compat) -> url={} SOAPAction={}",
            final_url,
            soap_action_header
        );
        log::debug!("UPnP Action (compat) body => {}", body);

        let mut headers = HeaderMap::new();
        headers.insert(
            "SOAPAction",
            HeaderValue::from_str(&soap_action_header)
                .map_err(|_| rupnp::Error::ParseError("SOAPAction header非法"))?,
        );
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("text/xml; charset=\"utf-8\""),
        );

        let client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .map_err(|_| rupnp::Error::ParseError("创建reqwest client失败"))?;

        match client
            .post(&final_url)
            .headers(headers)
            .body(body)
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                let text = resp.text().await.map_err(|e| {
                    rupnp::Error::ParseError(Box::leak(
                        format!("读取SOAP响应失败: {}", e).into_boxed_str(),
                    ))
                })?;

                if status.as_u16() == 200 {
                    log::debug!("UPnP Action (compat) succeeded with path: {}", final_url);
                    log::debug!("UPnP Action (compat) status=200 body={}", text);

                    let mut out = HashMap::new();
                    for k in [
                        "Track",
                        "TrackDuration",
                        "TrackMetaData",
                        "TrackURI",
                        "RelTime",
                        "AbsTime",
                        "RelCount",
                        "AbsCount",
                    ] {
                        if let Some(v) = extract_xml_tag_value(&text, k) {
                            log::debug!("提取到字段 '{}' 的值: '{}'", k, v);
                            out.insert(k.to_string(), v);
                        }
                    }

                    log::debug!("解析后的响应字段: {:?}", out);
                    return Ok(out);
                } else {
                    log::warn!(
                        "UPnP Action (compat) failed with path {}: status={} body={}",
                        final_url,
                        status,
                        text
                    );
                }
            }
            Err(e) => {
                log::warn!("UPnP Action (compat) failed with path {}: {}", final_url, e);
            }
        }
    }

    // 所有尝试都失败
    Err(rupnp::Error::ParseError(Box::leak(
        "所有AVTransport操作尝试都失败".to_string().into_boxed_str(),
    )))
}

fn normalize_control_path(path: &str) -> String {
    let p = path.trim();
    if p.starts_with("http://") || p.starts_with("https://") {
        return p.to_string();
    }
    if p.starts_with('/') {
        p.to_string()
    } else {
        format!("/{}", p)
    }
}

fn extract_control_endpoint_from_debug(service_debug: &str) -> Option<String> {
    if let Some(start) = service_debug.find("control_endpoint: ") {
        let start = start + "control_endpoint: ".len();
        if let Some(end) = service_debug[start..].find(", event_sub_endpoint") {
            let path = service_debug[start..start + end].trim();
            Some(path.to_string())
        } else {
            None
        }
    } else {
        None
    }
}

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
        log::info!("正在搜索DLNA设备...");

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

                        log::info!("发现设备: {} (位置: {})", friendly_name, location);
                        log::debug!("支持的服务: {:?}", services);

                        dlna_devices.push(DlnaDevice {
                            device,
                            friendly_name,
                            location,
                            services,
                        });
                    }
                }
                Err(e) => {
                    log::error!("设备发现错误: {}", e);
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

        log::info!("设置媒体URI: {}", media_url);
        log::debug!("元数据(传入): {}", current_uri_metadata);

        // If caller didn't provide metadata, generate a minimal DIDL-Lite for compatibility.
        let metadata = if current_uri_metadata.trim().is_empty() {
            // Title can be anything; devices often only care about protocolInfo.
            build_didl_lite_metadata(current_uri, &media_url, None)
        } else {
            current_uri_metadata.to_string()
        };

        // 准备SOAP请求参数 - 只使用标准参数以提高兼容性
        let action = "SetAVTransportURI";
        let args_str = format!(
            "<InstanceID>0</InstanceID><CurrentURI>{}</CurrentURI><CurrentURIMetaData>{}</CurrentURIMetaData>",
            xml_escape(&media_url),
            metadata
        );

        // 发送SOAP请求 - 统一使用设备描述文档URL(location)作为base url
        let base_url = device_location_uri(device)?;
        log_upnp_action(avtransport, &base_url, action, &args_str);
        let response = avtransport_action_compat(avtransport, &base_url, action, &args_str).await?;

        log::debug!("SetAVTransportURI响应: {:?}", response);

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
        let metadata = if next_uri_metadata.trim().is_empty() {
            build_didl_lite_metadata(next_uri, &media_url, None)
        } else {
            next_uri_metadata.to_string()
        };

        let args_str = format!(
            "<InstanceID>0</InstanceID><NextURI>{}</NextURI><NextURIMetaData>{}</NextURIMetaData>",
            xml_escape(&media_url),
            metadata
        );

        let base_url = device_location_uri(device)?;
        log_upnp_action(avtransport, &base_url, action, &args_str);
        let response = avtransport_action_compat(avtransport, &base_url, action, &args_str).await?;

        log::debug!("SetNextAVTransportURI响应: {:?}", response);

        Ok(())
    }

    // 播放媒体
    pub async fn play(&self, device: &DlnaDevice) -> Result<(), rupnp::Error> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or(rupnp::Error::ParseError("设备不支持AVTransport服务"))?;

        log::info!("正在发送Play指令...");
        let action = "Play";
        let args_str = "<InstanceID>0</InstanceID><Speed>1</Speed>";

        let base_url = device_location_uri(device)?;
        log_upnp_action(avtransport, &base_url, action, args_str);
        let response = avtransport_action_compat(avtransport, &base_url, action, args_str).await?;
        log::debug!("Play响应: {:?}", response);

        Ok(())
    }

    // 暂停播放
    pub async fn pause(&self, device: &DlnaDevice) -> Result<(), rupnp::Error> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or(rupnp::Error::ParseError("设备不支持AVTransport服务"))?;

        log::info!("正在发送Pause指令...");
        let action = "Pause";
        let args_str = "<InstanceID>0</InstanceID>";

        let base_url = device_location_uri(device)?;
        log_upnp_action(avtransport, &base_url, action, args_str);
        let response = avtransport_action_compat(avtransport, &base_url, action, args_str).await?;
        log::debug!("Pause响应: {:?}", response);

        Ok(())
    }

    // 停止播放
    pub async fn stop(&self, device: &DlnaDevice) -> Result<(), rupnp::Error> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or(rupnp::Error::ParseError("设备不支持AVTransport服务"))?;

        log::info!("正在发送Stop指令...");
        let action = "Stop";
        let args_str = "<InstanceID>0</InstanceID>";

        let base_url = device_location_uri(device)?;
        log_upnp_action(avtransport, &base_url, action, args_str);
        let response = avtransport_action_compat(avtransport, &base_url, action, args_str).await?;
        log::debug!("Stop响应: {:?}", response);

        Ok(())
    }

    // 下一首
    pub async fn next(&self, device: &DlnaDevice) -> Result<(), rupnp::Error> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or(rupnp::Error::ParseError("设备不支持AVTransport服务"))?;

        let action = "Next";
        let args_str = "<InstanceID>0</InstanceID>";

        let base_url = device_location_uri(device)?;
        log_upnp_action(avtransport, &base_url, action, args_str);
        let response = avtransport_action_compat(avtransport, &base_url, action, args_str).await?;
        log::debug!("Next响应: {:?}", response);

        Ok(())
    }

    // 获取传输信息
    pub async fn get_transport_info(&self, device: &DlnaDevice) -> Result<(), rupnp::Error> {
        let avtransport = self
            .get_avtransport_service(device)
            .ok_or(rupnp::Error::ParseError("设备不支持AVTransport服务"))?;

        let action = "GetTransportInfo";
        let args_str = "<InstanceID>0</InstanceID>";

        let base_url = device_location_uri(device)?;
        log_upnp_action(avtransport, &base_url, action, args_str);
        let response = avtransport_action_compat(avtransport, &base_url, action, args_str).await?;
        log::debug!("传输信息: {:?}", response);

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

        let base_url = device_location_uri(device)?;
        log_upnp_action(avtransport, &base_url, action, args_str);

        // 获取响应
        let response = avtransport_action_compat(avtransport, &base_url, action, args_str).await?;

        log::debug!("GetPositionInfo响应: {:?}", response);

        Ok(response)
    }

    // 获取当前播放进度，返回 (当前时间秒, 总时长秒)
    pub async fn get_secs(&self, device: &DlnaDevice) -> Result<(u32, u32), rupnp::Error> {
        let position_info = self.get_position_info(device).await?;

        // 获取相对时间
        let default_time = "00:00:00".to_string();
        let rel_time = position_info.get("RelTime").unwrap_or(&default_time);
        let duration = position_info
            .get("TrackDuration")
            .or_else(|| position_info.get("AbsTime"))
            .unwrap_or(&default_time);
        log::debug!(
            "get_secs() : RelTime: {}, TrackDuration: {}",
            rel_time,
            duration
        );

        // 解析时间字符串，支持格式如 "0:00:01" 和 "00:00:01"
        fn parse_time_str(time_str: &str) -> Result<NaiveTime, rupnp::Error> {
            let trimmed = time_str.trim();

            // 尝试多种时间格式
            let formats = ["%H:%M:%S", "%M:%S", "%S"];

            for fmt in &formats {
                if let Ok(time) = NaiveTime::parse_from_str(trimmed, fmt) {
                    log::debug!("成功解析时间 '{}' 格式: {}", trimmed, fmt);
                    return Ok(time);
                }
            }

            // 处理格式如 "0:00:01"（单个小时位）的情况
            if trimmed.contains(':') {
                let parts: Vec<&str> = trimmed.split(':').collect();
                if parts.len() == 3 {
                    // 确保每个部分都有正确的位数
                    let formatted = format!(
                        "{:02}:{:02}:{:02}",
                        parts[0].parse::<u32>().unwrap_or(0),
                        parts[1].parse::<u32>().unwrap_or(0),
                        parts[2].parse::<u32>().unwrap_or(0)
                    );
                    log::debug!("格式化时间 '{}' 为 '{}'", trimmed, formatted);
                    if let Ok(time) = NaiveTime::parse_from_str(&formatted, "%H:%M:%S") {
                        return Ok(time);
                    }
                }
            }

            Err(rupnp::Error::ParseError("无法解析时间字符串"))
        }

        let current_time =
            parse_time_str(rel_time).unwrap_or_else(|_| NaiveTime::from_hms_opt(0, 0, 0).unwrap());
        let total_time =
            parse_time_str(duration).unwrap_or_else(|_| NaiveTime::from_hms_opt(0, 0, 0).unwrap());

        let current_secs = current_time.num_seconds_from_midnight();
        let total_secs = total_time.num_seconds_from_midnight();

        Ok((current_secs, total_secs))
    }

    // 设置渲染器音量
    pub async fn set_volume(&self, device: &DlnaDevice, volume: u32) -> Result<(), rupnp::Error> {
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

        let base_url = device_location_uri(device)?;
        // RenderingControl uses a different service; still log with a reasonable SOAPAction.
        log::info!(
            "UPnP Action -> base_url={} service_id={} service_type={} SOAPAction=\"urn:schemas-upnp-org:service:RenderingControl:1#{}\"",
            base_url,
            rendering_control.service_id(),
            rendering_control.service_type(),
            action
        );
        log::debug!(
            "UPnP Action body (approx) => {}",
            format_args!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
                <s:Envelope s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/" xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
                <s:Body>
                    <u:{action} xmlns:u="urn:schemas-upnp-org:service:RenderingControl:1">{args}</u:{action}>
                </s:Body>
                </s:Envelope>"#,
                action = action,
                args = args_str
            )
        );

        let response = rendering_control
            .action(&base_url, action, &args_str)
            .await?;
        log::debug!("SetVolume响应: {:?}", response);

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

        let base_url = device_location_uri(device)?;
        log::info!(
            "UPnP Action -> base_url={} service_id={} service_type={} SOAPAction=\"urn:schemas-upnp-org:service:RenderingControl:1#{}\"",
            base_url,
            rendering_control.service_id(),
            rendering_control.service_type(),
            action
        );
        log::debug!(
            "UPnP Action body (approx) => {}",
            format_args!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
                <s:Envelope s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/" xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
                <s:Body>
                    <u:{action} xmlns:u="urn:schemas-upnp-org:service:RenderingControl:1">{args}</u:{action}>
                </s:Body>
                </s:Envelope>"#,
                action = action,
                args = args_str
            )
        );

        let response = rendering_control
            .action(&base_url, action, args_str)
            .await?;

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
                let result = controller
                    .set_next_avtransport_uri(
                        device,
                        "/media/test_next.mp4",
                        "",
                        "127.0.0.1".parse().unwrap(),
                        8080,
                    )
                    .await;

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
