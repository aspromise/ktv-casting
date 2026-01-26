use crate::tui_app::{AppState, TuiApp};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, List, ListItem, ListState, Paragraph},
};

// 定义现代配色方案
const COLOR_PRIMARY: Color = Color::Rgb(30, 144, 255); // 道奇蓝 - 主色调
const COLOR_SECONDARY: Color = Color::Rgb(138, 43, 226); // 蓝紫色 - 辅助色
const COLOR_ACCENT: Color = Color::Rgb(0, 255, 255); // 青色 - 强调色
const COLOR_SUCCESS: Color = Color::Rgb(0, 200, 0); // 深绿色 - 成功状态
const COLOR_WARNING: Color = Color::Rgb(255, 165, 0); // 橙色 - 警告状态
const COLOR_ERROR: Color = Color::Rgb(255, 69, 0); // 橙红色 - 错误状态
const COLOR_BACKGROUND: Color = Color::Rgb(18, 18, 18); // 深灰黑 - 背景色
const COLOR_SURFACE: Color = Color::Rgb(30, 30, 30); // 深灰 - 表面色
const COLOR_TEXT_PRIMARY: Color = Color::Rgb(240, 240, 240); // 亮白 - 主文本
const COLOR_TEXT_SECONDARY: Color = Color::Rgb(160, 160, 160); // 中灰 - 次要文本
const COLOR_BORDER: Color = Color::Rgb(60, 60, 60); // 中深灰 - 边框色

pub fn ui(frame: &mut Frame, app: &TuiApp) {
    let size = frame.area();

    // 设置全局背景色
    frame.render_widget(
        Block::default().style(Style::default().bg(COLOR_BACKGROUND)),
        size,
    );

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
        AppState::Error(error_msg) => render_error_view(frame, chunks[1], error_msg),
    }

    // 渲染底部状态栏
    render_status_bar(frame, chunks[2], app);
}

fn render_title_bar(frame: &mut Frame, area: Rect) {
    let title = Block::default()
        .title("🎤 KTV Casting - DLNA 媒体控制台")
        .title_alignment(ratatui::layout::Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(COLOR_PRIMARY).fg(Color::White))
        .border_style(Style::default().fg(Color::White));

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
    let hint = Paragraph::new("请输入 KTV 房间链接:")
        .alignment(ratatui::layout::Alignment::Center)
        .style(
            Style::default()
                .fg(COLOR_TEXT_PRIMARY)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(hint, chunks[0]);

    // 输入框
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title("房间链接")
        .style(Style::default().bg(COLOR_SURFACE).fg(COLOR_TEXT_PRIMARY))
        .border_style(Style::default().fg(COLOR_PRIMARY));

    let input = Paragraph::new(app.room_input.value())
        .block(input_block)
        .style(Style::default().fg(COLOR_TEXT_PRIMARY));

    frame.render_widget(input, chunks[1]);

    // 显示光标
    frame.set_cursor_position((
        chunks[1].x + app.room_input.visual_cursor() as u16 + 1,
        chunks[1].y + 1,
    ));
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
    let hint = Paragraph::new("请选择 DLNA 播放设备:")
        .alignment(ratatui::layout::Alignment::Center)
        .style(
            Style::default()
                .fg(COLOR_TEXT_PRIMARY)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(hint, chunks[0]);

    // 设备列表
    let items: Vec<ListItem>;
    if app.is_loading {
        // 如果正在加载，显示加载提示
        items = vec![ListItem::new(Line::from(Span::styled(
            "🔍 正在搜索 DLNA 设备...",
            Style::default().fg(COLOR_WARNING),
        )))];
    } else if app.devices.is_empty() {
        // 如果没有设备且不在加载中，显示提示
        items = vec![ListItem::new(Line::from(Span::styled(
            "⚠️ 未找到 DLNA 设备，请确保设备在同一网络中",
            Style::default().fg(COLOR_WARNING),
        )))];
    } else {
        // 显示设备列表
        items = app
            .devices
            .iter()
            .enumerate()
            .map(|(i, device)| {
                let style = if i == app.selected_device_idx {
                    Style::default().bg(COLOR_PRIMARY).fg(Color::White)
                } else {
                    Style::default().fg(COLOR_TEXT_PRIMARY)
                };

                let content = Line::from(vec![
                    Span::raw(format!("{}. ", i)),
                    Span::styled(
                        &device.friendly_name,
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" at "),
                    Span::styled(&device.location, Style::default().fg(COLOR_ACCENT)),
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
                .title("DLNA 设备列表")
                .style(Style::default().bg(COLOR_BACKGROUND).fg(COLOR_TEXT_PRIMARY))
                .border_style(Style::default().fg(COLOR_PRIMARY)),
        )
        .highlight_style(Style::default().bg(COLOR_SECONDARY).fg(Color::White));

    frame.render_stateful_widget(list, chunks[1], &mut state);

    // 操作提示
    let controls = if app.is_loading {
        Paragraph::new("正在搜索中，请稍候...")
            .alignment(ratatui::layout::Alignment::Center)
            .style(Style::default().fg(COLOR_WARNING))
    } else if app.devices.is_empty() {
        Paragraph::new("按 Esc 返回，检查网络后重试")
            .alignment(ratatui::layout::Alignment::Center)
            .style(Style::default().fg(COLOR_WARNING))
    } else {
        Paragraph::new("使用 ↑↓ 选择设备，按 Enter 确认")
            .alignment(ratatui::layout::Alignment::Center)
            .style(Style::default().fg(COLOR_ACCENT))
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
        String::from("🎵 等待播放...")
    };

    let song_paragraph = Paragraph::new(song_text)
        .alignment(ratatui::layout::Alignment::Center)
        .style(
            Style::default()
                .fg(COLOR_SUCCESS)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(song_paragraph, info_chunks[0]);

    // 播放状态
    let (status_text, status_color) = match app.state {
        AppState::Playing => ("▶️ 播放中", COLOR_SUCCESS),
        AppState::Paused => ("⏸️ 已暂停", COLOR_WARNING),
        _ => ("⏹️ 停止", COLOR_TEXT_SECONDARY),
    };

    let status_paragraph = Paragraph::new(status_text)
        .alignment(ratatui::layout::Alignment::Center)
        .style(
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(status_paragraph, info_chunks[1]);

    // 音量
    let volume_text = format!("🔊 音量: {}%", app.volume);
    let volume_paragraph = Paragraph::new(volume_text)
        .alignment(ratatui::layout::Alignment::Center)
        .style(
            Style::default()
                .fg(COLOR_WARNING)
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
                .title("播放进度")
                .style(Style::default().bg(COLOR_SURFACE).fg(COLOR_TEXT_PRIMARY))
                .border_style(Style::default().fg(COLOR_PRIMARY)),
        )
        .gauge_style(Style::default().fg(COLOR_SUCCESS).bg(COLOR_BACKGROUND))
        .percent(percentage)
        .label(Span::styled(
            progress_text,
            Style::default().fg(COLOR_TEXT_PRIMARY),
        ));

    frame.render_widget(gauge, area);
}

fn render_controls(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let control_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(area);

    // 播放/暂停按钮
    let (play_pause_text, play_pause_color) = match app.state {
        AppState::Playing => ("⏸️ 暂停 (Space)", COLOR_WARNING),
        AppState::Paused => ("▶️ 播放 (Space)", COLOR_SUCCESS),
        _ => ("▶️ 播放 (Space)", COLOR_SUCCESS),
    };

    let play_pause_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(COLOR_SURFACE).fg(play_pause_color))
        .border_style(Style::default().fg(play_pause_color));

    let play_pause = Paragraph::new(Line::from(Span::raw(play_pause_text)))
        .block(play_pause_block)
        .alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(play_pause, control_chunks[0]);

    // 下一首按钮
    let next_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(COLOR_SURFACE).fg(COLOR_PRIMARY))
        .border_style(Style::default().fg(COLOR_PRIMARY));

    let next = Paragraph::new(Line::from(Span::raw("⏭️ 下一首 (N)")))
        .block(next_block)
        .alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(next, control_chunks[1]);

    // 音量控制按钮
    let vol_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(COLOR_SURFACE).fg(COLOR_WARNING))
        .border_style(Style::default().fg(COLOR_WARNING));

    let vol = Paragraph::new(Line::from(Span::raw(format!("🔊 音量: {}%", app.volume))))
        .block(vol_block)
        .alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(vol, control_chunks[2]);
}

fn render_error_view(frame: &mut Frame, area: Rect, error_msg: &str) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // 错误标题
            Constraint::Min(5),    // 错误信息
            Constraint::Length(3), // 操作提示
        ])
        .split(area);

    // 错误标题
    let error_title = Paragraph::new("❌ 发生错误")
        .alignment(ratatui::layout::Alignment::Center)
        .style(
            Style::default()
                .fg(COLOR_ERROR)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(error_title, chunks[0]);

    // 错误信息
    let error_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(COLOR_SURFACE).fg(COLOR_TEXT_PRIMARY))
        .border_style(Style::default().fg(COLOR_ERROR));

    let error_para = Paragraph::new(error_msg)
        .block(error_block)
        .wrap(ratatui::widgets::Wrap { trim: true })
        .style(Style::default().fg(COLOR_ERROR));
    frame.render_widget(error_para, chunks[1]);

    // 操作提示
    let hint = Paragraph::new("按 R 重试，按 Q 退出")
        .alignment(ratatui::layout::Alignment::Center)
        .style(Style::default().fg(COLOR_WARNING));
    frame.render_widget(hint, chunks[2]);
}

fn render_status_bar(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let (status_text, status_color) = match &app.state {
        AppState::Startup => ("输入房间链接后按 Enter", COLOR_ACCENT),
        AppState::SelectDevice => ("↑↓选择设备，Enter确认", COLOR_ACCENT),
        AppState::Playing => ("Space:暂停 N:下一首 +/-:音量", COLOR_SUCCESS),
        AppState::Paused => ("Space:播放 N:下一首 +/-:音量", COLOR_WARNING),
        AppState::Error(_) => ("R:重试 Q:退出", COLOR_ERROR),
    };

    let status_block = Block::default()
        .title(Span::styled(
            status_text,
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(COLOR_SURFACE).fg(COLOR_TEXT_PRIMARY))
        .border_style(Style::default().fg(COLOR_PRIMARY));

    frame.render_widget(status_block, area);
}
