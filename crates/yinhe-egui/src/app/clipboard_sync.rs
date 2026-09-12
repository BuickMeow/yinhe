//! 系统剪贴板同步（跨 yinhe 实例剪贴板）。
//!
//! 复制：内部保留 O(1) 快照，同时后台把数据流式写入临时文件；写完后
//! 把引用文本 `yinhe-clip/1/<id>` 写入系统剪贴板。其他 yinhe 实例读
//! 到引用后从同一临时文件加载数据。
//!
//! 粘贴：引用属于本实例（`active_id`）就用内部剪贴板；属于其他实例
//! 则加载文件；外部非本格式内容使内部剪贴板失效（与成熟软件一致：
//! 系统剪贴板是剪贴板内容的唯一真相）。

use std::sync::mpsc;

use yinhe_editor_core::ClipboardContent;
use yinhe_editor_core::clipboard_file;

use crate::app::App;

/// 后台导出完成事件。
struct ClipExportDone {
    id: String,
    error: Option<String>,
}

/// 需要 App 层提示/处理的同步事件。
pub(crate) enum ClipboardSyncEvent {
    /// 数据已写入系统剪贴板（跨实例可用）。
    Exported,
    /// 导出失败（toast 提示用）。
    ExportFailed(String),
}

/// 系统剪贴板桥。
pub(crate) struct ClipboardSync {
    /// 系统剪贴板句柄；创建失败（无剪贴板环境）时降级为纯内部剪贴板。
    clipboard: Option<arboard::Clipboard>,
    tx: mpsc::Sender<ClipExportDone>,
    rx: mpsc::Receiver<ClipExportDone>,
    /// 正在后台导出的 id（最新一次复制）。
    pending_id: Option<String>,
    /// 内部剪贴板对应的系统引用 id（自己写的或已加载的外部数据）。
    active_id: Option<String>,
    /// 内部剪贴板比系统剪贴板新（复制后导出尚未成功/已失败）。
    local_newer: bool,
}

impl ClipboardSync {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            clipboard: arboard::Clipboard::new().ok(),
            tx,
            rx,
            pending_id: None,
            active_id: None,
            local_newer: false,
        }
    }

    /// 复制后启动后台导出：把剪贴板数据流式写入临时文件。
    pub fn start_export(&mut self, content: ClipboardContent) {
        if content.is_empty() {
            return;
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        self.pending_id = Some(id.clone());
        self.local_newer = true;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let path = clipboard_file::clip_file_path(&id);
            let result = match &content {
                ClipboardContent::Notes(cb) => clipboard_file::write_notes(&path, cb),
                ClipboardContent::Automation(cb) => clipboard_file::write_automation(&path, cb),
                ClipboardContent::Empty => Ok(()),
            };
            let _ = tx.send(ClipExportDone {
                id,
                error: result.err().map(|e| e.to_string()),
            });
        });
    }

    /// 每帧轮询后台导出完成：主线程把引用文本写入系统剪贴板。
    pub fn poll(&mut self) -> Option<ClipboardSyncEvent> {
        let mut event = None;
        while let Ok(done) = self.rx.try_recv() {
            // 已被更新的复制取代的旧导出：丢弃，不覆盖系统剪贴板。
            if self.pending_id.as_deref() != Some(done.id.as_str()) {
                continue;
            }
            self.pending_id = None;
            match done.error {
                None => {
                    let written = self
                        .clipboard
                        .as_mut()
                        .is_some_and(|cb| cb.set_text(clipboard_file::clip_ref(&done.id)).is_ok());
                    if written {
                        self.active_id = Some(done.id);
                        self.local_newer = false;
                        event = Some(ClipboardSyncEvent::Exported);
                    } else {
                        event = Some(ClipboardSyncEvent::ExportFailed(
                            "无法写入系统剪贴板".to_string(),
                        ));
                    }
                }
                Some(err) => event = Some(ClipboardSyncEvent::ExportFailed(err)),
            }
        }
        event
    }

    /// 粘贴前解析系统剪贴板；必要时把 `content` 更新为其他实例的数据。
    /// 返回 false 表示内部剪贴板已失效（外部内容或不兼容）。
    pub fn resolve_paste(&mut self, content: &mut ClipboardContent) -> bool {
        // 后台导出未完成：内部快照就是最新内容。
        if self.pending_id.is_some() {
            return true;
        }
        let Some(system) = self.clipboard.as_mut() else {
            return true; // 无系统剪贴板：退回纯内部
        };
        let Ok(text) = system.get_text() else {
            return true; // 读失败/为空：不破坏内部剪贴板
        };
        let Some(id) = clipboard_file::parse_clip_ref(&text) else {
            // 外部内容：内部剪贴板失效。除非本地复制尚未成功同步
            // （导出失败等），此时内部才是权威，保留它。
            if self.local_newer {
                return true;
            }
            *content = ClipboardContent::Empty;
            self.active_id = None;
            return false;
        };
        self.local_newer = false;
        if self.active_id.as_deref() == Some(id) {
            return !content.is_empty();
        }
        // 其他实例的引用：从临时文件加载。
        match clipboard_file::read(&clipboard_file::clip_file_path(id)) {
            Ok(loaded) => {
                *content = loaded;
                self.active_id = Some(id.to_string());
                true
            }
            Err(_) => {
                *content = ClipboardContent::Empty;
                self.active_id = None;
                false
            }
        }
    }
}

impl App {
    /// 每帧轮询系统剪贴板导出（在 `poll_async_operations` 之后调用）。
    pub(crate) fn poll_clipboard_sync(&mut self) {
        if let Some(event) = self.clipboard_sync.poll() {
            match event {
                ClipboardSyncEvent::Exported => {}
                ClipboardSyncEvent::ExportFailed(err) => {
                    self.notifications.error("跨实例剪贴板写入失败", err);
                }
            }
        }
    }

    /// 复制成功后启动跨实例导出（内部剪贴板已就绪，后台只是补系统剪贴板）。
    pub(crate) fn export_clipboard_to_system(&mut self) {
        self.clipboard_sync.start_export(self.clipboard.clone());
    }
}
