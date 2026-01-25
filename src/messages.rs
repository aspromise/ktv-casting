use crate::dlna_controller::DlnaDevice;

// 定义消息类型用于任务间通信
#[derive(Debug)]
pub enum Message {
    DevicesFound(Vec<DlnaDevice>),
    RoomInfo(String, u64),
    PlaybackProgress(u32, u32),
    CurrentSong(String),
    VolumeChanged(u32),
    NextTrack,
    Error(String),
}
