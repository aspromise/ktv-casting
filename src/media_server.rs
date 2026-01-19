// 使用示例
#[cfg(test)]
mod tests {
    use crate::proxy_handler;
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
