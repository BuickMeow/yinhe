//! 音色库压缩包：列出压缩包内 SF2/SF3/SFZ，整包解压到私有目录并导入。
//!
//! SFZ 的采样是相对路径，单独解压 `.sfz` 会找不到采样，因此 SFZ 走整包解压。
//! 解压根目录记录在 `soundfonts/manifest.json`，删除音色库时按根目录清理。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{Value, json};
use yinhe_archive::{Archive, ArchiveEntry};

/// 0=打开中 1=就绪 2=需要密码 3=失败
struct Session {
    path: String,
    archive: Option<Archive>,
    entries: Vec<ArchiveEntry>,
    status: u8,
    error: String,
}

static SESSION: OnceLock<Mutex<Option<Session>>> = OnceLock::new();

fn session_slot() -> &'static Mutex<Option<Session>> {
    SESSION.get_or_init(|| Mutex::new(None))
}

/// 0=空闲 1=解压中 2=完成 3=失败
struct ImportState {
    state: u8,
    progress: f32,
    error: String,
}

static IMPORT: OnceLock<Mutex<ImportState>> = OnceLock::new();

fn import_slot() -> &'static Mutex<ImportState> {
    IMPORT.get_or_init(|| {
        Mutex::new(ImportState {
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

/// 打开音色库压缩包（后台线程，只列 SF2/SF3/SFZ）。返回 1 = 已开始。
pub fn open(path: String, password: Option<String>) -> i32 {
    if let Ok(mut slot) = session_slot().lock() {
        *slot = Some(Session {
            path: path.clone(),
            archive: None,
            entries: Vec::new(),
            status: 0,
            error: String::new(),
        });
    }
    let path_for_thread = path.clone();
    let spawn = std::thread::Builder::new()
        .name("yinhe-sf-archive-open".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let result = match password.as_deref() {
                Some(pw) => Archive::open_with_password(&path_for_thread, Some(pw)),
                None => Archive::open(&path_for_thread),
            };
            let Ok(mut slot) = session_slot().lock() else {
                return;
            };
            let Some(session) = slot.as_mut() else {
                return;
            };
            match result {
                Ok(archive) => {
                    session.entries = archive.list_files(&["sf2", "sf3", "sfz"]);
                    session.archive = Some(archive);
                    session.status = 1;
                    log::info!(
                        "音色库压缩包已打开: {path_for_thread}（{} 个候选）",
                        session.entries.len()
                    );
                }
                Err(yinhe_archive::ArchiveError::PasswordRequired) => {
                    session.status = 2;
                    log::info!("音色库压缩包需要密码: {path_for_thread}");
                }
                Err(e) => {
                    session.error = e.to_string();
                    session.status = 3;
                    log::error!("音色库压缩包打开失败（{path_for_thread}）: {e}");
                }
            }
        });
    if let Err(e) = spawn {
        log::error!("音色库压缩包线程创建失败: {e}");
        return 0;
    }
    1
}

/// 压缩包状态：0=无 1=打开中 2=就绪 3=需要密码 4=失败。
pub fn status() -> i32 {
    match session_slot()
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|s| s.status))
    {
        None => 0,
        Some(0) => 1,
        Some(1) => 2,
        Some(2) => 3,
        Some(_) => 4,
    }
}

/// `{"path":..,"error":..,"entries":[{"name":..,"size":..}]}`
pub fn json() -> String {
    let Ok(slot) = session_slot().lock() else {
        return "{}".into();
    };
    let Some(session) = slot.as_ref() else {
        return "{}".into();
    };
    let mut out = String::with_capacity(64 + session.entries.len() * 48);
    out.push_str("{\"path\":\"");
    out.push_str(&json_escape(&session.path));
    out.push_str("\",\"error\":\"");
    out.push_str(&json_escape(&session.error));
    out.push_str("\",\"entries\":[");
    for (i, e) in session.entries.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"name\":\"{}\",\"size\":{}}}",
            json_escape(&e.name),
            e.size
        ));
    }
    out.push_str("]}");
    out
}

/// 导入第 `index` 个候选（后台解压）。返回 1 = 已开始。
pub fn import(index: usize) -> i32 {
    let (archive, entry) = {
        let Ok(slot) = session_slot().lock() else {
            return 0;
        };
        let Some(session) = slot.as_ref() else {
            return 0;
        };
        if session.status != 1 {
            return 0;
        }
        let Some(entry) = session.entries.get(index).cloned() else {
            return 0;
        };
        let Some(archive) = session.archive.clone() else {
            return 0;
        };
        (archive, entry)
    };
    {
        let Ok(mut st) = import_slot().lock() else {
            return 0;
        };
        if st.state == 1 {
            return 0;
        }
        st.state = 1;
        st.progress = 0.0;
        st.error.clear();
    }
    let spawn = std::thread::Builder::new()
        .name("yinhe-sf-archive-import".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            let result = import_entry(&archive, &entry);
            let Ok(mut st) = import_slot().lock() else {
                return;
            };
            match result {
                Ok(()) => {
                    st.state = 2;
                    st.progress = 1.0;
                    log::info!("音色库导入完成: {}", entry.name);
                }
                Err(e) => {
                    st.state = 3;
                    st.error = e;
                    log::error!("音色库导入失败（{}）: {}", entry.name, st.error);
                }
            }
        });
    if let Err(e) = spawn {
        if let Ok(mut st) = import_slot().lock() {
            st.state = 3;
            st.error = format!("线程创建失败: {e}");
        }
        return 0;
    }
    1
}

/// `{"state":1,"progress":0.42,"error":""}`
pub fn import_json() -> String {
    let Ok(st) = import_slot().lock() else {
        return "{\"state\":0,\"progress\":0,\"error\":\"\"}".into();
    };
    format!(
        "{{\"state\":{},\"progress\":{:.4},\"error\":\"{}\"}}",
        st.state,
        st.progress.clamp(0.0, 1.0),
        json_escape(&st.error)
    )
}

fn import_entry(archive: &Archive, entry: &ArchiveEntry) -> Result<(), String> {
    let dir = crate::scene::files_dir()
        .ok_or_else(|| "私有目录未初始化".to_string())
        .map(|d| Path::new(&d).join("soundfonts"))?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let root = unique_root(&dir, archive.path());
    let lower = entry.name.to_ascii_lowercase();
    let is_sfz = lower.ends_with(".sfz");
    let result = if is_sfz {
        archive
            .extract_all(&root, |done, total| {
                if let Ok(mut st) = import_slot().lock() {
                    st.progress = if total > 0 {
                        done as f32 / total as f32
                    } else {
                        0.5
                    };
                }
            })
            .map_err(|e| e.to_string())?;
        archive
            .extract_entry(&entry.name, &root)
            .map_err(|e| e.to_string())?
    } else {
        archive
            .extract_entry(&entry.name, &root)
            .map_err(|e| e.to_string())?
    };
    let path = result.to_string_lossy().into_owned();
    crate::audio::add_soundfont(path.clone());
    crate::scene::save_soundfont_settings();
    add_manifest(&dir, &root, &path);
    Ok(())
}

fn unique_root(dir: &Path, archive: &Path) -> PathBuf {
    let base = archive
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("soundfont")
        .replace(
            |c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "_",
        );
    let base = if base.is_empty() {
        "soundfont".to_string()
    } else {
        base
    };
    let mut root = dir.join(&base);
    let mut n = 2;
    while root.exists() {
        root = dir.join(format!("{base}_{n}"));
        n += 1;
    }
    root
}

fn manifest_path(dir: &Path) -> PathBuf {
    dir.join("manifest.json")
}

fn load_manifest(dir: &Path) -> Vec<(String, Vec<String>)> {
    let Ok(text) = std::fs::read_to_string(manifest_path(dir)) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(imports) = v.get("imports").and_then(Value::as_array) {
        for it in imports {
            let Some(root) = it.get("root").and_then(Value::as_str) else {
                continue;
            };
            let soundfonts = it
                .get("soundfonts")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            out.push((root.to_string(), soundfonts));
        }
    }
    out
}

fn save_manifest(dir: &Path, imports: &[(String, Vec<String>)]) {
    let items: Vec<Value> = imports
        .iter()
        .map(|(root, soundfonts)| json!({"root": root, "soundfonts": soundfonts}))
        .collect();
    let v = json!({"imports": items});
    let _ = std::fs::create_dir_all(dir);
    if let Err(e) = std::fs::write(manifest_path(dir), v.to_string()) {
        log::error!("音色库 manifest 写入失败: {e}");
    }
}

fn add_manifest(dir: &Path, root: &Path, soundfont: &str) {
    let mut imports = load_manifest(dir);
    let root_s = root.to_string_lossy().into_owned();
    if let Some((_, sfs)) = imports.iter_mut().find(|(r, _)| *r == root_s) {
        if !sfs.iter().any(|p| p == soundfont) {
            sfs.push(soundfont.to_string());
        }
    } else {
        imports.push((root_s, vec![soundfont.to_string()]));
    }
    save_manifest(dir, &imports);
}

/// 清理：删除已从音色库表移除的导入根目录/孤儿目录，并更新 manifest。
///
/// 停用（仍留在表中）的音色库保留文件，重新启用可立即使用。
pub fn cleanup() {
    let Some(dir) = crate::scene::files_dir().map(|d| Path::new(&d).join("soundfonts")) else {
        return;
    };
    let current: HashSet<String> = crate::audio::soundfonts()
        .into_iter()
        .map(|e| e.path)
        .collect();
    let mut kept: Vec<(String, Vec<String>)> = Vec::new();
    for (root, sfs) in load_manifest(&dir) {
        let remaining: Vec<String> = sfs.into_iter().filter(|p| current.contains(p)).collect();
        if remaining.is_empty() {
            if let Err(e) = std::fs::remove_dir_all(&root)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                log::warn!("清理音色库目录失败（{root}）: {e}");
            }
        } else {
            kept.push((root, remaining));
        }
    }
    // 孤儿目录：不在当前音色库表任何路径前缀下（旧文件夹导入/中断导入）。
    if let Ok(read) = std::fs::read_dir(&dir) {
        for entry in read.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let used = current.iter().any(|p| Path::new(p).starts_with(&path));
            if !used && let Err(e) = std::fs::remove_dir_all(&path) {
                log::warn!("清理孤儿音色库目录失败（{}）: {e}", path.display());
            }
        }
    }
    save_manifest(&dir, &kept);
}
