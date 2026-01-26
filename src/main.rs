use crate::dlna_controller::DlnaController;
use anyhow::{Result, anyhow};
use chrono::Local;
use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use env_logger;
use log::{LevelFilter, error, info, warn};
use playlist_manager::PlaylistManager;
use ratatui::Terminal;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{Mutex, mpsc};

mod bilibili_parser;
mod dlna_controller;
mod input_handler;
mod messages;
mod playlist_manager;
mod tui_app;
mod tui_renderer;

use input_handler::handle_events;
use messages::Message;
use tui_app::{AppState, TuiApp};
use tui_renderer::ui;

struct FileLogger {
    file: Arc<StdMutex<std::fs::File>>,
    level: LevelFilter,
}

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        if let Ok(mut file) = self.file.lock() {
            let now = Local::now().format("%Y-%m-%d %H:%M:%S");
            let _ = writeln!(file, "{} [{}] {}", now, record.level(), record.args());
            let _ = file.flush();
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // 检查环境变量KTV_LOG是否设置为true来决定是否启用文件日志
    let enable_file_log = std::env::var("KTV_LOG")
        .map(|v| v.to_lowercase() == "true")
        .unwrap_or(false);

    let log_file: Option<Arc<StdMutex<std::fs::File>>> = if enable_file_log {
        if std::env::var("RUST_LOG").is_err() {
            unsafe {
                std::env::set_var("RUST_LOG", "INFO");
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open("ktv-casting.log")?;
        let file = Arc::new(StdMutex::new(file));
        if let Ok(mut file) = file.lock() {
            let now = Local::now().format("%Y-%m-%d %H:%M:%S");
            let _ = writeln!(file, "{} [INFO] 日志启动", now);
            let _ = file.flush();
        }
        Some(file)
    } else {
        unsafe {
            std::env::set_var("RUST_LOG", "off");
        }
        None
    };

    // 设置panic钩子
    if let Some(ref panic_log) = log_file {
        let panic_log = Arc::clone(panic_log);
        std::panic::set_hook(Box::new(move |info| {
            if let Ok(mut file) = panic_log.lock() {
                let now = Local::now().format("%Y-%m-%d %H:%M:%S");
                let _ = writeln!(file, "{} [PANIC] {}", now, info);
                let _ = file.flush();
            }
        }));
    }

    // 配置日志记录器
    if let Some(ref logger_file) = log_file {
        let logger = FileLogger {
            file: Arc::clone(logger_file),
            level: LevelFilter::Info,
        };
        log::set_boxed_logger(Box::new(logger)).map_err(|e| anyhow!(e))?;
        log::set_max_level(LevelFilter::Info);
        info!("日志初始化完成");
    } else {
        // 如果不启用文件日志，使用默认的控制台日志
        env_logger::init();
        info!("控制台日志初始化完成");
    }

    // 初始化终端
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // 创建应用状态
    let mut app = TuiApp::new();

    // 创建用于在任务间通信的通道
    let (tx, mut rx) = mpsc::unbounded_channel();

    // 创建DLNA控制器和播放列表管理器
    let controller = DlnaController::new();
    let playlist: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
    let mut active_playlist_manager: Option<PlaylistManager> = None;

    // 主循环
    let mut should_quit = false;
    while !should_quit {
        // 渲染UI
        terminal.draw(|f| ui(f, &app))?;

        // 处理事件
        match handle_events(&mut app, &tx).await {
            Ok(continue_running) => {
                if !continue_running {
                    should_quit = true;
                }
            }
            Err(e) => {
                if e.to_string() == "quit" {
                    should_quit = true;
                } else {
                    // 设置错误状态
                    app.update_state(AppState::Error(e.to_string()));
                }
            }
        }

        // 处理从其他任务发来的消息
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Message::DevicesFound(devices) => {
                    app.set_devices(devices);
                    app.is_loading = false;
                    info!("设备搜索完成，共找到 {} 个可用设备", app.devices.len());
                }
                Message::RoomInfo(url, id) => {
                    app.set_room_info(url, id);
                }
                Message::PlaybackProgress(remaining, total) => {
                    app.playback_progress = (remaining, total);
                }
                Message::CurrentSong(song) => {
                    app.current_song = Some(song);
                }
                Message::VolumeChanged(volume) => {
                    app.volume = volume;
                }
                Message::NextTrack => {
                    // 处理下一首歌曲请求
                    if let Some(manager) = active_playlist_manager.clone() {
                        let mut playlist_manager = manager;
                        tokio::spawn(async move {
                            loop {
                                match playlist_manager.next_song().await {
                                    Ok(_) => {
                                        info!("切歌成功");
                                        break;
                                    }
                                    Err(e) => {
                                        let error_msg = e.to_string();
                                        error!("切歌失败: {}，500ms后重试", error_msg);
                                        tokio::time::sleep(std::time::Duration::from_millis(500))
                                            .await;
                                    }
                                }
                            }
                        });
                    }
                }
                Message::Error(err) => {
                    let err_msg = err.clone();
                    app.update_state(AppState::Error(err));
                    app.is_loading = false; // 确保在错误情况下也停止加载状态
                    error!("设备搜索错误: {}", err_msg);
                }
            }
        }

        // 离开播放状态时清理播放列表管理器
        if !matches!(app.state, AppState::Playing | AppState::Paused) {
            active_playlist_manager = None;
        }

        // 根据应用状态执行相应操作
        match &app.state {
            AppState::Startup => {
                // 在启动状态下，等待用户输入房间链接
                // URL解析在输入处理部分完成（按Enter时）
            }
            AppState::SelectDevice => {
                // 如果还没有搜索过设备，则开始搜索
                if app.devices.is_empty() && !app.device_search_started {
                    info!("进入设备选择界面，开始搜索DLNA设备...");
                    app.is_loading = true;
                    app.device_search_started = true;

                    // 异步搜索DLNA设备
                    let tx_clone = tx.clone();
                    let controller_clone = controller.clone();

                    tokio::spawn(async move {
                        info!("开始执行设备搜索任务");
                        let result = controller_clone.discover_devices().await;
                        match result {
                            Ok(devices) => {
                                info!("设备搜索任务完成，找到 {} 个设备", devices.len());
                                if devices.is_empty() {
                                    let _ = tx_clone.send(Message::DevicesFound(vec![]));
                                } else {
                                    let _ = tx_clone.send(Message::DevicesFound(devices));
                                }
                            }
                            Err(e) => {
                                error!("设备搜索任务失败: {}", e);
                                let _ = tx_clone.send(Message::DevicesFound(vec![]));
                                let _ = tx_clone
                                    .send(Message::Error(format!("搜索DLNA设备失败: {}", e)));
                            }
                        }
                    });
                }
            }
            AppState::Playing | AppState::Paused => {
                // 在播放状态下，如果还没有播放列表管理器，则创建它
                if let (Some(room_url), Some(room_id)) = (&app.room_url, app.room_id) {
                    if app.selected_device.is_some() && !app.playback_tasks_started {
                        app.playback_tasks_started = true;
                        let tx_clone = tx.clone();
                        let playlist_manager = 
                            PlaylistManager::new(room_url, room_id, playlist.clone(), Some(tx_clone.clone()));
                        active_playlist_manager = Some(playlist_manager.clone());
                        let controller_clone = controller.clone();
                        let device = app.selected_device.as_ref().unwrap().clone();

                        // 启动播放列表更新
                        playlist_manager.start_periodic_update(move |url| {
                            let controller = controller_clone.clone();
                            let device = device.clone();
                            let tx = tx_clone.clone();
                            Box::pin(async move {
                                // 播放URL
                                play_url(&controller, &device, &url, tx).await;
                            })
                        });

                        // 启动进度监控
                        let controller_clone = controller.clone();
                        let device = app.selected_device.as_ref().unwrap().clone();
                        let tx_clone = tx.clone();
                        let mut playlist_manager_for_monitor = playlist_manager.clone();
                        tokio::spawn(async move {
                            let mut interval =
                                tokio::time::interval(std::time::Duration::from_secs(1));
                            let mut next_cooldown_until = std::time::Instant::now();
                            loop {
                                interval.tick().await;
                                match controller_clone.get_secs(&device).await {
                                    Ok((remaining, total)) => {
                                        let _ = tx_clone
                                            .send(Message::PlaybackProgress(remaining, total));

                                        // 如果快播完了，自动切歌（带冷却）
                                        if remaining <= 2
                                            && total > 0
                                            && std::time::Instant::now() >= next_cooldown_until
                                        {
                                            info!(
                                                "剩余时间{}秒，总时间{}秒，准备切歌",
                                                remaining, total
                                            );

                                            // 重试next_song
                                            loop {
                                                match playlist_manager_for_monitor.next_song().await
                                                {
                                                    Ok(_) => {
                                                        info!("切歌成功");
                                                        next_cooldown_until =
                                                            std::time::Instant::now()
                                                                + std::time::Duration::from_secs(5);
                                                        break;
                                                    }
                                                    Err(e) => {
                                                        let error_msg = e.to_string();
                                                        error!(
                                                            "切歌失败: {}，500ms后重试",
                                                            error_msg
                                                        );
                                                        tokio::time::sleep(
                                                            std::time::Duration::from_millis(500),
                                                        )
                                                        .await;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        warn!("获取播放进度失败: {}", e);
                                    }
                                }
                            }
                        });
                    }
                }
            }
            AppState::Error(_) => {
                // 错误状态下等待用户操作
            }
        }

        // 处理播放控制命令（节流）
        if let Some(ref device) = app.selected_device {
            let now = std::time::Instant::now();
            let should_sync_playback = app
                .last_playback_sync
                .map(|t| now.duration_since(t) >= std::time::Duration::from_millis(500))
                .unwrap_or(true);
            let should_sync_volume = app
                .last_volume_sync
                .map(|t| now.duration_since(t) >= std::time::Duration::from_millis(500))
                .unwrap_or(true);

            if should_sync_playback {
                app.last_playback_sync = Some(now);
                // 处理播放/暂停状态变化
                if let AppState::Playing = app.state {
                    // 检查是否需要发送播放命令
                    match controller.get_playback_state(device).await {
                        Ok(state) if state != "PLAYING" => {
                            // 发送播放命令
                            if let Err(e) = controller.play(device).await {
                                error!("发送播放命令失败: {}", e);
                            }
                        }
                        Err(e) => {
                            error!("获取播放状态失败: {}", e);
                        }
                        _ => {} // 状态已经是PLAYING，无需操作
                    }
                } else if let AppState::Paused = app.state {
                    // 检查是否需要发送暂停命令
                    match controller.get_playback_state(device).await {
                        Ok(state) if state == "PLAYING" => {
                            // 发送暂停命令
                            if let Err(e) = controller.pause(device).await {
                                error!("发送暂停命令失败: {}", e);
                            }
                        }
                        Err(e) => {
                            error!("获取播放状态失败: {}", e);
                        }
                        _ => {} // 状态已经是PAUSED或STOPPED，无需操作
                    }
                }
            }

            if should_sync_volume {
                app.last_volume_sync = Some(now);
                // 处理音量变化
                match controller.get_volume(device).await {
                    Ok(current_vol) => {
                        if current_vol != app.volume {
                            // 发送音量设置命令
                            if let Err(e) = controller.set_volume(device, app.volume).await {
                                error!("设置音量失败: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        error!("获取当前音量失败: {}", e);
                    }
                }
            }
        }

        // 添加延迟以防止CPU占用过高
        tokio::time::sleep(std::time::Duration::from_millis(5)).await; // 更高的响应性

        // 模拟更新一些状态（在实际实现中，这些会来自实际的DLNA控制器）
        if matches!(app.state, AppState::Playing) {
            // 播放进度由实际的DLNA控制器提供，不需要模拟
        }
    }

    // 恢复终端
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    println!("应用已退出");
    Ok(())
}

// 播放URL的辅助函数
async fn play_url(
    controller: &DlnaController,
    device: &dlna_controller::DlnaDevice,
    url: &str,
    _tx: mpsc::UnboundedSender<Message>,
) {
    use crate::bilibili_parser::get_bilibili_direct_link;
    use std::time::Duration;
    use tokio::time::sleep;

    loop {
        match controller.stop(device).await {
            Ok(_) => {
                info!("成功停止播放");
                break;
            }
            Err(e) => {
                let error_msg = format!("{}", e);
                let error_code: Option<u32> = error_msg
                    .split(|c: char| !c.is_numeric())
                    .find(|s| s.len() == 3)
                    .and_then(|s| s.parse().ok());
                if let Some(code) = error_code
                    && code / 100 == 2
                {
                    // 2xx错误码视为成功
                    info!("停止播放返回错误码{}，视为成功", code);
                    break;
                }

                warn!("停止播放失败: {}，500ms后重试", error_msg);
                sleep(Duration::from_millis(500)).await;
            }
        }
    }

    let bv_id = &url[..url.find('-').unwrap_or(url.len())];
    let page: Option<u32> = if let Some(pos) = url.find("-page") {
        url[pos + 5..].parse().ok()
    } else {
        None
    };

    let target_url = get_bilibili_direct_link(bv_id, page).await;
    let url = match target_url {
        Ok(u) => u,
        Err(e) => {
            error!("获取视频直链失败: {}", e);
            return;
        }
    };

    loop {
        match controller.set_avtransport_uri(device, &url, "").await {
            Ok(_) => {
                info!("成功设置AVTransport URI为 {}", url);
                break;
            }
            Err(e) => {
                let error_msg = format!("{}", e);
                let error_code: Option<u32> = error_msg
                    .split(|c: char| !c.is_numeric())
                    .find(|s| s.len() == 3)
                    .and_then(|s| s.parse().ok());
                if let Some(code) = error_code
                    && code / 100 == 2
                {
                    // 2xx错误码视为成功
                    info!("设置AVTransport URI返回错误码{}，视为成功", code);
                    break;
                }

                warn!("设置AVTransport URI失败: {}，500ms后重试", error_msg);
                sleep(Duration::from_millis(500)).await;
            }
        }
    }

    // 重试play
    loop {
        match controller.play(device).await {
            Ok(_) => {
                info!("成功开始播放");
                break;
            }
            Err(e) => {
                let error_msg = format!("{}", e);
                let error_code: Option<u32> = error_msg
                    .split(|c: char| !c.is_numeric())
                    .find(|s| s.len() == 3)
                    .and_then(|s| s.parse().ok());
                if let Some(code) = error_code
                    && code / 100 == 2
                {
                    // 2xx错误码视为成功
                    info!("播放返回错误码{}，视为成功", code);
                    break;
                }

                warn!("play失败: {}，500ms后重试", error_msg);
                sleep(Duration::from_millis(500)).await;
            }
        }
    }
}
