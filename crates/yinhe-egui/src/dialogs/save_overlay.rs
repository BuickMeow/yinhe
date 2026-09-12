//! "正在保存工程"进度数据源。
//!
//! v5 保存包含全局排序 + 6 流 zstd 压缩，1.64 亿音符可达 30s+，
//! 必须给用户进度反馈。保存无法取消，toast 卡不提供 stop 按钮。

use std::sync::{Arc, Mutex};

use rust_i18n::t;
use yinhe_yin::YinProgressStage;

use crate::widgets::toast::model::ProgressSource;

/// 阶段 → 中文描述。
pub(crate) fn stage_label(stage: YinProgressStage) -> String {
    match stage {
        YinProgressStage::Collect => t!("dialog.saving.stage.collect").to_string(),
        YinProgressStage::Sort => t!("dialog.saving.stage.sort").to_string(),
        YinProgressStage::Compress => t!("dialog.saving.stage.compress").to_string(),
        YinProgressStage::Decompress => t!("dialog.saving.stage.decompress").to_string(),
        YinProgressStage::Rebuild => t!("dialog.saving.stage.rebuild").to_string(),
        YinProgressStage::Resort => t!("dialog.saving.stage.resort").to_string(),
    }
}

/// 保存进度的共享状态：poll 线程 drain channel 后写入，toast 渲染时 pull 读取。
pub(crate) type SharedSaveProgress = Arc<Mutex<Option<(YinProgressStage, f32)>>>;

/// 保存进度数据源：渲染时读共享状态，不再每帧拷贝文案。
pub(crate) struct SaveToastSource {
    pub state: SharedSaveProgress,
}

impl SaveToastSource {
    fn current(&self) -> Option<(YinProgressStage, f32)> {
        self.state.lock().ok().and_then(|s| *s)
    }
}

impl ProgressSource for SaveToastSource {
    fn title(&self) -> String {
        "正在保存".to_string()
    }
    fn message(&self) -> String {
        self.current()
            .map(|(stage, _)| stage_label(stage))
            .unwrap_or_else(|| "准备中…".to_string())
    }
    fn fraction(&self) -> f32 {
        self.current().map(|(_, f)| f).unwrap_or(0.0)
    }
    fn detail(&self) -> String {
        // 与 message 去重：第二行显示阶段内百分比
        self.current()
            .map(|(_, f)| format!("{:.0}%", f.clamp(0.0, 1.0) * 100.0))
            .unwrap_or_else(|| "0%".to_string())
    }
    fn cancel(&self) -> Option<std::sync::Arc<std::sync::atomic::AtomicBool>> {
        None
    }
}
