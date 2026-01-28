use anyhow::{Result, anyhow};
use reqwest::Client;
use std::io::Cursor;
use std::time::Duration;

pub async fn get_mp4_duration(url: &str) -> Result<Duration> {
    let client = Client::builder().use_rustls_tls().build()?;

    // 1. 先尝试获取前 2MB 数据，这通常足以包含大部分视频的 moov 块
    let response = client.get(url)
        .header("Range", "bytes=0-2097151") // 读取前 2MB
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/118.0.0.0 Safari/537.36")
        .header("Referer", "https://www.bilibili.com/")
        .send()
        .await?;

    if !response.status().is_success() && response.status() != reqwest::StatusCode::PARTIAL_CONTENT
    {
        return Err(anyhow!(
            "Failed to fetch video header: status {}",
            response.status()
        ));
    }

    // 从 Content-Range 中提取文件的真实总大小
    // 格式通常为: bytes 0-2097151/总大小
    let total_size = response
        .headers()
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split('/').last())
        .and_then(|s| s.parse::<u64>().ok())
        .or_else(|| {
            response
                .headers()
                .get("content-length")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
        })
        .unwrap_or(2097152); // 回退值

    let bytes = response.bytes().await?;
    let mut cursor = Cursor::new(&bytes);

    // 关键点：传入总文件大小 total_size，而不是缓冲区大小 bytes.len()
    // 这样 mp4 crate 就不会因为发现 box 大于当前已读取的字节而报错，
    // 而是会尝试在 cursor 中继续读取。如果读到末尾还没读完 box，会返回 UnexpectedEof。
    match mp4::Mp4Reader::read_header(&mut cursor, total_size) {
        Ok(mp4) => Ok(mp4.duration()),
        Err(e) => {
            // 如果 2MB 还是不够（例如 moov 非常大），且报错是 UnexpectedEof，可以考虑在这里增加重试逻辑
            // 但对于一般 B 站视频，2MB 配合正确的 total_size 参数应该足够解决问题。
            Err(anyhow!(
                "Failed to parse MP4 header (total_size={}): {}",
                total_size,
                e
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bilibili_parser::get_bilibili_direct_link;

    #[tokio::test]
    async fn test_get_duration_from_bilibili() {
        let bv_id = "BV1DWrABZEPi";

        println!("正在为 {} 获取直链...", bv_id);
        let direct_link = get_bilibili_direct_link(bv_id, None)
            .await
            .expect("获取直链失败");

        println!("获取到直链: {}", direct_link);

        let duration = get_mp4_duration(&direct_link).await.expect("解析时长失败");

        println!("解析成功！视频时长为: {:?}", duration);
        assert!(duration.as_secs() > 0, "时长应该大于0");
    }
}
