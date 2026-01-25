use crate::messages::Message;
use crate::tui_app::{AppState, TuiApp};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;
use tui_input::InputRequest;

pub async fn handle_events(
    app: &mut TuiApp,
    tx: &mpsc::UnboundedSender<Message>,
) -> Result<bool, Box<dyn std::error::Error>> {
    // 使用较短的超时时间以获得更好的响应性
    if event::poll(std::time::Duration::from_millis(10))? {
        if let Event::Key(key) = event::read()? {
            match &app.state {
                AppState::Startup => handle_startup_events(app, key)?,
                AppState::SelectDevice => handle_device_selection_events(app, key)?,
                AppState::Playing | AppState::Paused => handle_player_events(app, key, tx)?,
                AppState::Error(_) => handle_error_events(app, key)?,
            }
        }
    }
    Ok(true)
}

fn handle_startup_events(
    app: &mut TuiApp,
    key: KeyEvent,
) -> Result<(), Box<dyn std::error::Error>> {
    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            // 退出应用
            return Err("quit".into());
        }
        KeyCode::Enter => {
            // 确认房间链接，开始搜索DLNA设备
            if !app.room_input.value().is_empty() {
                // 解析房间链接
                if let Ok(parsed_url) = url::Url::parse(app.room_input.value()) {
                    if let Some(host) = parsed_url.host_str() {
                        let base_url = format!(
                            "{}://{}",
                            parsed_url.scheme(),
                            host
                        );

                        let segments: Vec<&str> = parsed_url
                            .path_segments()
                            .map(|s| s.filter(|seg| !seg.is_empty()).collect())
                            .unwrap_or_default();

                        if !segments.is_empty() {
                            let room_str = segments.last().unwrap();
                            if let Ok(room_id) = room_str.parse::<u64>() {
                                app.set_room_info(base_url, room_id);
                                app.is_loading = true;
                                // 这里应该触发搜索DLNA设备的异步操作
                                // 在实际实现中，我们会调用DLNA控制器来搜索设备
                                app.update_state(AppState::SelectDevice);
                            } else {
                                // 房间ID不是数字，显示错误
                                app.update_state(AppState::Error(format!("房间ID必须是数字")));
                            }
                        } else {
                            // 没有路径段，显示错误
                            app.update_state(AppState::Error(format!("URL 中没有找到房间ID")));
                        }
                    } else {
                        // URL没有主机，显示错误
                        app.update_state(AppState::Error(format!("URL 没有主机")));
                    }
                } else {
                    // URL格式无效，显示错误
                    app.update_state(AppState::Error(format!("无效的URL格式")));
                }
            }
        }
        KeyCode::Backspace => {
            app.room_input.handle(InputRequest::DeletePrevChar);
        }
        KeyCode::Char(c) => {
            // 直接处理字符输入，不进行时间过滤以避免误删正常重复字符
            app.last_char_input = Some(c);
            app.last_char_input_time = Some(std::time::Instant::now());
            app.room_input.handle(InputRequest::InsertChar(c));
        }
        KeyCode::Left => {
            app.room_input.handle(InputRequest::GoToPrevChar);
        }
        KeyCode::Right => {
            app.room_input.handle(InputRequest::GoToNextChar);
        }
        _ => {}
    }
    Ok(())
}

fn handle_device_selection_events(
    app: &mut TuiApp,
    key: KeyEvent,
) -> Result<(), Box<dyn std::error::Error>> {
    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            return Err("quit".into());
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.prev_device();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.next_device();
        }
        KeyCode::Enter => {
            // 确认选择设备，进入播放状态
            if !app.devices.is_empty() && app.selected_device_idx < app.devices.len() {
                app.update_state(AppState::Playing);
            }
        }
        KeyCode::Esc => {
            // 返回上一状态（输入房间链接）
            app.update_state(AppState::Startup);
        }
        _ => {}
    }
    Ok(())
}

fn handle_player_events(
    app: &mut TuiApp,
    key: KeyEvent,
    tx: &mpsc::UnboundedSender<Message>,
) -> Result<(), Box<dyn std::error::Error>> {
    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            return Err("quit".into());
        }
        KeyCode::Char(' ') => {
            // 空格键：播放/暂停切换
            app.toggle_playback();
            // 实际的播放/暂停控制将在主循环中处理
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            // N键：下一首
            app.next_track();
            // 发送下一首歌曲的消息
            let _ = tx.send(Message::NextTrack);
        }
        KeyCode::Char('p') | KeyCode::Char('P') => {
            // P键：上一首
            // 注意：在当前实现中，我们只支持下一首功能，因为原项目中主要是自动切歌
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            // +/=键：增加音量
            app.increase_volume();
            // 实际的音量控制将在主循环中处理
        }
        KeyCode::Char('-') => {
            // -键：减少音量
            app.decrease_volume();
            // 实际的音量控制将在主循环中处理
        }
        KeyCode::Char('m') | KeyCode::Char('M') => {
            // M键：静音切换
            // 这里可以实现静音功能，暂时不做
        }
        _ => {}
    }
    Ok(())
}

fn handle_error_events(app: &mut TuiApp, key: KeyEvent) -> Result<(), Box<dyn std::error::Error>> {
    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            return Err("quit".into());
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            // R键：重试
            app.update_state(AppState::Startup);
            app.error_message = None;
        }
        _ => {}
    }
    Ok(())
}
