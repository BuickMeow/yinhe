use super::*;

/// 压缩包扩展名（与桌面 `file_loading::load_path` 同一份列表）。
const ARCHIVE_EXTS: &[&str] = &[
    "zip", "7z", "rar", "lzh", "lha", "tar", "gz", "xz", "tgz", "txz", "tbz", "bz2",
];

/// 路径是否压缩包（按扩展名判断，与桌面一致）。
pub fn is_archive_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    ARCHIVE_EXTS
        .iter()
        .any(|e| lower.ends_with(&format!(".{e}")))
}

/// 压缩包会话：打开后只收集 MIDI 条目元数据（懒解压）。
struct ArchiveSession {
    path: String,
    archive: Option<yinhe_archive::Archive>,
    entries: Vec<yinhe_archive::ArchiveEntry>,
    /// 0=打开中 1=就绪 2=需要密码 3=失败
    status: u8,
    error: String,
}

static ARCHIVE: OnceLock<Mutex<Option<ArchiveSession>>> = OnceLock::new();

fn archive_slot() -> &'static Mutex<Option<ArchiveSession>> {
    ARCHIVE.get_or_init(|| Mutex::new(None))
}

/// 压缩包状态：0=无 1=打开中 2=就绪 3=需要密码 4=失败。
pub fn archive_status() -> i32 {
    match archive_slot()
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

/// 压缩包信息 JSON：`{"path":..,"error":..,"entries":[{"name":..,"size":..}]}`。
pub fn archive_json() -> String {
    let Ok(slot) = archive_slot().lock() else {
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

/// 打开压缩包（后台线程，只列 MIDI 条目）。Dart 轮询 `archive_status`。
pub fn open_archive(path: String, password: Option<String>) -> i32 {
    if let Ok(mut slot) = archive_slot().lock() {
        *slot = Some(ArchiveSession {
            path: path.clone(),
            archive: None,
            entries: Vec::new(),
            status: 0,
            error: String::new(),
        });
    }
    let path_for_thread = path.clone();
    let spawn = std::thread::Builder::new()
        .name("yinhe-archive-open".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let result = match password.as_deref() {
                Some(pw) => yinhe_archive::Archive::open_with_password(&path_for_thread, Some(pw)),
                None => yinhe_archive::Archive::open(&path_for_thread),
            };
            let Ok(mut slot) = archive_slot().lock() else {
                return;
            };
            let Some(session) = slot.as_mut() else {
                return;
            };
            match result {
                Ok(archive) => {
                    session.entries = archive.list_midi_files();
                    session.archive = Some(archive);
                    session.status = 1;
                    log::info!(
                        "压缩包已打开: {path_for_thread}（{} 个 MIDI）",
                        session.entries.len()
                    );
                }
                Err(yinhe_archive::ArchiveError::PasswordRequired) => {
                    session.status = 2;
                    log::info!("压缩包需要密码: {path_for_thread}");
                }
                Err(e) => {
                    session.error = e.to_string();
                    session.status = 3;
                    log::error!("压缩包打开失败（{path_for_thread}）: {e}");
                }
            }
        });
    if let Err(e) = spawn {
        log::error!("压缩包加载线程创建失败: {e}");
        return 0;
    }
    1
}

/// 打开压缩包内第 `index` 个 MIDI 条目（后台解析并替换场景）。
pub fn open_archive_entry(index: usize) -> i32 {
    let (archive, entry_name) = {
        let Ok(slot) = archive_slot().lock() else {
            return 0;
        };
        let Some(session) = slot.as_ref() else {
            return 0;
        };
        if session.status != 1 {
            return 0;
        }
        let Some(entry) = session.entries.get(index) else {
            return 0;
        };
        let Some(archive) = session.archive.clone() else {
            return 0;
        };
        (archive, entry.name.clone())
    };
    let display = format!("{}#{entry_name}", archive.path().display());
    let token = begin_load(&display);
    let spawn = std::thread::Builder::new()
        .name("yinhe-archive-entry".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let data = match archive.read_file(&entry_name) {
                Ok(d) => d,
                Err(e) => {
                    log::error!("压缩包条目解压失败（{entry_name}）: {e}");
                    finish_load(token, Err(format!("解压失败: {e}")));
                    return;
                }
            };
            set_load_progress(token, 0.10);
            match load_midi_bytes_document(data, &display, token) {
                Ok(doc) => {
                    if !load_current(token) {
                        return;
                    }
                    push_recent(&archive.path().display().to_string());
                    apply_document(doc, 0.0);
                    finish_load(token, Ok(()));
                }
                Err(e) => {
                    log::error!("压缩包条目解析失败（{entry_name}）: {e}");
                    finish_load(token, Err(e));
                }
            }
        });
    if let Err(e) = spawn {
        log::error!("压缩包条目加载线程创建失败: {e}");
        finish_load(token, Err(format!("加载线程创建失败: {e}")));
        return 0;
    }
    1
}

/// 从 MIDI 字节构建文档（压缩包条目用；`path` 只用于显示/最近记录）。
fn load_midi_bytes_document(data: Vec<u8>, path: &str, token: u64) -> Result<Document, String> {
    let started = std::time::Instant::now();
    let model =
        yinhe_midi::parse_bytes_with_encoding(&data, yinhe_midi::MidiImportEncoding::Utf8, |p| {
            let frac = if p.total_tracks > 0 {
                p.current_track as f32 / p.total_tracks as f32
            } else {
                0.0
            };
            set_load_progress(token, 0.10 + frac * 0.78);
        })
        .map_err(|e| format!("解析失败: {e}"))?;
    // 解析完立即释放解压出来的原始字节，不要拖到 Document 构建之后。
    drop(data);
    set_load_progress(token, 0.90);
    log::info!(
        "MIDI 解析完成（压缩包条目）: {} 音符，用时 {:?}",
        model.note_count,
        started.elapsed()
    );
    let project_file = yinhe_yin::ProjectFile::from_meta(&model.meta);
    let mapping_file = yinhe_yin::MappingFile::default();
    let mut doc = Document::from_model(
        path,
        model,
        QuantizePreset::Fraction(1, 1),
        QuantizePreset::Fraction(1, 16),
        project_file,
        mapping_file,
        None,
    )?;
    doc.mark_loaded();
    doc.sync_track_caches_with_conductor_color(crate::theme_colors::text_primary());
    doc.file_path = Some(path.to_string());
    Ok(doc)
}

/// 文件加载进度（Flutter 轮询）：0=空闲 1=加载中 2=完成 3=失败。
struct LoadStatus {
    state: u8,
    error: String,
    token: u64,
    progress: f32,
}

static LOAD_STATUS: OnceLock<Mutex<LoadStatus>> = OnceLock::new();

fn load_slot() -> &'static Mutex<LoadStatus> {
    LOAD_STATUS.get_or_init(|| {
        Mutex::new(LoadStatus {
            state: 0,
            error: String::new(),
            token: 0,
            progress: 0.0,
        })
    })
}

fn begin_load(path: &str) -> u64 {
    let Ok(mut st) = load_slot().lock() else {
        return 0;
    };
    st.token = st.token.wrapping_add(1);
    st.state = 1;
    st.error.clear();
    st.progress = 0.0;
    log::info!("开始加载: {path}");
    st.token
}

fn set_load_progress(token: u64, progress: f32) {
    if let Ok(mut st) = load_slot().lock()
        && st.token == token
        && st.state == 1
    {
        let p = progress.clamp(0.0, 1.0);
        if p > st.progress {
            st.progress = p;
        }
    }
}

fn load_current(token: u64) -> bool {
    load_slot()
        .lock()
        .map(|st| st.token == token && st.state == 1)
        .unwrap_or(false)
}

fn finish_load(token: u64, result: Result<(), String>) {
    let Ok(mut st) = load_slot().lock() else {
        return;
    };
    if st.token != token {
        return;
    }
    match result {
        Ok(()) => {
            st.state = 2;
            st.progress = 1.0;
            st.error.clear();
        }
        Err(e) => {
            st.state = 3;
            st.error = e;
        }
    }
}

/// `{"state":1,"error":"","progress":0.42}`
pub fn load_status_json() -> String {
    let Ok(st) = load_slot().lock() else {
        return "{\"state\":0,\"error\":\"\",\"progress\":0}".into();
    };
    format!(
        "{{\"state\":{},\"error\":\"{}\",\"progress\":{:.4}}}",
        st.state,
        json_escape(&st.error),
        st.progress.clamp(0.0, 1.0)
    )
}

/// 保存进度：0=空闲 1=保存中 2=完成 3=失败。
struct SaveStatus {
    state: u8,
    progress: f32,
    error: String,
}

static SAVE_STATUS: OnceLock<Mutex<SaveStatus>> = OnceLock::new();

fn save_slot() -> &'static Mutex<SaveStatus> {
    SAVE_STATUS.get_or_init(|| {
        Mutex::new(SaveStatus {
            state: 0,
            progress: 0.0,
            error: String::new(),
        })
    })
}

/// 工程保存进度 JSON（Dart 轮询）。
pub fn save_status_json() -> String {
    let Ok(st) = save_slot().lock() else {
        return "{\"state\":0,\"progress\":0,\"error\":\"\"}".into();
    };
    format!(
        "{{\"state\":{},\"progress\":{:.4},\"error\":\"{}\"}}",
        st.state,
        st.progress.clamp(0.0, 1.0),
        json_escape(&st.error)
    )
}

/// 后台保存 `.yin`（带进度）。返回 1 = 已开始。
pub fn save_project_start(path: String) -> i32 {
    {
        let Ok(st) = save_slot().lock() else {
            return 0;
        };
        if st.state == 1 {
            return 0;
        }
    }

    // 保存前在 UI 线程生成快照：把编辑态（静音/独奏、每通道音色库覆盖、
    // 映射）同步回持久化结构；移动端不编辑混音台，但会原样保留从桌面
    // 工程读到的 mixer 段，避免跨端另存时丢失。
    let (model, project_file, mapping_file, mixer) = {
        let Ok(mut s) = scene().lock() else {
            return 0;
        };
        let Some(doc) = s.document.as_mut() else {
            return 0;
        };
        doc.sync_overrides_to_model();
        doc.data.sync_project_file();
        doc.data.sync_mapping_file();
        doc.sync_sf_overrides_to_project_file();
        (
            Arc::clone(&doc.data.model),
            doc.data.project_file.clone(),
            doc.data.mapping_file.clone(),
            doc.mixer.clone(),
        )
    };

    {
        let Ok(mut st) = save_slot().lock() else {
            return 0;
        };
        if st.state == 1 {
            return 0;
        }
        st.state = 1;
        st.progress = 0.0;
        st.error.clear();
    }
    let path_for_thread = path.clone();
    let spawn = std::thread::Builder::new()
        .name("yinhe-save-project".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let result = yinhe_yin::save_yin_with_files_progress(
                &model,
                &path_for_thread,
                &project_file,
                &mapping_file,
                Some(&mixer),
                |p| {
                    let (start, end) = match p.stage {
                        yinhe_yin::YinProgressStage::Collect => (0.0, 0.05),
                        yinhe_yin::YinProgressStage::Sort => (0.05, 0.65),
                        yinhe_yin::YinProgressStage::Compress => (0.65, 1.0),
                        _ => (0.0, 1.0),
                    };
                    let next = start + (end - start) * p.fraction.clamp(0.0, 1.0);
                    if let Ok(mut st) = save_slot().lock()
                        && next > st.progress
                    {
                        st.progress = next;
                    }
                },
            );
            let Ok(mut st) = save_slot().lock() else {
                return;
            };
            match result {
                Ok(()) => {
                    if let Ok(mut s) = scene().lock()
                        && let Some(doc) = s.document.as_mut()
                    {
                        doc.file_path = Some(path_for_thread.clone());
                    }
                    st.state = 2;
                    st.progress = 1.0;
                    st.error.clear();
                    log::info!("工程已导出: {path_for_thread}");
                }
                Err(e) => {
                    st.state = 3;
                    st.error = e.to_string();
                    log::error!("工程导出失败: {e}");
                }
            }
        });
    if let Err(e) = spawn {
        if let Ok(mut st) = save_slot().lock() {
            st.state = 3;
            st.error = format!("保存线程创建失败: {e}");
        }
        return 0;
    }
    1
}

/// 当前文档来源路径（无文档时返回空串）。
pub fn current_path() -> String {
    scene()
        .lock()
        .ok()
        .and_then(|s| s.source_path.clone())
        .unwrap_or_default()
}

/// 同一文件已经在当前文档中：把加载状态标记为完成（Dart 不弹进度框）。
fn mark_load_done() {
    if let Ok(mut st) = load_slot().lock() {
        st.token = st.token.wrapping_add(1);
        st.state = 2;
        st.progress = 1.0;
        st.error.clear();
    }
}

/// 后台加载用户选择的文件（.mid / .yin / 压缩包）并替换当前场景。
pub fn open_midi(path: String) {
    if is_archive_path(&path) {
        open_archive(path, None);
        return;
    }
    if current_path() == path {
        log::info!("同一文件已加载，跳过重复解析: {path}");
        mark_load_done();
        return;
    }
    // 预检可读性：外置存储的路径在分区存储 / 鸿蒙上可能已经失去权限。
    // 直接给可操作的提示，而不是让用户看到裸的 "os error 13"。
    if let Err(e) = std::fs::File::open(&path)
        && e.kind() == std::io::ErrorKind::PermissionDenied
    {
        finish_load(
            begin_load(&path),
            Err("无权读取该文件：请重新用「打开」选择（文件会复制到应用内），                 或把音色库/工程打成压缩包导入"
                .into()),
        );
        return;
    }
    let token = begin_load(&path);
    let path_for_thread = path.clone();
    let spawn = std::thread::Builder::new()
        .name("yinhe-midi-load".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || match load_document(&path_for_thread, token) {
            Ok(doc) => {
                if !load_current(token) {
                    return;
                }
                apply_document(doc, 0.0);
                finish_load(token, Ok(()));
            }
            Err(e) => {
                log::error!("文件加载失败（{path_for_thread}）: {e}");
                finish_load(token, Err(e));
            }
        });
    if let Err(e) = spawn {
        log::error!("加载线程创建失败: {e}");
        finish_load(token, Err(format!("加载线程创建失败: {e}")));
    }
}

/// 按扩展名解析文件并组装为可渲染/可播放的文档（工程 / MIDI）。
fn load_document(path: &str, token: u64) -> Result<Document, String> {
    if yinhe_yin::is_project_path(path) {
        return load_project_document(path, token);
    }
    let started = std::time::Instant::now();
    set_load_progress(token, 0.02);
    // mmap 直接解析文件，不再先把整份字节读进匿名堆（大 MIDI 省一个文件大小）。
    let model =
        yinhe_midi::parse_file_with_encoding(path, yinhe_midi::MidiImportEncoding::Utf8, |p| {
            let frac = if p.total_tracks > 0 {
                p.current_track as f32 / p.total_tracks as f32
            } else {
                0.0
            };
            set_load_progress(token, 0.02 + frac * 0.78);
        })
        .map_err(|e| format!("加载失败: {e}"))?;
    set_load_progress(token, 0.82);
    log::info!(
        "MIDI 解析完成: {} 音符，用时 {:?}",
        model.note_count,
        started.elapsed()
    );

    let project_file = yinhe_yin::ProjectFile::from_meta(&model.meta);
    let mapping_file = yinhe_yin::MappingFile::default();
    let mut doc = Document::from_model(
        path,
        model,
        QuantizePreset::Fraction(1, 1),
        QuantizePreset::Fraction(1, 16),
        project_file,
        mapping_file,
        None,
    )?;
    doc.mark_loaded();
    // 与演示文档一致的 conductor 色（保证两台机器/两种来源配色相同）。
    doc.sync_track_caches_with_conductor_color(crate::theme_colors::text_primary());
    // 记录来源：加载完成即视为“已落盘来源”，dirty 从干净状态起步；
    // 同时供主页重复选择同一文件时跳过解析。
    if crate::autosave::is_autosave_path(path) {
        // 恢复备份：不绑定备份文件路径，保持“未保存”让用户重新落盘。
        if let Some(name) = crate::autosave::name_for_path(path) {
            doc.file_name = name;
        }
        doc.file_path = None;
        crate::autosave::clear();
    } else {
        doc.file_path = Some(path.to_string());
        // 打开别的文件：旧文档的自动保存没有意义了。
        crate::autosave::clear();
        push_recent(path);
    }
    Ok(doc)
}

/// 解析 `.yin` 工程文件。
///
/// 移动端没有混音台/每通道音色库 UI，但会把文件里的 `mixer` 段和
/// `sf_channel_overrides` 原样装进 Document，保存时再写回：桌面工程
/// 在手机上另存不会丢混音台或音色库覆盖。
fn load_project_document(path: &str, token: u64) -> Result<Document, String> {
    let started = std::time::Instant::now();
    set_load_progress(token, 0.02);
    let (model, sf, mapping, mixer, cover) = yinhe_yin::load_yin_with_sf_progress(path, |p| {
        let frac = match p.stage {
            yinhe_yin::YinProgressStage::Decompress => p.fraction * 0.55,
            yinhe_yin::YinProgressStage::Rebuild => 0.55 + p.fraction * 0.25,
            yinhe_yin::YinProgressStage::Resort => 0.80 + p.fraction * 0.10,
            _ => p.fraction * 0.90,
        };
        set_load_progress(token, 0.02 + frac * 0.88);
    })
    .map_err(|e| format!("工程解析失败: {e}"))?;
    log::info!(
        "工程解析完成: {} 音符，用时 {:?}",
        model.note_count,
        started.elapsed()
    );
    let mut project_file = yinhe_yin::ProjectFile::from_meta(&model.meta);
    project_file.cover = cover;
    let mut doc = Document::from_model(
        path,
        model,
        QuantizePreset::Fraction(1, 1),
        QuantizePreset::Fraction(1, 16),
        project_file,
        mapping,
        mixer,
    )?;
    doc.apply_sf_overrides(&sf.overrides);
    doc.sync_sf_overrides_to_project_file();
    // 打开既有工程视为已保存（dirty 起点干净）。
    doc.mark_saved();
    if crate::autosave::is_autosave_path(path) {
        if let Some(name) = crate::autosave::name_for_path(path) {
            doc.file_name = name;
        }
        doc.file_path = None;
        crate::autosave::clear();
    } else {
        doc.file_path = Some(path.to_string());
        crate::autosave::clear();
        push_recent(path);
    }
    Ok(doc)
}

/// 新建空工程（仅 Conductor，其余音轨由 AR 末尾的加号创建；120BPM、4/4）。
pub fn new_project() -> i32 {
    crate::autosave::clear();
    let doc = Document::empty_conductor_only();
    // 空工程给 4 小节的初始时间轴长度，避免除零缩放。
    apply_document(doc, 1920.0);
    1
}

/// 没有工程时的工作区占位文档：只保留 Conductor，AR 末尾的加号用于建轨。
///
/// 时间轴按新工程规则预留 64 小节；已有文档时不做任何事。
pub fn ensure_starter_document() -> i32 {
    if scene().lock().map(|s| s.document.is_some()).unwrap_or(true) {
        return 0;
    }
    apply_document(Document::empty_conductor_only(), 1920.0);
    log::info!("空白工作区：已创建 Conductor 占位文档");
    1
}

/// 保存完成后标记工程状态（清除 dirty）。
pub fn mark_saved() -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    doc.mark_saved();
    crate::autosave::clear();
    1
}

/// 工程状态：`[can_undo, can_redo, dirty]`。
pub fn doc_state() -> [i32; 3] {
    let Ok(s) = scene().lock() else {
        return [0, 0, 0];
    };
    let Some(doc) = s.document.as_ref() else {
        return [0, 0, 0];
    };
    [
        i32::from(doc.undo_past_len() > 0),
        i32::from(!doc.redo_labels().is_empty()),
        i32::from(doc.is_dirty()),
    ]
}

/// 文档/选区世代号（Dart 在变化时刷新轨道/自动化元数据）。
pub fn document_generation() -> u32 {
    scene().lock().map(|s| s.doc_generation).unwrap_or(0)
}

/// 把文档写入场景快照（演示与文件加载共用）。
///
/// `min_ticks`：演示内容生成时模型 tick_length 尚未刷新，用它兜底总长。
fn apply_document(mut doc: Document, min_ticks: f64) {
    // 新建/占位文档也必须同步 Conductor 色：`Document::from_empty_model` 里
    // 用的是暗色主题 fallback（220,220,220），浅色背景下那条 6px 色带几乎看不见。
    doc.sync_track_caches_with_conductor_color(crate::theme_colors::text_primary());
    // 移动端编辑设置与桌面同名配置对齐（当前只有重叠开关）。
    doc.edit.allow_overlapping_notes = crate::audio::settings().allow_overlapping_notes;
    doc.edit.overlap_blocked_behavior =
        yinhe_editor_core::audio_settings::OverlapBlockedBehavior::ReplaceTarget;
    // 初始缩放只看实际内容（不含尾部 64 小节，避免打开时音符变小）；
    // 可视总长额外预留 64 小节（与桌面 follow::total_ticks_padded 一致）。
    let content_ticks = (doc.data.model.tick_length as f64).max(min_ticks).max(1.0);
    let total_ticks = total_ticks_padded(doc.data.model.tick_length, doc.data.model.meta.ppq)
        .max(min_ticks)
        .max(1.0);

    let mut pr_view = PianoRollView::default();
    // 初始缩放：每 tick 0.5 像素（直接、可预期；不再按屏宽拟合）。
    pr_view.base.pixels_per_tick = 0.5;

    let arr_base = TimelineViewBase {
        pixels_per_tick: (1280.0 * 2.0 / content_ticks as f32).clamp(
            yinhe_types::PIXELS_PER_TICK_MIN,
            yinhe_types::PIXELS_PER_TICK_MAX,
        ),
        scroll_x: 0.0,
        scroll_y: 0.0,
        left_panel_width: AR_TRACK_PANEL_W,
        dirty: true,
        track_panel_row_height: AR_LANE_HEIGHT,
        track_panel_scroll_y: 0.0,
        follow_target: None,
        follow_anim_start: 0.0,
        follow_anim_elapsed: 0.0,
    };
    let arr_view = ArrangementView::with_base(arr_base);

    let model = doc.data.model.clone();
    let note_count = model.note_count;
    let source_path = doc.file_path.clone();

    let Ok(mut s) = scene().lock() else {
        return;
    };
    s.document = Some(doc);
    s.source_path = source_path;
    s.model_epoch = s.model_epoch.wrapping_add(1);
    s.pr_view = pr_view;
    s.arr_view = arr_view;
    s.pr_key_zoom_accum = 1.0;
    s.view_mode = ViewMode::Arrangement;
    s.total_ticks = total_ticks;
    // 新文档从 0 开始：清掉上一个工程残留的非播放态光标。
    s.cursor_tick = 0.0;
    refresh_snapshot(&mut s);
    let num_tracks = s.num_tracks;
    drop(s);

    log::info!("文档就绪: {note_count} 音符, {total_ticks:.0} tick, {num_tracks} 轨");
    crate::audio::on_model_loaded(model);
}
