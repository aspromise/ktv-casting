// 使用示例
use crate::SharedState;
use crate::bilibili_parser::get_bilibili_direct_link;
use crate::mp4_util::get_mp4_duration;
use actix_web::{HttpRequest, HttpResponse, get, web};
use futures_util::StreamExt;
use log::info;

#[get("/{url:.*}")]
pub async fn proxy_handler(
    req: HttpRequest,
    path: web::Path<(String,)>,
    client: web::Data<reqwest::Client>,
    shared_state: web::Data<SharedState>,
) -> Result<HttpResponse, actix_web::Error> {
    let (origin_url,) = path.into_inner();
    let range_hdr = req
        .headers()
        .get(actix_web::http::header::RANGE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("<none>");
    let if_range_hdr = req
        .headers()
        .get(actix_web::http::header::IF_RANGE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("<none>");

    info!(
        "Proxy request: method={} path={} origin_url={} Range={} If-Range={}",
        req.method(),
        req.path(),
        origin_url,
        range_hdr,
        if_range_hdr
    );

    let bv_id = &origin_url[..origin_url.find('-').unwrap_or(origin_url.len())];
    let page: Option<u32> = if let Some(pos) = origin_url.find("-page") {
        origin_url[pos + 5..].parse().ok()
    } else {
        None
    };

    info!("Proxy parsed: bv_id={} page={:?}", bv_id, page);

    let target_url = get_bilibili_direct_link(bv_id, page)
        .await
        .map_err(actix_web::error::ErrorInternalServerError)?;

    info!("Proxy resolved target_url={}", target_url);

    // 异步获取视频时长并存入缓存
    let duration_cache = shared_state.duration_cache.clone();
    let origin_url_clone = origin_url.clone();
    let target_url_clone = target_url.clone();
    tokio::spawn(async move {
        // 先检查缓存中是否已有该视频的时长
        {
            let cache = duration_cache.lock().await;
            if cache.contains_key(&origin_url_clone) {
                return;
            }
        }

        match get_mp4_duration(&target_url_clone).await {
            Ok(duration) => {
                let mut cache = duration_cache.lock().await;
                cache.insert(origin_url_clone, duration.as_secs() as u32);
                info!(
                    "成功获取并缓存视频时长: {} -> {}s",
                    target_url_clone,
                    duration.as_secs()
                );
            }
            Err(e) => {
                log::warn!("无法获取视频时长: {}", e);
            }
        }
    });

    // DLNA renderers often probe with HEAD and/or send Range requests.
    let mut upstream = match *req.method() {
        actix_web::http::Method::HEAD => client.head(&target_url),
        _ => client.get(&target_url),
    };

    upstream = upstream
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/118.0.0.0 Safari/537.36")
        .header("Referer", "https://www.bilibili.com/");

    // Forward Range-related headers to support seek/probe.
    if let Some(range) = req.headers().get(actix_web::http::header::RANGE) {
        upstream = upstream.header("Range", range.as_bytes());
    }
    if let Some(if_range) = req.headers().get(actix_web::http::header::IF_RANGE) {
        upstream = upstream.header("If-Range", if_range.as_bytes());
    }

    let response = upstream
        .send()
        .await
        .map_err(actix_web::error::ErrorInternalServerError)?;

    let ct = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("<none>");
    let cl = response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("<none>");
    let ar = response
        .headers()
        .get("accept-ranges")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("<none>");
    let cr = response
        .headers()
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("<none>");

    info!(
        "Proxy upstream: status={} Content-Type={} Content-Length={} Accept-Ranges={} Content-Range={}",
        response.status(),
        ct,
        cl,
        ar,
        cr
    );

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

    // Some renderers require this header to decide whether they can seek.
    if !response.headers().contains_key("accept-ranges") {
        client_resp.insert_header(("accept-ranges", "bytes"));
    }

    // HEAD should not include a body.
    if *req.method() == actix_web::http::Method::HEAD {
        return Ok(client_resp.finish());
    }

    let body_stream = response
        .bytes_stream()
        .map(|item| item.map_err(std::io::Error::other));

    Ok(client_resp.streaming(body_stream))
}

#[cfg(test)]
mod tests {
    use crate::media_server::proxy_handler;
    use actix_web::{App, HttpServer, web};
    use reqwest::Client;

    #[tokio::test]
    async fn test_https() {
        let client = reqwest::Client::new();

        match client
            .get("https://www.bilibili.com/")
            .header("User-Agent", "Mozilla/5.0 ...")
            .send()
            .await
        {
            Ok(res) => println!("成功连接! 状态码: {}", res.status()),
            Err(e) => println!("连接失败: {:?}. 请检查网络连接。", e),
        }
    }
    #[tokio::test]
    async fn test_proxy() -> std::io::Result<()> {
        // 在外面创建全局唯一的 Client，内部已配置好纯 Rustls
        let client = Client::builder()
            .use_rustls_tls() // 强制使用 rustls
            .build()
            .expect("Failed to create client");

        let client_data = web::Data::new(client);

        HttpServer::new(move || {
            App::new()
                .app_data(client_data.clone())
                .service(proxy_handler)
        })
        .bind(("127.0.0.1", 8080))?
        .run()
        .await
    }
}
