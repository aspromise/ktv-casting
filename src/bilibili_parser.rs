use reqwest::Client;
use serde_json::Value;
use log::warn;

/// 获取BiliBili视频直链
///
/// # Arguments
/// * `bv_id` - 视频BV号（例如："BV1AP411x7YW"）
/// * `page` - 分P页码，默认为1
///
/// # Returns
/// * `Result<String, String>` - 返回直链URL或错误信息
pub async fn get_bilibili_direct_link(bv_id: &str, page: Option<u32>) -> Result<String, String> {
    let client = Client::new();
    let mut page = page.unwrap_or(1);

    // Page is 1-based for bilibili APIs. Guard against 0 to avoid (page - 1) underflow.
    if page == 0 {
        warn!("Invalid page number: page must start from 1. Defaulting to page 1.");
        page = 1;
    }

    // 第一步：获取CID
    let cid = get_video_cid(&client, bv_id, page).await?;

    // 第二步：获取视频直链
    get_video_url(&client, bv_id, &cid).await
}

/// 获取视频的CID（分集ID）
async fn get_video_cid(client: &Client, bv_id: &str, page: u32) -> Result<String, String> {
    let url = format!("https://api.bilibili.com/x/player/pagelist?bvid={}", bv_id);

    let response = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await
        .map_err(|e| format!("请求CID失败: {}", e))?;

    let json: Value = response
        .json()
        .await
        .map_err(|e| format!("解析JSON失败: {}", e))?;

    // 检查API返回状态
    if json["code"].as_i64() != Some(0) {
        return Err(format!(
            "API错误:  {}",
            json.get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("未知错误")
        ));
    }

    // 检查分P是否存在
    let data = json
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| "无效的数据格式".to_string())?;

    if data.is_empty() {
        return Err("该视频没有可用的分P数据".to_string());
    }

    // bilibili page is 1-based.
    if page == 0 {
        return Err("无效的分P: page 必须从 1 开始".to_string());
    }

    let idx = (page - 1) as usize;
    if idx >= data.len() {
        return Err(format!(
            "无效的分P: page={}, 有效范围: 1..={}, 总分P数: {}",
            page,
            data.len(),
            data.len()
        ));
    }

    // 获取指定分P的CID
    let cid = data[idx]
        .get("cid")
        .and_then(|c| c.as_u64())
        .ok_or_else(|| "无法获取CID".to_string())?;

    Ok(cid.to_string())
}

/// 获取视频播放链接
async fn get_video_url(client: &Client, bv_id: &str, cid: &str) -> Result<String, String> {
    let url = format!(
        "https://api.bilibili.com/x/player/playurl?bvid={}&cid={}&qn=116&type=&otype=json&platform=html5&high_quality=1",
        bv_id, cid
    );

    let response = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await
        .map_err(|e| format!("请求视频链接失败: {}", e))?;

    let json: Value = response
        .json()
        .await
        .map_err(|e| format!("解析JSON失败:  {}", e))?;

    // 检查API返回状态
    if json["code"].as_i64() != Some(0) {
        return Err(format!(
            "API错误: {}",
            json.get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("未知错误")
        ));
    }

    // 提取直链
    let video_url = json
        .get("data")
        .and_then(|d| d.get("durl"))
        .and_then(|d| d.get(0))
        .and_then(|d| d.get("url"))
        .and_then(|u| u.as_str())
        .ok_or_else(|| "无法获取视频链接".to_string())?;

    Ok(video_url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_bilibili_direct_link() {
        // 示例：测试获取视频直链
        match get_bilibili_direct_link("BV1LS4MzKE8y", Some(2)).await {
            Ok(url) => println!("视频直链: {}", url),
            Err(e) => println!("错误: {}", e),
        }
    }
}
