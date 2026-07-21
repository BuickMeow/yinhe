//! Rescale state subsystem — async PPQ rescale with progress reporting.
//!
//! 当用户修改项目 PPQ 并选择"缩放音符"时，主线程 clone 当前 model
//! 并 spawn 子线程执行 [`yinhe_core::YinModel::rescale_ppq_with_progress`]。
//! 主线程通过 `progress` 字段实时显示进度，通过 `cancel` 字段支持取消。
//!
//! 完成后 [`crate::app::poll`] 检测到结果，把新 model 替换回 doc 并
//! 调用 `commit_ppq(rescale=true)` 推 undo。

use std::sync::{mpsc, Arc, Mutex};

use eframe::egui;

use yinhe_core::{RescaleProgress, YinModel};
use yinhe_editor_core::history::commit_ppq;

use crate::app::App;

/// egui memory 中暂存 rescale 请求的 Id 常量。
///
/// `project_info.rs` 弹框确认"是（缩放音符）"后写入此 Id，
/// `main_loop.rs` 每帧检测此 Id 并启动异步线程。
pub(crate) const RESCALE_REQUEST_ID: &str = "ppq_rescale_request";

/// 从 `project_info.rs` 传给 `main_loop.rs` 的 rescale 请求。
#[derive(Clone, Copy)]
pub(crate) struct RescaleRequest {
    pub old_ppq: u32,
    pub new_ppq: u32,
    pub dragvalue_id: u64,
}

/// 所有 PPQ rescale 相关状态，从 `App` 拆出避免 God Object。
///
/// 参考 [`crate::app::export_state::ExportState`] 的模式：
/// `rx` + `progress` + `cancel` 三件套。
pub(crate) struct RescaleState {
    /// 异步 rescale 结果接收端。`Some` 表示 rescale 正在进行中。
    pub rx: Option<mpsc::Receiver<Result<YinModel, String>>>,
    /// 子线程实时更新的进度（0.0..1.0 + label）。
    pub progress: Arc<Mutex<RescaleProgress>>,
    /// 取消标志。设为 true 后子线程在下个 bucket 开始前退出。
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
    /// rescale 完成后 commit undo 所需的上下文：
    /// `(old_ppq, new_ppq, dragvalue_id, doc_idx)`
    pub pending: Option<(u32, u32, u64, usize)>,
}

impl RescaleState {
    pub fn new() -> Self {
        Self {
            rx: None,
            progress: Arc::new(Mutex::new(RescaleProgress::default())),
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            pending: None,
        }
    }

    /// 是否有 rescale 正在进行。
    pub fn is_running(&self) -> bool {
        self.rx.is_some()
    }
}

impl App {
    /// 检测 egui memory 中是否有 rescale 请求，若有则启动异步线程。
    ///
    /// 由 `main_loop` 每帧调用（在 `poll_async_operations` 之前）。
    /// 请求由 `project_info.rs` 弹框确认"是（缩放音符）"后写入。
    pub(in crate::app) fn start_rescale_if_requested(&mut self, ctx: &egui::Context) {
        // 已有 rescale 在跑：拒绝新请求（防重入）。
        if self.rescale.is_running() {
            return;
        }
        let req: Option<RescaleRequest> = ctx.data(|d| d.get_temp(egui::Id::new(RESCALE_REQUEST_ID)));
        let Some(req) = req else { return };
        // 取出请求后立即清除，避免下帧重复启动。
        ctx.data_mut(|d| d.remove::<RescaleRequest>(egui::Id::new(RESCALE_REQUEST_ID)));

        let Some(doc_idx) = self.active_doc else { return };
        let Some(doc) = self.documents.get(doc_idx) else { return };

        // clone model（Arc clone，廉价；子线程内部 Arc::make_mut 才深拷贝）。
        let model = (*doc.data.model).clone();
        let new_ppq = req.new_ppq;

        // 重置 progress + cancel。
        if let Ok(mut p) = self.rescale.progress.lock() {
            *p = RescaleProgress::default();
        }
        self.rescale.cancel.store(false, std::sync::atomic::Ordering::Relaxed);

        let progress = self.rescale.progress.clone();
        let cancel = self.rescale.cancel.clone();
        let (tx, rx) = mpsc::channel();

        std::thread::spawn(move || {
            let result = YinModel::rescale_ppq_with_progress(model, new_ppq, progress, cancel);
            // tx send 失败说明主线程已关闭 rx，忽略即可。
            let _ = tx.send(result);
        });

        self.rescale.rx = Some(rx);
        self.rescale.pending = Some((req.old_ppq, req.new_ppq, req.dragvalue_id, doc_idx));
    }

    /// 检测异步 rescale 是否完成，若完成则把新 model 写回 doc 并 commit undo。
    ///
    /// 由 `poll_async_operations` 调用。
    pub(in crate::app) fn poll_rescale_completion(&mut self) {
        let Some(rx) = &self.rescale.rx else { return };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                // 子线程 panic 或异常退出：清理状态，还原 meta.ppq。
                self.rescale.rx = None;
                let pending = self.rescale.pending.take();
                if let Some((old_ppq, _new_ppq, _id, doc_idx)) = pending {
                    if let Some(doc) = self.documents.get_mut(doc_idx) {
                        let model = std::sync::Arc::make_mut(&mut doc.data.model);
                        model.meta.ppq = old_ppq;
                    }
                }
                self.load_error = Some("PPQ 缩放线程异常退出".to_string());
                return;
            }
        };
        // 收到结果：清理 rx。
        self.rescale.rx = None;
        let (old_ppq, new_ppq, dragvalue_id, doc_idx) =
            self.rescale.pending.take().expect("pending must be Some when rx was Some");

        match result {
            Ok(new_model) => {
                if let Some(doc) = self.documents.get_mut(doc_idx) {
                    // 用新 model 替换 doc.data.model。
                    doc.data.model = std::sync::Arc::new(new_model);
                    doc.data.bump_revision();
                    // 推 undo（带 rescale 标志）。
                    commit_ppq(
                        &mut doc.history,
                        &mut doc.edit.pending_edits,
                        dragvalue_id,
                        new_ppq,
                        true, // rescale
                        doc.edit.selected.clone(),
                        doc.edit.track_selected.clone(),
                        doc.edit.sel_rect.clone(),
                    );
                }
            }
            Err(msg) => {
                // 用户取消或子线程报错：还原 meta.ppq = old_ppq。
                if let Some(doc) = self.documents.get_mut(doc_idx) {
                    let model = std::sync::Arc::make_mut(&mut doc.data.model);
                    model.meta.ppq = old_ppq;
                }
                if msg != "已取消" {
                    self.load_error = Some(msg);
                }
            }
        }
    }
}


