use super::*;

/// 设置文件写入（音色库 + 音频参数持久化）。
pub fn save_soundfont_settings() {
    write_settings();
}

/// 音频设置变更后持久化（设置页调用）。
pub fn save_audio_settings() {
    write_settings();
}

/// 音符重叠设置（复用桌面 `allow_overlapping_notes`）：立即应用到当前文档，
/// 新建工程/打开工程时由 [`apply_document`] 再次同步。
pub fn set_allow_overlapping_notes(allow: bool) {
    crate::audio::set_allow_overlapping_notes(allow);
    if let Ok(mut s) = scene().lock()
        && let Some(doc) = s.document.as_mut()
    {
        doc.edit.allow_overlapping_notes = allow;
        doc.edit.overlap_blocked_behavior =
            yinhe_editor_core::audio_settings::OverlapBlockedBehavior::ReplaceTarget;
    }
    save_audio_settings();
}

/// 卷帘背景音名开关（设置页）。
pub fn set_show_keyboard_labels(show: bool) {
    crate::audio::set_show_keyboard_labels(show);
    save_audio_settings();
}

pub fn show_keyboard_labels() -> bool {
    crate::audio::show_keyboard_labels()
}

/// 播放时视口跟随播放头（居中）。
pub fn set_follow_playhead(on: bool) {
    crate::audio::set_follow_playhead(on);
    save_audio_settings();
}

pub fn follow_playhead() -> bool {
    crate::audio::follow_playhead()
}

/// 进入主页/退出应用时暂停播放。
pub fn set_pause_on_home(on: bool) {
    crate::audio::set_pause_on_home(on);
    save_audio_settings();
}

pub fn pause_on_home() -> bool {
    crate::audio::pause_on_home()
}

pub fn set_pause_on_exit(on: bool) {
    crate::audio::set_pause_on_exit(on);
    save_audio_settings();
}

pub fn pause_on_exit() -> bool {
    crate::audio::pause_on_exit()
}

/// 自动保存开关（持久化到 settings.txt）。
pub fn set_auto_save_enabled(on: bool) {
    crate::audio::set_auto_save_enabled(on);
    save_audio_settings();
}

fn write_settings() {
    let Some(dir) = files_dir_slot().lock().ok().and_then(|g| g.clone()) else {
        return;
    };
    let mut text = String::new();
    // 音色库表：每行一个 `soundfont=<0|1>|<path>`（0/1 = 停用/启用）。
    for e in crate::audio::soundfonts() {
        text.push_str(&format!(
            "soundfont={}|{}\n",
            if e.enabled { 1 } else { 0 },
            portable_doc_path(&e.path)
        ));
    }
    text.push_str(&format!("ui_language={}\n", ui_language()));
    text.push_str(&format!("ui_theme={}\n", ui_theme()));
    let a = crate::audio::settings();
    text.push_str(&format!(
        "max_voices={}\nbuffer_size={}\nignore_velocity={}\nallow_overlapping_notes={}\n\
         show_keyboard_labels={}\nfollow_playhead={}\npause_on_home={}\npause_on_exit={}\nauto_save={}\nexport_audio_metadata={}\n",
        a.max_voices,
        a.buffer_size,
        a.ignore_velocity,
        if a.allow_overlapping_notes { 1 } else { 0 },
        if a.show_keyboard_labels { 1 } else { 0 },
        if a.follow_playhead { 1 } else { 0 },
        if a.pause_on_home { 1 } else { 0 },
        if a.pause_on_exit { 1 } else { 0 },
        if a.auto_save_enabled { 1 } else { 0 },
        if a.export_audio_metadata { 1 } else { 0 }
    ));
    if let Err(e) = std::fs::write(format!("{dir}/settings.txt"), text) {
        log::error!("设置写入失败: {e}");
    }
}

/// 读取并应用持久化设置（启动时由 `set_files_dir` 触发）。
pub(super) fn load_settings() {
    let Some(dir) = files_dir_slot().lock().ok().and_then(|g| g.clone()) else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(format!("{dir}/settings.txt")) else {
        return;
    };
    let defaults = crate::audio::AudioSettings::default();
    let mut max_voices = defaults.max_voices;
    let mut buffer_size = defaults.buffer_size;
    let mut ignore_velocity = defaults.ignore_velocity;
    let mut allow_overlapping_notes = defaults.allow_overlapping_notes;
    let mut show_keyboard_labels = defaults.show_keyboard_labels;
    let mut follow_playhead = defaults.follow_playhead;
    let mut pause_on_home = defaults.pause_on_home;
    let mut pause_on_exit = defaults.pause_on_exit;
    let mut auto_save = defaults.auto_save_enabled;
    let mut export_audio_metadata = defaults.export_audio_metadata;
    let mut soundfonts: Vec<crate::audio::SoundfontEntry> = Vec::new();
    let mut language = String::new();
    let mut theme = String::new();
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("soundfont=") {
            // 常规格式 `enabled|path`；旧格式 `path`（视为启用）。
            // 三段 `enabled|channel|path` 是已回滚的按通道绑定版本写的，
            // 读取时丢弃中间段，避免用户设置丢失。
            let (enabled, path) = match v.splitn(3, '|').collect::<Vec<_>>()[..] {
                [flag, _channel, path] => (flag != "0", path),
                [flag, path] => (flag != "0", path),
                [path] => (true, path),
                _ => (true, ""),
            };
            if !path.is_empty() && crate::audio::is_soundfont_path(path) {
                let path = resolve_existing_soundfont(path);
                // 重装/恢复备份后 settings.txt 可能还在，但音色库文件已被清掉；
                // 保留条目只会让引擎加载空音色库，表现为“能播放但没声音”。
                if !std::path::Path::new(&path).exists() {
                    log::warn!("音色库文件不存在，已跳过: {path}");
                    continue;
                }
                soundfonts.push(crate::audio::SoundfontEntry { path, enabled });
            } else if !path.is_empty() {
                // 旧版本选择器可能把 mid 等文件写进音色库表，读盘时清理掉。
                log::warn!("忽略无效音色库设置: {path}");
            }
        } else if let Some(v) = line.strip_prefix("ui_language=") {
            language = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("ui_theme=") {
            theme = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("max_voices=") {
            max_voices = v.trim().parse().unwrap_or(max_voices);
        } else if let Some(v) = line.strip_prefix("buffer_size=") {
            buffer_size = v.trim().parse().unwrap_or(buffer_size);
        } else if let Some(v) = line.strip_prefix("ignore_velocity=") {
            ignore_velocity = v.trim().parse().unwrap_or(ignore_velocity);
        } else if let Some(v) = line.strip_prefix("allow_overlapping_notes=") {
            allow_overlapping_notes = v.trim() != "0";
        } else if let Some(v) = line.strip_prefix("show_keyboard_labels=") {
            show_keyboard_labels = v.trim() != "0";
        } else if let Some(v) = line.strip_prefix("follow_playhead=") {
            follow_playhead = v.trim() != "0";
        } else if let Some(v) = line.strip_prefix("pause_on_home=") {
            pause_on_home = v.trim() != "0";
        } else if let Some(v) = line.strip_prefix("pause_on_exit=") {
            pause_on_exit = v.trim() != "0";
        } else if let Some(v) = line.strip_prefix("auto_save=") {
            auto_save = v.trim() != "0";
        } else if let Some(v) = line.strip_prefix("export_audio_metadata=") {
            export_audio_metadata = v.trim() != "0";
        }
    }
    if !language.is_empty() {
        set_ui_language(language);
    }
    if !theme.is_empty() {
        set_ui_theme(theme);
    }
    if !soundfonts.is_empty() {
        log::info!("恢复音色库设置: {} 个", soundfonts.len());
        crate::audio::set_soundfonts(soundfonts);
    }
    crate::audio::set_settings(max_voices, buffer_size, ignore_velocity);
    crate::audio::set_allow_overlapping_notes(allow_overlapping_notes);
    crate::audio::set_show_keyboard_labels(show_keyboard_labels);
    crate::audio::set_follow_playhead(follow_playhead);
    crate::audio::set_pause_on_home(pause_on_home);
    crate::audio::set_pause_on_exit(pause_on_exit);
    crate::audio::set_auto_save_enabled(auto_save);
    crate::audio::set_export_audio_metadata(export_audio_metadata);
    crate::soundfont_archive::cleanup();
}

static UI_LANGUAGE: OnceLock<Mutex<String>> = OnceLock::new();

fn language_slot() -> &'static Mutex<String> {
    UI_LANGUAGE.get_or_init(|| Mutex::new("system".to_string()))
}

/// 界面语言设置：`system` / `zh` / `en`。
pub fn ui_language() -> String {
    language_slot()
        .lock()
        .map(|g| g.clone())
        .unwrap_or_else(|_| "system".into())
}

/// 写入界面语言（可选值 system/zh/en，其它忽略）。
pub fn set_ui_language(lang: String) -> i32 {
    /// 桌面端同款 15 种界面语言代码。
    const UI_LANGS: &[&str] = &[
        "zh", "zh-TW", "zh-HK", "en", "ja", "ko", "de", "fr", "es", "it", "pt", "ru", "pl", "cs",
        "tr",
    ];
    let v = match lang.as_str() {
        // 兼容旧设置（zh-CN/en-US 及两语言版本）。
        "zh-CN" => "zh".to_string(),
        "en-US" => "en".to_string(),
        other if UI_LANGS.contains(&other) => other.to_string(),
        _ => "system".to_string(),
    };
    if let Ok(mut g) = language_slot().lock() {
        *g = v.clone();
    }
    log::info!("界面语言: {v}");
    1
}

static UI_THEME: OnceLock<Mutex<String>> = OnceLock::new();

fn theme_slot() -> &'static Mutex<String> {
    UI_THEME.get_or_init(|| Mutex::new("auto".to_string()))
}

/// 界面主题（light/dark）。
pub fn ui_theme() -> String {
    theme_slot()
        .lock()
        .map(|g| g.clone())
        .unwrap_or_else(|_| "light".into())
}

/// 写入界面主题（light/dark）。
pub fn set_ui_theme(theme: String) -> i32 {
    let v = match theme.as_str() {
        "dark" => "dark",
        "auto" | "system" => "auto",
        _ => "light",
    };
    if let Ok(mut g) = theme_slot().lock() {
        *g = v.to_string();
    }
    log::info!("界面主题偏好: {v}");
    1
}

/// 主题实际明暗切换后：重刷 Conductor（000）色带并让 Flutter/wgpu 快照跟上。
pub fn apply_theme_colors() {
    let Ok(mut s) = scene().lock() else {
        return;
    };
    if let Some(doc) = s.document.as_mut() {
        doc.sync_track_caches_with_conductor_color(crate::theme_colors::text_primary());
    }
    refresh_snapshot(&mut s);
}

/// 当前工程累计前台工作时间（秒；无文档返回 0）。
pub fn work_time_secs() -> u64 {
    scene()
        .lock()
        .ok()
        .and_then(|s| s.document.as_ref().map(|d| d.data.work_time_secs))
        .unwrap_or(0)
}

/// 前台计时器每秒钟累加一次；不 bump revision、不触发重渲染。
pub fn add_work_time(secs: u64) -> u64 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    doc.data.work_time_secs = doc.data.work_time_secs.saturating_add(secs);
    doc.data.work_time_secs
}

/// 全曲音符数（顶栏标题展示用）。
pub fn note_count() -> i64 {
    scene()
        .lock()
        .ok()
        .and_then(|s| s.model.clone())
        .map(|m| m.note_count as i64)
        .unwrap_or(0)
}
