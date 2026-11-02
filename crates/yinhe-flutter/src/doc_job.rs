//! 文档变更后台任务：批量编辑 / 删除 / 人性化 / 粘贴 / 撤销重做 / 工程 PPQ。
//!
//! 这些操作全是 O(选中音符) 且会写 undo 栈，不能在 Dart UI isolate 上同步跑。
//! 做法：把 `Document` 从场景里**取走**交给后台线程，期间 `s.model` 快照仍供
//! 渲染（模态进度框盖住画布）；完成后把文档放回、刷新快照并重载音频。
//!
//! Dart 侧通过 `doc_job_status_json()` 轮询进度；PPQ 任务复用
//! `rescale_ppq_with_progress` 的进度/取消。其余任务不可取消（进度只有
//! “运行中/完成”两态）。

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use yinhe_core::RescaleProgress;
use yinhe_editor_core::document::Document;

/// 任务完成后如何重载音频。
#[derive(Clone, Copy)]
pub enum Reload {
    /// 仅音符/自动化变化（保留播放位置与通道）。
    Notes,
    /// 工程级变化（PPQ/通道）：整模型重载。
    Model,
}

struct Shared {
    state: AtomicU32,
    progress: Arc<Mutex<RescaleProgress>>,
    cancel: Arc<AtomicBool>,
    error: Mutex<String>,
}

/// 任务内可用的进度/取消句柄。
pub struct JobHandle {
    shared: Arc<Shared>,
}

impl JobHandle {
    /// PPQ 缩放用的进度结构（`rescale_ppq_with_progress` 直接写它）。
    pub fn progress(&self) -> Arc<Mutex<RescaleProgress>> {
        Arc::clone(&self.shared.progress)
    }

    /// PPQ 缩放用的取消标志。
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.shared.cancel)
    }

    /// 非 PPQ 任务的粗粒度进度（0..1）。
    pub fn set_progress(&self, progress: f32, label: &'static str) {
        if let Ok(mut p) = self.shared.progress.lock() {
            p.progress = progress.clamp(0.0, 1.0);
            p.label = label.to_string();
        }
    }
}

fn slot() -> &'static Mutex<Option<Arc<Shared>>> {
    static SLOT: OnceLock<Mutex<Option<Arc<Shared>>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// 启动一个文档任务。返回 1 = 已启动；0 = 无文档/已有任务/线程创建失败。
///
/// `after` 在文档放回并刷新后执行（此时不持场景锁），用于粘贴后设置光标。
pub fn try_start<F, A>(label: &'static str, reload: Reload, run: F, after: Option<A>) -> i32
where
    F: FnOnce(&mut Document, &JobHandle) -> bool + Send + 'static,
    A: FnOnce() + Send + 'static,
{
    {
        let Ok(guard) = slot().lock() else {
            return 0;
        };
        if guard
            .as_ref()
            .is_some_and(|shared| shared.state.load(Ordering::Relaxed) == 1)
        {
            return 0;
        }
    }
    let Ok(mut scene) = crate::scene::scene().lock() else {
        return 0;
    };
    let Some(doc) = scene.document.take() else {
        return 0;
    };
    drop(scene);
    log::info!("文档任务开始: {label}");

    let shared = Arc::new(Shared {
        state: AtomicU32::new(1),
        progress: Arc::new(Mutex::new(RescaleProgress::default())),
        cancel: Arc::new(AtomicBool::new(false)),
        error: Mutex::new(String::new()),
    });
    if let Ok(mut guard) = slot().lock() {
        *guard = Some(Arc::clone(&shared));
    }

    let handle = JobHandle {
        shared: Arc::clone(&shared),
    };
    // 先起线程再投递文档：线程创建失败时必须把文档还回场景，不能把它丢了。
    let (tx, rx) = std::sync::mpsc::channel::<Document>();
    let worker = std::thread::Builder::new()
        .name("yinhe-doc-job".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let Ok(mut doc) = rx.recv() else {
                return;
            };
            let outcome = catch_unwind(AssertUnwindSafe(|| run(&mut doc, &handle)));
            let panicked = match outcome {
                Ok(_) => false,
                Err(_) => {
                    if let Ok(mut err) = shared.error.lock() {
                        *err = "内部错误".into();
                    }
                    true
                }
            };
            let model = doc.data.model.clone();
            if let Ok(mut scene) = crate::scene::scene().lock() {
                scene.document = Some(doc);
                crate::scene::refresh_snapshot(&mut scene);
            }
            match reload {
                Reload::Notes => crate::audio::reload_notes(model),
                Reload::Model => crate::audio::on_model_loaded(model),
            }
            // 文档已放回、音频已重载，Dart 关掉进度框后立即能读到新状态。
            shared
                .state
                .store(if panicked { 3 } else { 2 }, Ordering::Relaxed);
            if let Some(after) = after {
                after();
            }
        });
    if worker.is_err() {
        if let Ok(mut scene) = crate::scene::scene().lock() {
            scene.document = Some(doc);
        }
        if let Ok(mut guard) = slot().lock() {
            *guard = None;
        }
        return 0;
    }
    if tx.send(doc).is_err() {
        // 线程已退出（极罕见）：文档随通道丢弃，无法恢复。
        return 0;
    }
    1
}

/// Dart 轮询用状态 JSON。`state`: 0=无任务 1=运行中 2=完成 3=失败。
pub fn status_json() -> String {
    let Ok(guard) = slot().lock() else {
        return "{\"state\":0,\"progress\":0,\"status\":\"\",\"error\":\"\"}".into();
    };
    let Some(shared) = guard.as_ref() else {
        return "{\"state\":0,\"progress\":0,\"status\":\"\",\"error\":\"\"}".into();
    };
    let state = shared.state.load(Ordering::Relaxed);
    let (progress, status) = shared
        .progress
        .lock()
        .map(|p| (p.progress.clamp(0.0, 1.0), p.label.clone()))
        .unwrap_or((0.0, String::new()));
    let error = shared.error.lock().map(|e| e.clone()).unwrap_or_default();
    format!(
        "{{\"state\":{state},\"progress\":{progress:.3},\"status\":\"{}\",\"error\":\"{}\"}}",
        crate::scene::json_escape(&status),
        crate::scene::json_escape(&error),
    )
}

/// 取消当前任务（目前只有 PPQ 缩放会在安全点响应）。
pub fn cancel() -> i32 {
    let Ok(guard) = slot().lock() else {
        return 0;
    };
    let Some(shared) = guard.as_ref() else {
        return 0;
    };
    if shared.state.load(Ordering::Relaxed) != 1 {
        return 0;
    }
    shared.cancel.store(true, Ordering::Relaxed);
    1
}
