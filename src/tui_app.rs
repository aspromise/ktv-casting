use crate::dlna_controller::DlnaDevice;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tui_input::Input;

#[derive(Clone)]
pub enum AppState {
    Startup,       // 初始状态 - 输入房间链接
    SelectDevice,  // 选择DLNA设备
    Playing,       // 播放状态
    Paused,        // 暂停状态
    Error(String), // 错误状态
}

#[derive(Clone)]
pub struct TuiApp {
    pub state: AppState,
    pub room_input: Input,
    pub selected_device_idx: usize,
    pub devices: Vec<DlnaDevice>,
    pub current_song: Option<String>,
    pub playback_progress: (u32, u32), // (remaining_secs, total_secs)
    pub volume: u32,
    pub is_loading: bool,
    pub error_message: Option<String>,
    // 添加用于存储当前选中设备的引用
    pub selected_device: Option<DlnaDevice>,
    // 添加用于存储房间信息
    pub room_url: Option<String>,
    pub room_id: Option<u64>,
    // 添加输入去重字段
    pub last_char_input_time: Option<Instant>,
    pub last_char_input: Option<char>,
}

impl TuiApp {
    pub fn new() -> Self {
        Self {
            state: AppState::Startup,
            room_input: Input::default(),
            selected_device_idx: 0,
            devices: vec![],
            current_song: None,
            playback_progress: (0, 0),
            volume: 50, // 默认音量50%
            is_loading: false,
            error_message: None,
            selected_device: None,
            room_url: None,
            room_id: None,
            last_char_input_time: None,
            last_char_input: None,
        }
    }

    pub fn update_state(&mut self, new_state: AppState) {
        self.state = new_state;
    }

    pub fn next_device(&mut self) {
        if !self.devices.is_empty() {
            self.selected_device_idx = (self.selected_device_idx + 1) % self.devices.len();
            if !self.devices.is_empty() {
                self.selected_device = Some(self.devices[self.selected_device_idx].clone());
            }
        }
    }

    pub fn prev_device(&mut self) {
        if !self.devices.is_empty() {
            if self.selected_device_idx == 0 {
                self.selected_device_idx = self.devices.len() - 1;
            } else {
                self.selected_device_idx -= 1;
            }
            self.selected_device = Some(self.devices[self.selected_device_idx].clone());
        }
    }

    pub fn increase_volume(&mut self) {
        if self.volume < 100 {
            self.volume += 5;
        }
    }

    pub fn decrease_volume(&mut self) {
        if self.volume >= 5 {
            self.volume -= 5;
        }
    }

    pub fn toggle_playback(&mut self) {
        match self.state {
            AppState::Playing => {
                self.state = AppState::Paused;
            }
            AppState::Paused => {
                self.state = AppState::Playing;
            }
            _ => {} // 其他状态下不处理
        }
    }

    pub fn next_track(&mut self) {
        // 这里只是更新UI状态，实际的下一首歌曲逻辑在主程序中处理
        // 在实际实现中，这里会触发播放列表管理器的下一首歌曲功能
    }

    pub fn format_time(&self, seconds: u32) -> String {
        let mins = seconds / 60;
        let secs = seconds % 60;
        format!("{:02}:{:02}", mins, secs)
    }

    pub fn set_devices(&mut self, devices: Vec<DlnaDevice>) {
        self.devices = devices;
        if !self.devices.is_empty() {
            self.selected_device = Some(self.devices[0].clone());
        }
    }

    pub fn set_room_info(&mut self, url: String, id: u64) {
        self.room_url = Some(url);
        self.room_id = Some(id);
    }
}
