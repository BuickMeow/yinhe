use super::*;

struct RecentEntry {
    path: String,
    opened: u64,
}

static RECENT: OnceLock<Mutex<Vec<RecentEntry>>> = OnceLock::new();

static FILES_DIR: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn recent_slot() -> &'static Mutex<Vec<RecentEntry>> {
    RECENT.get_or_init(|| Mutex::new(Vec::new()))
}

pub(super) fn files_dir_slot() -> &'static Mutex<Option<String>> {
    FILES_DIR.get_or_init(|| Mutex::new(None))
}

/// 应用私有目录（音色库压缩包解压/manifest 用）。
pub fn files_dir() -> Option<String> {
    files_dir_slot().lock().ok().and_then(|g| g.clone())
}

/// Kotlin 侧传入私有目录（用于最近文件/设置持久化）。
pub fn set_files_dir(dir: String) {
    if let Ok(mut g) = files_dir_slot().lock() {
        *g = Some(dir);
    }
    load_recent();
    load_settings();
}

static DOCUMENTS_DIR: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn documents_dir_slot() -> &'static Mutex<Option<String>> {
    DOCUMENTS_DIR.get_or_init(|| Mutex::new(None))
}

/// iOS 用户可见 Documents（与私有状态目录分开）。Android 不设置，回退到 files_dir。
pub fn set_documents_dir(dir: String) {
    if let Ok(mut g) = documents_dir_slot().lock() {
        *g = Some(dir);
    }
}

fn documents_dir() -> Option<String> {
    documents_dir_slot()
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .or_else(files_dir)
}

const DOC_PREFIX: &str = "@doc/";

/// 最近文件/音色库持久化：沙箱路径写成相对 Documents 的形式，
/// 重装/容器 UUID 变化后仍能解析到当前沙箱。
pub(super) fn portable_doc_path(path: &str) -> String {
    if let Some(dir) = documents_dir()
        && let Ok(rel) = std::path::Path::new(path).strip_prefix(&dir)
    {
        return format!("{DOC_PREFIX}{}", rel.display());
    }
    // 旧版本存的是绝对路径：截取 `/Documents/` 之后的部分。
    if let Some(idx) = path.find("/Documents/") {
        return format!("{DOC_PREFIX}{}", &path[idx + "/Documents/".len()..]);
    }
    path.to_string()
}

/// 把 `@doc/...` 还原成当前沙箱的绝对路径。
fn resolve_doc_path(path: &str) -> String {
    if let Some(rel) = path.strip_prefix(DOC_PREFIX)
        && let Some(dir) = documents_dir()
    {
        return format!("{dir}/{rel}");
    }
    path.to_string()
}

/// 音色库路径自愈：旧容器/错误目录写下的路径，尽量找回当前 Documents 下的文件。
///
/// 优先保持原路径（存在即用），其次按 `/Documents/` 后缀重定位，
/// 最后按文件名在 Documents 根查找（覆盖早期写成 Application Support 的坏路径）。
pub(super) fn resolve_existing_soundfont(path: &str) -> String {
    let resolved = resolve_doc_path(path);
    if std::path::Path::new(&resolved).exists() {
        return resolved;
    }
    let Some(dir) = documents_dir() else {
        return resolved;
    };
    if let Some(idx) = path.find("/Documents/") {
        let candidate = format!("{dir}/{}", &path[idx + "/Documents/".len()..]);
        if std::path::Path::new(&candidate).exists() {
            return candidate;
        }
    }
    if let Some(name) = std::path::Path::new(path).file_name() {
        let candidate = format!("{dir}/{}", name.to_string_lossy());
        if std::path::Path::new(&candidate).exists() {
            return candidate;
        }
    }
    resolved
}

fn recent_file_path() -> Option<String> {
    files_dir_slot()
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|d| format!("{d}/recent.txt")))
}

fn load_recent() {
    let Some(path) = recent_file_path() else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let mut list = Vec::new();
    for line in text.lines() {
        let mut parts = line.splitn(2, '\t');
        let (Some(p), Some(t)) = (parts.next(), parts.next()) else {
            continue;
        };
        let Ok(opened) = t.trim().parse::<u64>() else {
            continue;
        };
        if !p.is_empty() {
            let abs = resolve_doc_path(p);
            // 文件已不存在（旧容器路径/用户删除）就不要留在列表里，
            // 否则点击会报 `No such file or directory`。
            if std::path::Path::new(&abs).exists() {
                list.push(RecentEntry { path: abs, opened });
            }
        }
    }
    list.truncate(10);
    if let Ok(mut slot) = recent_slot().lock() {
        *slot = list;
    }
}

fn save_recent() {
    let Some(path) = recent_file_path() else {
        return;
    };
    let Ok(list) = recent_slot().lock() else {
        return;
    };
    let mut text = String::new();
    for e in list.iter() {
        text.push_str(&portable_doc_path(&e.path));
        text.push('\t');
        text.push_str(&e.opened.to_string());
        text.push('\n');
    }
    if let Err(e) = std::fs::write(&path, text) {
        log::error!("最近文件写入失败: {e}");
    }
}

/// 记录一次成功打开（去重、置顶、限 10 条、持久化）。
pub(super) fn push_recent(path: &str) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut list) = recent_slot().lock() {
        list.retain(|e| e.path != path);
        list.insert(
            0,
            RecentEntry {
                path: path.to_string(),
                opened: now,
            },
        );
        list.truncate(10);
    }
    save_recent();
}

/// 最近文件 JSON（Dart 解析）：`[{"name","path","time"}]`。
/// 最近打开的本地路径（按最近优先；音乐库扫描用）。
pub fn recent_paths() -> Vec<String> {
    recent_slot()
        .lock()
        .map(|list| list.iter().map(|e| e.path.clone()).collect())
        .unwrap_or_default()
}

pub fn recent_json() -> String {
    let Ok(list) = recent_slot().lock() else {
        return "[]".into();
    };
    let visible: Vec<&RecentEntry> = list
        .iter()
        .filter(|e| std::path::Path::new(&e.path).exists())
        .collect();
    let mut out = String::from("[");
    for (i, e) in visible.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let name = e.path.rsplit('/').next().unwrap_or(&e.path);
        out.push_str(&format!(
            "{{\"name\":\"{}\",\"path\":\"{}\",\"time\":{}}}",
            json_escape(name),
            json_escape(&e.path),
            e.opened
        ));
    }
    out.push(']');
    out
}

/// 从最近列表移除一项（不删除磁盘文件）。成功 1。
pub fn recent_remove(path: &str) -> i32 {
    let removed = {
        let Ok(mut list) = recent_slot().lock() else {
            return 0;
        };
        let before = list.len();
        list.retain(|e| e.path != path);
        before != list.len()
    };
    if removed {
        save_recent();
        1
    } else {
        0
    }
}

/// 重命名最近工程文件并同步列表 / 当前文档路径。成功 1。
///
/// 只允许改文件名，不允许带路径分隔符；`.yin` 后缀缺省时自动补齐。
pub fn recent_rename(path: &str, new_name: &str) -> i32 {
    let name = new_name.trim();
    if name.is_empty() || name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        return 0;
    }
    let old = std::path::Path::new(path);
    if !old.exists() {
        return 0;
    }
    let mut file_name = name.to_string();
    if old.extension().is_some_and(|e| e == "yin") && !file_name.ends_with(".yin") {
        file_name.push_str(".yin");
    }
    let Some(parent) = old.parent() else {
        return 0;
    };
    let new_path = parent.join(&file_name);
    if new_path == old {
        return 1;
    }
    if new_path.exists() {
        return 0;
    }
    if std::fs::rename(old, &new_path).is_err() {
        return 0;
    }
    let new_path_str = new_path.to_string_lossy().to_string();

    // 列表里保留原来的打开时间，并把它置顶。
    if let Ok(mut list) = recent_slot().lock() {
        let opened = list
            .iter()
            .find(|e| e.path == path)
            .map(|e| e.opened)
            .unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            });
        list.retain(|e| e.path != path && e.path != new_path_str);
        list.insert(
            0,
            RecentEntry {
                path: new_path_str.clone(),
                opened,
            },
        );
        list.truncate(10);
    }
    save_recent();

    // 改的是当前打开的工程：同步标题与后续保存路径。
    if let Ok(mut s) = scene().lock()
        && let Some(doc) = s.document.as_mut()
        && doc.file_path.as_deref() == Some(path)
    {
        doc.file_path = Some(new_path_str);
        doc.file_name = file_name;
    }
    1
}

/// 当前文档模型（导出用；无文档返回 None）。
pub fn model_arc() -> Option<Arc<YinModel>> {
    scene()
        .lock()
        .ok()
        .and_then(|s| s.document.as_ref().map(|d| Arc::clone(&d.data.model)))
}
