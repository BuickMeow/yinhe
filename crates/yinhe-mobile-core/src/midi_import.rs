//! 移动端「导入 MIDI」：后台解析 → 摘要 → 合并到当前文档末尾。
//!
//! 解析状态由本模块持有（Dart 轮询 JSON）；真正的合并走 [`crate::doc_job`]，
//! 因为 1 亿音符的插入/统计重建不能在 Dart UI isolate 上同步跑。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use yinhe_core::YinModel;
use yinhe_editor_core::midi_import::{ImportChannelMode, MidiImportOptions, MidiImportSummary};

struct State {
    token: u64,
    /// 0=空闲 1=解析中 2=可导入 3=失败
    state: u32,
    progress: f32,
    error: String,
    path: String,
    summary: String,
    model: Option<Box<YinModel>>,
}

fn slot() -> &'static Mutex<State> {
    static SLOT: OnceLock<Mutex<State>> = OnceLock::new();
    SLOT.get_or_init(|| {
        Mutex::new(State {
            token: 0,
            state: 0,
            progress: 0.0,
            error: String::new(),
            path: String::new(),
            summary: "{}".into(),
            model: None,
        })
    })
}

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

fn json_escape(s: &str) -> String {
    crate::scene::json_escape(s)
}

/// 开始解析一个 MIDI 文件；返回 1 = 已开始，0 = 正在解析/线程失败。
pub fn start(path: String) -> i32 {
    let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
    {
        let Ok(mut st) = slot().lock() else {
            return 0;
        };
        if st.state == 1 {
            return 0;
        }
        st.token = token;
        st.state = 1;
        st.progress = 0.0;
        st.error.clear();
        st.summary = "{}".into();
        st.path = path.clone();
        st.model = None;
    }

    let spawn = std::thread::Builder::new()
        .name("yinhe-midi-import".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let result = yinhe_midi::parse_file_with_encoding(
                &path,
                yinhe_midi::MidiImportEncoding::Utf8,
                |p| {
                    let frac = if p.total_tracks > 0 {
                        p.current_track as f32 / p.total_tracks as f32
                    } else {
                        0.0
                    };
                    if let Ok(mut st) = slot().lock()
                        && st.token == token
                    {
                        st.progress = (0.02 + frac * 0.93).clamp(0.0, 1.0);
                    }
                },
            );
            match result {
                Ok(model) => {
                    let summary = MidiImportSummary::from_model(&model).to_json();
                    if let Ok(mut st) = slot().lock()
                        && st.token == token
                    {
                        st.model = Some(Box::new(model));
                        st.summary = summary;
                        st.progress = 1.0;
                        st.state = 2;
                    }
                }
                Err(e) => {
                    if let Ok(mut st) = slot().lock()
                        && st.token == token
                    {
                        st.state = 3;
                        st.error = format!("解析失败: {e}");
                    }
                }
            }
        });
    if spawn.is_err() {
        if let Ok(mut st) = slot().lock() {
            st.state = 0;
            st.error = "解析线程创建失败".into();
        }
        return 0;
    }
    1
}

/// 解析状态 JSON：`{"state","progress","error","summary"}`。
pub fn status_json() -> String {
    let Ok(st) = slot().lock() else {
        return "{\"state\":0,\"progress\":0,\"error\":\"\",\"summary\":{}}".into();
    };
    format!(
        "{{\"state\":{},\"progress\":{:.3},\"error\":\"{}\",\"summary\":{}}}",
        st.state,
        st.progress.clamp(0.0, 1.0),
        json_escape(&st.error),
        if st.summary.is_empty() {
            "{}"
        } else {
            &st.summary
        }
    )
}

/// 取消：丢弃已解析结果；正在解析的线程会在完成时发现 token 变化而丢弃结果。
pub fn cancel() -> i32 {
    if let Ok(mut st) = slot().lock() {
        st.state = 0;
        st.progress = 0.0;
        st.error.clear();
        st.summary = "{}".into();
        st.model = None;
        st.token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
    }
    1
}

/// 应用导入：`include_json` 是每源轨的 bool 数组，`channel_mode` 0=保留源通道 1=分配空闲。
pub fn apply(include_json: &str, channel_mode: i32) -> i32 {
    let (model, path) = {
        let Ok(mut st) = slot().lock() else {
            return 0;
        };
        if st.state != 2 {
            return 0;
        }
        let Some(model) = st.model.take() else {
            return 0;
        };
        let path = st.path.clone();
        st.state = 0;
        st.progress = 0.0;
        st.summary = "{}".into();
        (model, path)
    };
    let include: Vec<bool> = serde_json::from_str(include_json).unwrap_or_default();
    let opts = MidiImportOptions {
        include,
        channel_mode: if channel_mode == 1 {
            ImportChannelMode::AllocateFree
        } else {
            ImportChannelMode::KeepSource
        },
    };
    let source = *model;
    let started = crate::doc_job::try_start(
        "Import MIDI",
        crate::doc_job::Reload::Model,
        move |doc, handle| {
            doc.import_midi_with_progress(source, &opts, |p| {
                handle.set_progress(p, "导入中");
            })
            .is_some()
        },
        None::<fn()>,
    );
    if started == 0 {
        log::warn!("[midi-import] 启动失败（无文档或已有任务）: {path}");
    }
    started
}

/// 让 AR 视口定位到导入起点（Dart 在工作完成后调用）。
pub fn focus_selection() {
    let tick = {
        let Ok(s) = crate::scene::scene().lock() else {
            return;
        };
        let Some(doc) = s.document.as_ref() else {
            return;
        };
        doc.edit
            .arr_sel_rect
            .first()
            .map(|r| r.0)
            .or(doc.edit.cursor_tick)
    };
    let Some(tick) = tick else {
        return;
    };
    crate::scene::focus_arr_tick(tick);
}
