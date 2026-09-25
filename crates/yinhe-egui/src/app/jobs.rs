//! 异步任务中心：保存 / 加载 / 导出 / 重采样 / 自动保存的运行时状态聚合。
//!
//! 从 `App` 聚合而来，减少 God Object 字段数；行为与拆分前完全一致。
//! 任务的具体轮询逻辑仍在各自的 `app/*.rs` 中（`poll.rs`、`autosave.rs`、
//! `rescale_state.rs`），本结构体只持有状态并提供查询方法。

use std::sync::mpsc;

use yinhe_editor_core::progress::SharedProgress;

use crate::app::autosave::AutoSaveState;
use crate::app::export_state::ExportState;
use crate::app::rescale_state::RescaleState;
use crate::dialogs::save_overlay::SharedSaveProgress;

/// 异步保存结果通道：`(目标文档 idx, 发起时撤销栈长度, 路径, 结果)`。
pub(crate) type SaveResultRx = mpsc::Receiver<(usize, usize, String, Result<(), String>)>;

/// 所有异步任务的运行时状态。
pub(crate) struct JobCenter {
    // ── 异步保存 ──
    /// 异步保存结果：`Some` 表示保存进行中。
    pub save_rx: Option<SaveResultRx>,
    /// 保存进度（阶段 + 阶段内 0.0~1.0），由保存线程经 channel 推送、
    /// poll 写入共享状态，toast 渲染时 pull 读取。
    pub save_progress: SharedSaveProgress,
    /// 保存线程进度推送端。
    pub save_progress_rx: Option<mpsc::Receiver<yinhe_yin::YinProgress>>,

    // ── 多阶段加载进度（文件 / 音频 / 插件共用）──
    pub load_progress: SharedProgress,

    // ── 自动保存（定时备份 + 崩溃恢复）──
    pub autosave: AutoSaveState,

    // ── 异步音频导出 ──
    pub export: ExportState,

    // ── 异步 PPQ 重采样 ──
    pub rescale: RescaleState,
}

impl JobCenter {
    pub fn new() -> Self {
        Self {
            save_rx: None,
            save_progress: Default::default(),
            save_progress_rx: None,
            load_progress: yinhe_editor_core::progress::new_shared(),
            autosave: AutoSaveState::new(),
            export: ExportState::new(),
            rescale: RescaleState::new(),
        }
    }

    /// 是否有异步保存进行中。
    pub fn is_saving(&self) -> bool {
        self.save_rx.is_some()
    }

    /// 是否有音频导出进行中。
    pub fn is_exporting(&self) -> bool {
        self.export.running
    }

    /// 是否有 PPQ 重采样进行中。
    pub fn is_rescaling(&self) -> bool {
        self.rescale.is_running()
    }
}
