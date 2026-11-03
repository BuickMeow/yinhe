//! 移动端自动保存：定时把脏文档备份到 `<files>/autosave/doc_<id>.yin`。
//!
//! - 触发与快照在 UI 线程，只做 Arc 克隆 + 元数据同步；
//! - 序列化/压缩/写盘全部在后台线程；
//! - 手工保存成功、打开其它文件、恢复后都会清掉备份；
//! - 启动时若发现残留备份，Dart 侧会异步把它当工程打开（不阻塞启动）。

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// 自动保存间隔（秒）。
const INTERVAL_SECS: u64 = 300;

struct Session {
    last_run: Option<Instant>,
    in_flight: bool,
    rx: Option<Receiver<Result<(), String>>>,
}

fn session() -> &'static Mutex<Session> {
    static S: OnceLock<Mutex<Session>> = OnceLock::new();
    S.get_or_init(|| {
        Mutex::new(Session {
            // 启动后先等一个完整间隔，避免刚打开工程就写盘。
            last_run: Some(Instant::now()),
            in_flight: false,
            rx: None,
        })
    })
}

fn dir() -> Option<PathBuf> {
    crate::scene::files_dir().map(|d| Path::new(&d).join("autosave"))
}

fn manifest_path() -> Option<PathBuf> {
    dir().map(|d| d.join("manifest.json"))
}

/// 是否为自动保存备份路径。
pub fn is_autosave_path(path: &str) -> bool {
    dir().is_some_and(|d| Path::new(path).starts_with(d))
}

/// 删除全部自动保存文件与 manifest。
pub fn clear() {
    let Some(dir) = dir() else {
        return;
    };
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_file() {
                let _ = std::fs::remove_file(p);
            }
        }
    }
    let _ = std::fs::remove_dir(&dir);
}

fn read_manifest() -> Option<Value> {
    let text = std::fs::read_to_string(manifest_path()?).ok()?;
    serde_json::from_str(&text).ok()
}

/// 启动时检查残留备份：返回 `{"path","name"}` 或 `"{}"`。
pub fn pending_json() -> String {
    let Some(m) = read_manifest() else {
        return "{}".into();
    };
    let Some(file) = m.get("file").and_then(Value::as_str) else {
        return "{}".into();
    };
    if !Path::new(file).exists() {
        return "{}".into();
    }
    json!({
        "path": file,
        "name": m.get("name").and_then(Value::as_str).unwrap_or(""),
    })
    .to_string()
}

/// 备份文件对应的原工程名（恢复时告诉 Dart）。
pub fn name_for_path(path: &str) -> Option<String> {
    let m = read_manifest()?;
    (m.get("file").and_then(Value::as_str) == Some(path))
        .then(|| m.get("name").and_then(Value::as_str).map(str::to_string))
        .flatten()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Dart 每约 1 秒调用一次：回收结果并按间隔触发。
pub fn tick() {
    poll_result();
    trigger();
}

fn poll_result() {
    let msg = {
        let Ok(mut s) = session().lock() else {
            return;
        };
        let Some(rx) = s.rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("自动保存线程异常退出".into())),
        }
    };
    let Some(result) = msg else {
        return;
    };
    if let Ok(mut s) = session().lock() {
        s.rx = None;
        s.in_flight = false;
    }
    if let Err(e) = result {
        log::warn!("[autosave] 失败: {e}");
        return;
    }
    // 若期间用户已经手工保存（文档不再脏），备份没有意义，删掉。
    let dirty = crate::scene::scene()
        .lock()
        .ok()
        .and_then(|s| s.document.as_ref().map(|d| d.is_dirty()))
        .unwrap_or(false);
    if !dirty {
        clear();
    }
}

fn trigger() {
    if !crate::audio::settings().auto_save_enabled {
        return;
    }
    {
        let Ok(s) = session().lock() else {
            return;
        };
        if s.in_flight
            || s.last_run
                .is_some_and(|t| t.elapsed() < Duration::from_secs(INTERVAL_SECS))
        {
            return;
        }
    }
    let snapshot = {
        let Ok(mut s) = crate::scene::scene().lock() else {
            return;
        };
        let Some(doc) = s.document.as_mut() else {
            return;
        };
        if !doc.is_dirty() {
            return;
        }
        // 与手工保存相同的持久化同步（只动元数据，不复制音符）。
        doc.sync_overrides_to_model();
        doc.data.sync_project_file();
        doc.data.sync_mapping_file();
        doc.sync_sf_overrides_to_project_file();
        Some((
            doc.doc_id,
            doc.file_path.clone(),
            doc.file_name.clone(),
            Arc::clone(&doc.data.model),
            doc.data.project_file.clone(),
            doc.data.mapping_file.clone(),
            doc.mixer.clone(),
        ))
    };
    let Some((doc_id, original, name, model, project_file, mapping_file, mixer)) = snapshot else {
        return;
    };
    let Some(dir) = dir() else {
        return;
    };
    let path = dir.join(format!("doc_{doc_id}.yin"));
    let tmp = dir.join(format!("doc_{doc_id}.yin.tmp"));

    {
        let Ok(mut s) = session().lock() else {
            return;
        };
        s.in_flight = true;
        s.last_run = Some(Instant::now());
    }
    let (tx, rx) = mpsc::channel();
    let spawn = std::thread::Builder::new()
        .name("yinhe-autosave".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let result = (|| -> Result<(), String> {
                std::fs::create_dir_all(&dir).map_err(|e| format!("创建目录失败: {e}"))?;
                let path_str = tmp.to_string_lossy().to_string();
                yinhe_yin::save_yin_with_files_progress(
                    &model,
                    &path_str,
                    &project_file,
                    &mapping_file,
                    Some(&mixer),
                    |_| {},
                )
                .map_err(|e| e.to_string())?;
                std::fs::rename(&tmp, &path).map_err(|e| format!("重命名失败: {e}"))?;
                let manifest = json!({
                    "doc_id": doc_id,
                    "file": path.to_string_lossy(),
                    "original": original,
                    "name": name,
                    "saved_at": now_secs(),
                });
                let mp = manifest_path().ok_or("无私有目录")?;
                std::fs::write(&mp, manifest.to_string())
                    .map_err(|e| format!("写 manifest 失败: {e}"))?;
                Ok(())
            })();
            let _ = tx.send(result);
        });
    if spawn.is_err() {
        if let Ok(mut s) = session().lock() {
            s.in_flight = false;
        }
        return;
    }
    if let Ok(mut s) = session().lock() {
        s.rx = Some(rx);
    }
}
