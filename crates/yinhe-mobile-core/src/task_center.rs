//! 统一的移动端长任务视图。
//!
//! 目前各任务自己的状态槽仍然保留（避免一次性重写所有后台线程），
//! 本模块把它们归一化成一份 `{"tasks":[...]}`，Dart 只轮询这一个入口。
//! 后续新增任务应直接注册到这里，不再新增 `*_status_json`。

use serde_json::{Value, json};

/// 聚合当前活动任务。`state`: 0=空闲 1=运行中 2=完成 3=失败。
pub fn tasks_json() -> String {
    let mut tasks: Vec<Value> = Vec::new();
    push_status(&mut tasks, "doc", "文档任务", &crate::doc_job::status_json());
    push_status(
        &mut tasks,
        "midi_import",
        "导入 MIDI",
        &crate::midi_import::status_json(),
    );
    push_status(
        &mut tasks,
        "midi_export",
        "导出 MIDI",
        &crate::midi_export::status_json(),
    );
    push_status(
        &mut tasks,
        "audio_export",
        "导出音频",
        &crate::audio::export_progress_json(),
    );
    push_status(
        &mut tasks,
        "soundfont_archive",
        "音色库压缩包",
        &crate::soundfont_archive::import_json(),
    );

    // 音色库热切换没有 state 字段，用 loading 合成一个任务。
    if let Ok(v) = serde_json::from_str::<Value>(&crate::audio::sf_loading_json())
        && v.get("loading").and_then(Value::as_bool) == Some(true)
    {
        let progress = v.get("progress").and_then(Value::as_f64).unwrap_or(0.0);
        tasks.push(json!({
            "id": "soundfont",
            "title": "加载音色库",
            "detail": "",
            "progress": progress,
            "state": 1,
            "error": "",
        }));
    }

    json!({ "tasks": tasks }).to_string()
}

fn push_status(tasks: &mut Vec<Value>, id: &str, title: &str, raw: &str) {
    let Ok(v) = serde_json::from_str::<Value>(raw) else {
        return;
    };
    let state = v.get("state").and_then(Value::as_u64).unwrap_or(0);
    if state == 0 {
        return;
    }
    let progress = v.get("progress").and_then(Value::as_f64).unwrap_or(0.0);
    let detail = v
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let error = v
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    tasks.push(json!({
        "id": id,
        "title": title,
        "detail": detail,
        "progress": progress.clamp(0.0, 1.0),
        "state": state,
        "error": error,
    }));
}
