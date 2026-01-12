use reqwest::Client;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{Duration, sleep};
#[derive(Clone)]
pub struct PlaylistManager {
    url: String,
    room_id: u64,
    hash: Arc<Mutex<Option<String>>>,
    playlist: Arc<Mutex<Vec<String>>>,
}

impl PlaylistManager {
    pub fn new(url: String, room_id: u64, playlist: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            url,
            room_id,
            hash: Arc::new(Mutex::new(None)),
            playlist,
        }
    }

    pub async fn fetch_playlist(&mut self) -> Result<(), String> {
        let client = Client::builder()
            .tls_backend_rustls()
            .build()
            .map_err(|e| format!("创建HTTP客户端失败: {}", e))?;

        let hash_guard = self.hash.lock().await;
        let last_hash = hash_guard
            .clone()
            .unwrap_or_else(|| "EMPTY_LIST_HASH".to_string());
        drop(hash_guard); // 释放锁，避免长时间持有

        let url = format!(
            "{}/api/songListInfo?roomId={}&lastHash={}",
            self.url, self.room_id, last_hash
        );

        println!("正在获取播放列表: {}", url);

        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("发送请求失败: {}", e))?;
        if !resp.status().is_success() {
            return Err(format!("请求失败，状态码: {}", resp.status()));
        }

        let resp_json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("解析JSON失败: {}", e))?;
        let changed: bool = resp_json["changed"].as_bool().unwrap_or(false);

        if !changed {
            println!("播放列表未改变，跳过更新");
            return Ok(());
        }

        // 获取新的 hash 值
        let new_hash = resp_json["hash"]
            .as_str()
            .unwrap_or_else(|| "EMPTY_LIST_HASH")
            .to_string();

        // 从 list 数组中提取所有 URL
        let urls: Vec<String> = if let Some(list_array) = resp_json["list"].as_array() {
            list_array
                .iter()
                .filter(|item| {
                    item.get("state")
                        .map_or(true, |s| s.as_str().unwrap_or("") != "sung")
                })
                .filter_map(|item| item["url"].as_str())
                .map(|url| {
                    // 提取 bilibili://video/ 后面的部分
                    if let Some(start) = url.find("bilibili://video/") {
                        let after_prefix = &url[start + "bilibili://video/".len()..];
                        after_prefix.to_string()
                    } else {
                        url.to_string()
                    }
                })
                .collect()
        } else {
            Vec::new()
        };

        println!("获取到 {} 个URL，新的hash: {}", urls.len(), new_hash);

        // 打印每个URL用于调试
        for (i, url) in urls.iter().enumerate() {
            println!("  {}. {}", i + 1, url);
        }

        // 更新播放列表
        let mut playlist = self.playlist.lock().await;
        playlist.clear();
        playlist.extend(urls);
        drop(playlist); // 释放锁，避免长时间持有

        // 更新 hash 值
        let mut hash = self.hash.lock().await;
        *hash = Some(new_hash);
        drop(hash); // 释放锁

        Ok(())
    }

    pub fn start_periodic_update(&self) {
        let mut self_clone = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                interval.tick().await;
                if let Err(e) = self_clone.fetch_playlist().await {
                    eprintln!("定时更新播放列表失败: {}", e);
                }
            }
        });
    }

    pub async fn next_song(&mut self) -> Result<(), String> {
        let url = format!("{}/api/nextSong?roomId={}", self.url, self.room_id);
        let temp_hash = self
            .hash
            .lock()
            .await
            .clone()
            .unwrap_or_else(|| "EMPTY_LIST_HASH".to_string());
        let client = Client::builder()
            .tls_backend_rustls()
            .build()
            .map_err(|e| format!("创建HTTP客户端失败: {}", e))?;
        let resp = client
            .post(&url)
            .json(&json!({"idArrayHash": temp_hash}))
            .send()
            .await
            .map_err(|e| format!("发送请求失败: {}", e))?;
        let resp_json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("解析JSON失败: {}", e))?;

        if !resp_json["success"].as_bool().unwrap_or(false) {
            return Err(format!("请求失败: {}", resp_json));
        }
        self.fetch_playlist().await?;

        Ok(())
    }
}

#[tokio::test]
async fn test_playlist_manager() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== PlaylistManager 使用示例 ===");

    let playlist = Arc::new(Mutex::new(Vec::<String>::new()));

    let mut manager = PlaylistManager::new(
        "http://localhost:5823".to_string(), 
        0, 
        playlist.clone(),
    );
    
    println!("开始获取播放列表...");
    
    // --- 第一次操作 ---
    match manager.fetch_playlist().await {
        Ok(_) => {
            println!("✓ 成功获取播放列表");
            // 【关键点 1】：用大括号包裹锁的使用
            {
                let playlist_lock = playlist.lock().await;
                println!("播放列表内容 ({} 个项目):", playlist_lock.len());
                for (i, url) in playlist_lock.iter().enumerate() {
                    println!("  {}. {}", i + 1, url);
                }
            } // <--- 锁在这里被强制释放 (DROP)
        }
        Err(e) => eprintln!("✗ 获取播放列表失败: {}", e),
    }

    // --- 第二次操作 ---
    manager.next_song().await?;
    println!("请求下一首歌曲后播放列表状态:");
    
    // 【关键点 2】：再次用大括号包裹锁
    {
        let playlist_lock = playlist.lock().await;
        for (i, url) in playlist_lock.iter().enumerate() {
            println!("  {}. {}", i + 1, url);
        }
    } // <--- 锁在这里被强制释放 (DROP)

    // --- 后台任务开始 ---
    manager.start_periodic_update();

    
    // 【关键点 3】：sleep 必须在“裸奔”状态下运行（不持有任何锁）
    // 此时 playlist 锁是空闲的，后台线程的 fetch_playlist 才能拿到锁并更新数据
    sleep(Duration::from_secs(5)).await;
    
    println!("5秒后播放列表状态:");
    
    // 【关键点 4】：休眠结束后，再次获取锁查看结果
    {
        let playlist_lock = playlist.lock().await;
        for (i, url) in playlist_lock.iter().enumerate() {
            println!("  {}. {}", i + 1, url);
        }
    }

    println!("=== 示例结束 ===");
    Ok(())
}
