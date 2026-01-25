use crate::tui_app::{AppState, TuiApp};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, List, ListItem, ListState, Paragraph},
};

pub fn ui(frame: &mut Frame, app: &TuiApp) {
    let size = frame.size();

    // 创建主布局
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // 顶部标题栏
            Constraint::Min(10),   // 主内容区
            Constraint::Length(3), // 底部状态栏
        ])
        .split(size);

    // 渲染顶部标题栏
    render_title_bar(frame, chunks[0]);

    // 根据不同状态渲染主内容区域
    match &app.state {
        AppState::Startup => render_startup_view(frame, chunks[1], app),
        AppState::SelectDevice => render_device_selection_view(frame, chunks[1], app),
        AppState::Playing | AppState::Paused => render_player_view(frame, chunks[1], app),
        AppState::Error(error_msg) => render_error_view(frame, chunks[1], app, error_msg),
    }

    // 渲染底部状态栏
    render_status_bar(frame, chunks[2], app);
}

fn render_title_bar(frame: &mut Frame, area: Rect) {
    let title = Block::default()
        .title("KTV Casting - DLNA 控制台")
        .title_alignment(ratatui::layout::Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(Color::Blue).fg(Color::White));

    frame.render_widget(title, area);
}

fn render_startup_view(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // 提示文字
            Constraint::Length(3), // 输入框
            Constraint::Min(1),    // 空白区域
        ])
        .split(area);

    // 提示文字
    let hint = Paragraph::new("请输入房间链接:")
        .alignment(ratatui::layout::Alignment::Center)
        .style(Style::default().add_modifier(Modifier::BOLD));
    frame.render_widget(hint, chunks[0]);

    // 输入框
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title("房间链接")
        .style(Style::default().bg(Color::DarkGray));

    let input = Paragraph::new(app.room_input.value())
        .block(input_block)
        .style(Style::default().fg(Color::White));

    frame.render_widget(input, chunks[1]);

    // 显示光标
    frame.set_cursor(
        chunks[1].x + app.room_input.visual_cursor() as u16 + 1,
        chunks[1].y + 1,
    );
}

fn render_device_selection_view(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // 提示文字
            Constraint::Min(5),    // 设备列表
            Constraint::Length(3), // 操作提示
        ])
        .split(area);

    // 提示文字
    let hint = Paragraph::new("请选择DLNA设备:")
        .alignment(ratatui::layout::Alignment::Center)
        .style(Style::default().add_modifier(Modifier::BOLD));
    frame.render_widget(hint, chunks[0]);

    // 设备列表
    let items: Vec<ListItem>;
    if app.is_loading {
        // 如果正在加载，显示加载提示
        items = vec![ListItem::new(
            Line::from(Span::styled(
                "正在搜索DLNA设备...",
                Style::default().fg(Color::Yellow)
            ))
        )];
    } else if app.devices.is_empty() {
        // 如果没有设备且不在加载中，显示提示
        items = vec![ListItem::new(
            Line::from(Span::styled(
                "未找到DLNA设备，请确保设备在同一网络中",
                Style::default().fg(Color::Red)
            ))
        )];
    } else {
        // 显示设备列表
        items = app
            .devices
            .iter()
            .enumerate()
            .map(|(i, device)| {
                let style = if i == app.selected_device_idx {
                    Style::default().bg(Color::LightBlue).fg(Color::Black)
                } else {
                    Style::default().fg(Color::White)
                };

                let content = Line::from(vec![
                    Span::raw(format!("{}. ", i)),
                    Span::styled(
                        &device.friendly_name,
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" at "),
                    Span::styled(&device.location, Style::default().fg(Color::Cyan)),
                ]);

                ListItem::new(content).style(style)
            })
            .collect();
    }

    let mut state = ListState::default();
    if !app.devices.is_empty() {
        state.select(Some(app.selected_device_idx));
    }

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title("DLNA 设备列表"),
        )
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White));

    frame.render_stateful_widget(list, chunks[1], &mut state);

    // 操作提示
    let controls = if app.is_loading {
        Paragraph::new("正在搜索中...")
            .alignment(ratatui::layout::Alignment::Center)
            .style(Style::default().fg(Color::Yellow))
    } else if app.devices.is_empty() {
        Paragraph::new("按 Esc 返回，稍后重试")
            .alignment(ratatui::layout::Alignment::Center)
            .style(Style::default().fg(Color::Yellow))
    } else {
        Paragraph::new("使用 ↑↓ 选择设备，按 Enter 确认")
            .alignment(ratatui::layout::Alignment::Center)
            .style(Style::default().fg(Color::Yellow))
    };
    frame.render_widget(controls, chunks[2]);
}

fn render_player_view(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40), // 播放信息区
            Constraint::Percentage(30), // 进度条区
            Constraint::Percentage(30), // 控制按钮区
        ])
        .split(area);

    // 播放信息区
    render_player_info(frame, chunks[0], app);

    // 进度条区
    render_progress_bar(frame, chunks[1], app);

    // 控制按钮区
    render_controls(frame, chunks[2], app);
}

fn render_player_info(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let info_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // 当前歌曲
            Constraint::Length(3), // 播放状态
            Constraint::Length(3), // 音量
        ])
        .split(area);

    // 当前歌曲
    let song_text = if let Some(ref song) = app.current_song {
        format!("🎵 正在播放: {}", song)
    } else {
        "🎵 当前无播放内容".to_string()
    };

    let song_paragraph = Paragraph::new(song_text)
        .alignment(ratatui::layout::Alignment::Center)
        .style(
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(song_paragraph, info_chunks[0]);

    // 播放状态
    let status_text = match app.state {
        AppState::Playing => "▶️ 播放中",
        AppState::Paused => "⏸️ 已暂停",
        _ => "⏹️ 停止",
    };

    let status_paragraph = Paragraph::new(status_text)
        .alignment(ratatui::layout::Alignment::Center)
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(status_paragraph, info_chunks[1]);

    // 音量
    let volume_text = format!("🔊 音量: {}%", app.volume);
    let volume_paragraph = Paragraph::new(volume_text)
        .alignment(ratatui::layout::Alignment::Center)
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(volume_paragraph, info_chunks[2]);
}

fn render_progress_bar(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let (remaining, total) = app.playback_progress;
    let elapsed = if total > 0 && remaining <= total {
        total - remaining
    } else {
        0
    };

    let percentage = if total > 0 {
        (elapsed as f64 / total as f64 * 100.0).round() as u16
    } else {
        0
    };

    let progress_text = format!("{} / {}", app.format_time(elapsed), app.format_time(total));

    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title("播放进度"),
        )
        .gauge_style(Style::default().fg(Color::Green))
        .percent(percentage)
        .label(Span::raw(progress_text));

    frame.render_widget(gauge, area);
}

fn render_controls(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let control_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(area);

    // 播放/暂停按钮
    let play_pause_text = match app.state {
        AppState::Playing => "⏸️ 暂停 (Space)",
        AppState::Paused => "▶️ 播放 (Space)",
        _ => "▶️ 播放 (Space)",
    };

    let play_pause_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(
            Style::default().fg(if matches!(app.state, AppState::Playing) {
                Color::Red
            } else {
                Color::Green
            }),
        );

    let play_pause = Paragraph::new(Line::from(Span::raw(play_pause_text)))
        .block(play_pause_block)
        .alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(play_pause, control_chunks[0]);

    // 上一首按钮
    let prev_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().fg(Color::Blue));

    let prev = Paragraph::new(Line::from(Span::raw("⏮️ 上一首 (P)")))
        .block(prev_block)
        .alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(prev, control_chunks[1]);

    // 下一首按钮
    let next_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().fg(Color::Blue));

    let next = Paragraph::new(Line::from(Span::raw("⏭️ 下一首 (N)")))
        .block(next_block)
        .alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(next, control_chunks[2]);

    // 音量控制按钮
    let vol_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().fg(Color::Yellow));

    let vol = Paragraph::new(Line::from(Span::raw(format!("🔊 音量 ({})", app.volume))))
        .block(vol_block)
        .alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(vol, control_chunks[3]);
}

fn render_error_view(frame: &mut Frame, area: Rect, app: &TuiApp, error_msg: &str) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // 错误标题
            Constraint::Min(5),    // 错误信息
            Constraint::Length(3), // 操作提示
        ])
        .split(area);

    // 错误标题
    let error_title = Paragraph::new("❌ 错误")
        .alignment(ratatui::layout::Alignment::Center)
        .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD));
    frame.render_widget(error_title, chunks[0]);

    // 错误信息
    let error_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(Color::Red).fg(Color::White));

    let error_para = Paragraph::new(error_msg)
        .block(error_block)
        .wrap(ratatui::widgets::Wrap { trim: true });
    frame.render_widget(error_para, chunks[1]);

    // 操作提示
    let hint = Paragraph::new("按 R 重试，按 Q 退出")
        .alignment(ratatui::layout::Alignment::Center)
        .style(Style::default().fg(Color::Yellow));
    frame.render_widget(hint, chunks[2]);
}

fn render_status_bar(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let status_text = match &app.state {
        AppState::Startup => "输入房间链接后按 Enter",
        AppState::SelectDevice => "↑↓选择设备，Enter确认",
        AppState::Playing => "Space:暂停 P:上一首 N:下一首 +/-:音量",
        AppState::Paused => "Space:播放 P:上一首 N:下一首 +/-:音量",
        AppState::Error(_) => "R:重试 Q:退出",
    };

    let status_block = Block::default()
        .title(status_text)
        .borders(Borders::TOP)
        .style(Style::default().bg(Color::DarkGray).fg(Color::White));

    frame.render_widget(status_block, area);
}
