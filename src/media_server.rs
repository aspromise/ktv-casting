use actix_files::NamedFile;
use actix_web::http::header::{HeaderName, HeaderValue};
use actix_web::{App, HttpRequest, HttpResponse, HttpServer, Result, web};
use moka::{Expiry, future::Cache};
use std::time::{Duration, Instant};
use chrono::{Utc, NaiveTime};
use url;

struct MyExpiry;

impl Expiry<String, String> for MyExpiry {
    fn expire_after_create(&self, key: &String, value: &String, _current_time: Instant) 
        -> Option<Duration> {
        // 解析 URL 中的 deadline 参数
        let url = match url::Url::parse(value) {
            Ok(url) => url,
            Err(_) => return None,
        };

        // 从查询参数中获取 deadline
        let deadline_str = match url.query_pairs().find(|(key, _)| key == "deadline") {
            Some((_, value)) => value.to_string(),
            None => return None,
        };

        // 解析 deadline 为 Unix 时间戳（秒级）
        let deadline_timestamp: i64 = match deadline_str.parse() {
            Ok(ts) => ts,
            Err(_) => return None,
        };

        // 获取当前 UTC Unix 时间戳（秒级）
        let current_timestamp = Utc::now().timestamp();

        // 计算剩余时间（秒）
        let remaining_seconds = deadline_timestamp - current_timestamp;

        // 如果已过期，返回 None 或立即过期
        if remaining_seconds <= 0 {
            return None;
        }

        // 返回剩余时间作为 Duration
        Some(Duration::from_secs(remaining_seconds as u64))
    }
    
    fn expire_after_update(&self, _key: &String, _value: &String, 
                          _current_time: Instant, _current_duration: Option<Duration>) 
        -> Option<Duration> {
        None // No change on update
    }
    
    fn expire_after_read(&self, _key: &String, _value: &String,
                        _current_time: Instant, current_duration: Option<Duration>,
                        _last_modified_at: Instant) 
        -> Option<Duration> {
        current_duration // No change on read
    }
}

struct UrlCache {
    cache: Cache<String, String>,
}

impl UrlCache {
    fn new() -> Self {
        let cache = Cache::builder()
            .max_capacity(200) // Set a default capacity
            .expire_after(MyExpiry)
            .build();
        UrlCache { cache }
    }

    async fn get(&self, key: &str) -> Option<String> {
        self.cache.get(key).await
    }

    // origin_url:未解析的BV号与p号，如`BV1cet2z9Ety?page=1`,`BV1kGYyzZEc6`
    async fn insert(&self, origin_url: &String) {
        let bv_id = origin_url[..origin_url.find('?').unwrap_or(origin_url.len())].to_string();
        let page: Option<u32> = if let Some(pos) = origin_url.find("?page=") {
            origin_url[pos + 6..].parse().ok()
        } else {
            None
        };
        let value = match crate::bilibili_parser::get_bilibili_direct_link(&bv_id, page).await {
            Ok(url) => url,
            Err(e) => {
                println!("获取视频直链失败: {}", e);
                return;
            }
        };
        let key = origin_url.clone();
        self.cache.insert(key, value).await;
    }
    
}

// 使用示例
#[cfg(test)]
mod tests {
use actix_web::{get, web, App, HttpServer, HttpResponse, Error};
use reqwest::Client;
use crate::proxy_handler;

#[tokio::test]
async fn test_https() {
    let client = reqwest::Client::new();

        match client.get("https://www.bilibili.com/")
        .header("User-Agent", "Mozilla/5.0 ...")
        .send().await {
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
