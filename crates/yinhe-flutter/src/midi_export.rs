//! 移动端主页 MIDI 导出：后台线程写文件，Flutter 轮询状态。

use std::sync::{Mutex, OnceLock};

use yinhe_midi::{MidiExportOptions, MidiImportEncoding, write_to_file};

struct Status {
    /// 0=空闲、1=导出中、2=完成、3=失败
    state: u32,
    progress: f32,
    error: String,
}

static STATUS: OnceLock<Mutex<Status>> = OnceLock::new();

fn status_slot() -> &'static Mutex<Status> {
    STATUS.get_or_init(|| {
        Mutex::new(Status {
            state: 0,
            progress: 0.0,
            error: String::new(),
        })
    })
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ")
}

/// 开始导出 MIDI；返回 1 = 已开始，0 = 无工程/正在导出。
pub fn start(path: String, opts_json: &str) -> i32 {
    let Some(model) = crate::scene::model_arc() else {
        return 0;
    };
    let Ok(mut st) = status_slot().lock() else {
        return 0;
    };
    if st.state == 1 {
        return 0;
    }
    st.state = 1;
    st.progress = 0.0;
    st.error.clear();
    drop(st);

    let opts = parse_options(opts_json);
    std::thread::spawn(move || {
        let result = write_to_file(&model, &path, &opts).map_err(|e| e.to_string());
        if let Ok(mut st) = status_slot().lock() {
            match result {
                Ok(()) => {
                    st.state = 2;
                    st.progress = 1.0;
                }
                Err(e) => {
                    st.state = 3;
                    st.error = e;
                }
            }
        }
    });
    1
}

/// 开始导出压缩 MIDI（.7z）；返回 1 = 已开始。
pub fn start_7z(path: String, opts_json: &str) -> i32 {
    let Some(model) = crate::scene::model_arc() else {
        return 0;
    };
    let Ok(mut st) = status_slot().lock() else {
        return 0;
    };
    if st.state == 1 {
        return 0;
    }
    st.state = 1;
    st.progress = 0.0;
    st.error.clear();
    drop(st);

    let opts = parse_options(opts_json);
    std::thread::spawn(move || {
        let result = yinhe_editor_core::midi_7z::export_midi_7z(
            &model,
            std::path::Path::new(&path),
            &opts,
            |f| {
                if let Ok(mut st) = status_slot().lock() {
                    st.progress = f.clamp(0.0, 1.0);
                }
            },
        );
        if let Ok(mut st) = status_slot().lock() {
            match result {
                Ok(()) => {
                    st.state = 2;
                    st.progress = 1.0;
                }
                Err(e) => {
                    st.state = 3;
                    st.error = e;
                }
            }
        }
    });
    1
}

/// `{"state":1,"error":""}`
pub fn status_json() -> String {
    let Ok(st) = status_slot().lock() else {
        return "{\"state\":0,\"error\":\"\"}".into();
    };
    format!(
        "{{\"state\":{},\"progress\":{:.3},\"error\":\"{}\"}}",
        st.state,
        st.progress.clamp(0.0, 1.0),
        json_escape(&st.error)
    )
}

fn parse_options(json: &str) -> MidiExportOptions {
    let defaults = MidiExportOptions::default();
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return defaults;
    };
    let encoding = match v.get("encoding").and_then(|x| x.as_str()) {
        Some("gbk") => MidiImportEncoding::Gbk,
        Some("big5") => MidiImportEncoding::Big5,
        Some("shift_jis") => MidiImportEncoding::ShiftJis,
        _ => defaults.encoding,
    };
    MidiExportOptions {
        encoding,
        rpn_full: v
            .get("rpn_full")
            .and_then(|x| x.as_bool())
            .unwrap_or(defaults.rpn_full),
        curve_interpolate: v
            .get("curve_interpolate")
            .and_then(|x| x.as_bool())
            .unwrap_or(defaults.curve_interpolate),
        curve_density: v
            .get("curve_density")
            .and_then(|x| x.as_u64())
            .map_or(defaults.curve_density, |x| x.clamp(1, 10_000) as u32),
        strip_empty_tracks: v
            .get("strip_empty_tracks")
            .and_then(|x| x.as_bool())
            .unwrap_or(defaults.strip_empty_tracks),
        dedup_overlaps: v
            .get("dedup_overlaps")
            .and_then(|x| x.as_bool())
            .unwrap_or(defaults.dedup_overlaps),
    }
}
