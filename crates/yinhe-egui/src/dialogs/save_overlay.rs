//! "正在保存工程"进度数据源。
//!
//! v5 保存包含全局排序 + 6 流 zstd 压缩，1.64 亿音符可达 30s+，
//! 必须给用户进度反馈。保存无法取消，toast 卡不提供 stop 按钮。

use std::sync::{Arc, Mutex};

use rust_i18n::t;
use yinhe_yin::YinProgressStage;

use crate::widgets::toast::model::ProgressSource;

/// 保存进度的共享状态：poll 线程 drain channel 后写入，toast 渲染时 pull 读取。
pub(crate) type SharedSaveProgress = Arc<Mutex<Option<(YinProgressStage, f32)>>>;

/// 保存进度数据源：渲染时读共享状态，不再每帧拷贝文案。
pub(crate) struct SaveToastSource {
    pub state: SharedSaveProgress,
    pub pause: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl SaveToastSource {
    fn current(&self) -> Option<(YinProgressStage, f32)> {
        self.state.lock().ok().and_then(|s| *s)
    }
}

impl ProgressSource for SaveToastSource {
    fn title(&self) -> String {
        t!("toast.saving").to_string()
    }
    fn fraction(&self) -> f32 {
        self.current().map(|(_, f)| f).unwrap_or(0.0)
    }
    fn cancel(&self) -> Option<std::sync::Arc<std::sync::atomic::AtomicBool>> {
        None
    }
    fn pause(&self) -> Option<std::sync::Arc<std::sync::atomic::AtomicBool>> {
        Some(self.pause.clone())
    }
}
