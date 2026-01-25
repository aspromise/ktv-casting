use crate::dlna_controller::DlnaController;
use anyhow::{Context, Result, anyhow, bail};
use bilibili_parser::get_bilibili_direct_link;
use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use log::{error, info, warn};
use playlist_manager::PlaylistManager;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};
use tokio::time::sleep;
use url::Url;

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

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("RUST_LOG").is_err() {
        unsafe {
            std::env::set_var("RUST_LOG", "INFO");
        }
    }
    env_logger::init();

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
                    if let (Some(room_url), Some(room_id)) = (&app.room_url, app.room_id) {
                        if let Some(_device) = &app.selected_device {
                            let mut playlist_manager =
                                PlaylistManager::new(room_url, room_id, playlist.clone());
                            tokio::spawn(async move {
                                match playlist_manager.next_song().await {
                                    Ok(_) => info!("切歌成功"),
                                    Err(e) => error!("切歌失败: {}", e),
                                }
                            });
                        }
                    }
                }
                Message::Error(err) => {
                    app.update_state(AppState::Error(err));
                    app.is_loading = false;  // 确保在错误情况下也停止加载状态
                }
            }
        }

        // 处理播放控制命令
        if let Some(ref device) = app.selected_device {
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

        // 根据应用状态执行相应操作
        match &app.state {
            AppState::Startup => {
                // 在启动状态下，等待用户输入房间链接
                // URL解析在输入处理部分完成（按Enter时）
            }
            AppState::SelectDevice => {
                // 如果还没有搜索过设备，则开始搜索
                if app.devices.is_empty() && !app.is_loading {
                    app.is_loading = true;

                    // 异步搜索DLNA设备
                    let tx_clone = tx.clone();
                    let controller_clone = controller.clone();

                    tokio::spawn(async move {
                        match controller_clone.discover_devices().await {
                            Ok(devices) => {
                                if devices.is_empty() {
                                    // 如果没有找到设备，发送错误消息
                                    let _ = tx_clone
                                        .send(Message::Error("未找到DLNA设备，请确保设备在同一网络中".to_string()));
                                } else {
                                    let _ = tx_clone.send(Message::DevicesFound(devices));
                                }
                            }
                            Err(e) => {
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
                    if app.selected_device.is_some() {
                        let playlist_manager =
                            PlaylistManager::new(room_url, room_id, playlist.clone());
                        let controller_clone = controller.clone();
                        let device = app.selected_device.as_ref().unwrap().clone();
                        let tx_clone = tx.clone();

                        // 创建需要在多个任务中使用的克隆值
                        let room_url_clone = room_url.clone();
                        let room_id_clone = room_id;
                        let playlist_clone = playlist.clone();

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
                        let mut playlist_manager_for_monitor =
                            PlaylistManager::new(&room_url_clone, room_id_clone, playlist_clone);
                        tokio::spawn(async move {
                            let mut interval =
                                tokio::time::interval(std::time::Duration::from_secs(1));
                            loop {
                                interval.tick().await;
                                match controller_clone.get_secs(&device).await {
                                    Ok((remaining, total)) => {
                                        let _ = tx_clone
                                            .send(Message::PlaybackProgress(remaining, total));

                                        // 如果快播完了，自动切歌
                                        if remaining <= 2 && total > 0 {
                                            info!(
                                                "剩余时间{}秒，总时间{}秒，准备切歌",
                                                remaining, total
                                            );

                                            // 发起切歌请求
                                            match playlist_manager_for_monitor.next_song().await {
                                                Ok(_) => info!("切歌请求已发送"),
                                                Err(e) => error!("切歌失败: {}", e),
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        warn!("获取播放进度失败: {}", e);
                                    }
                                }
                            }
                        });

                        // 启动播放状态和音量监控
                        let controller_clone = controller.clone();
                        let device = app.selected_device.as_ref().unwrap().clone();
                        let tx_clone = tx.clone();
                        tokio::spawn(async move {
                            let mut interval =
                                tokio::time::interval(std::time::Duration::from_secs(2));
                            loop {
                                interval.tick().await;
                                // 获取播放状态
                                match controller_clone.get_playback_state(&device).await {
                                    Ok(state) => {
                                        info!("播放状态: {}", state);
                                        // 根据播放状态更新UI - 这里我们简单记录状态，实际UI更新通过其他方式处理
                                    }
                                    Err(e) => {
                                        warn!("获取播放状态失败: {}", e);
                                    }
                                }

                                // 获取音量
                                match controller_clone.get_volume(&device).await {
                                    Ok(volume) => {
                                        let _ = tx_clone.send(Message::VolumeChanged(volume));
                                    }
                                    Err(e) => {
                                        warn!("获取音量失败: {}", e);
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
    tx: mpsc::UnboundedSender<Message>,
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
