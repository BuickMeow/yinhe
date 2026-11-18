use super::*;

/// 工程元数据 JSON（工程设置对话框回显）。
pub fn project_meta_json() -> String {
    let Ok(s) = scene().lock() else {
        return "{}".into();
    };
    let Some(doc) = s.document.as_ref() else {
        return "{}".into();
    };
    let m = &doc.data.model.meta;
    let cover = doc
        .data
        .cover
        .as_ref()
        .map(|c| serde_json::json!({"set": true, "mime": c.mime, "size": c.data.len()}))
        .unwrap_or_else(|| serde_json::json!({"set": false}));
    serde_json::json!({
        "name": m.name,
        "artist": m.artist,
        "description": m.description,
        "album": m.album,
        "album_artist": m.album_artist,
        "genre": m.genre,
        "year": m.year,
        "track_no": m.track_no,
        "disc_no": m.disc_no,
        "copyright": m.copyright,
        "ppq": m.ppq,
        // 统一输出迁移后的列表：旧工程只有 artist 时也会得到「艺术家 1」。
        "artists": yinhe_core::SongInfo::from_meta(m).artists,
        "cover": cover,
    })
    .to_string()
}

/// 设置工程封面（读取图片文件；成功 1）。仅当当前文档未在后台任务中。
pub fn set_project_cover(path: String) -> i32 {
    let cover = match yinhe_editor_core::cover::load_cover(std::path::Path::new(&path)) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("设置封面失败: {e}");
            return 0;
        }
    };
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let before = doc.capture_snapshot();
    let old = doc.data.cover.clone();
    doc.data.cover = Some(cover.clone());
    doc.data.bump_revision();
    doc.push_undo(
        yinhe_editor_core::history::UndoAction::Cover {
            old,
            new: Some(cover),
        },
        "Edit cover",
        before,
    );
    1
}

/// 把当前封面写成预览文件，返回路径（无封面返回空串）。
pub fn cover_preview_path() -> String {
    let Some(dir) = files_dir() else {
        return String::new();
    };
    let (data, ext) = {
        let Ok(s) = scene().lock() else {
            return String::new();
        };
        let Some(doc) = s.document.as_ref() else {
            return String::new();
        };
        let Some(cover) = doc.data.cover.as_ref() else {
            return String::new();
        };
        let ext = match cover.mime.as_str() {
            "image/png" => "png",
            "image/webp" => "webp",
            _ => "jpg",
        };
        (cover.data.clone(), ext)
    };
    let path = format!("{dir}/cover_preview.{ext}");
    if std::fs::write(&path, &*data).is_err() {
        return String::new();
    }
    path
}

/// 清除工程封面（成功 1）。
pub fn clear_project_cover() -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    if doc.data.cover.is_none() {
        return 1;
    }
    let before = doc.capture_snapshot();
    let old = doc.data.cover.take();
    doc.data.bump_revision();
    doc.push_undo(
        yinhe_editor_core::history::UndoAction::Cover { old, new: None },
        "Edit cover",
        before,
    );
    1
}

/// 工程元数据编辑（歌名/艺术家/简介/PPQ），差异字段合并为一条 undo。
///
/// PPQ 改动在后台任务里用 `rescale_ppq_with_progress` 执行（可取消/有进度），
/// 与桌面一致；元数据字段在同一任务内一次性提交。
pub fn set_project_meta_json(json: &str) -> i32 {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return 0;
    };
    let text = |key: &str| {
        v.get(key)
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string()
    };
    let info = yinhe_core::SongInfo {
        name: text("name"),
        artist: text("artist"),
        description: text("description"),
        album: text("album"),
        album_artist: text("album_artist"),
        genre: text("genre"),
        year: text("year"),
        track_no: v.get("track_no").and_then(|x| x.as_u64()).unwrap_or(0) as u16,
        disc_no: v.get("disc_no").and_then(|x| x.as_u64()).unwrap_or(0) as u16,
        copyright: text("copyright"),
        artists: v
            .get("artists")
            .and_then(|x| serde_json::from_value(x.clone()).ok())
            .unwrap_or_default(),
    };
    let ppq = v.get("ppq").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
    crate::doc_job::try_start(
        "project-meta",
        crate::doc_job::Reload::Model,
        move |doc, job| {
            use yinhe_editor_core::history::UndoAction;

            let old_ppq = doc.data.model.meta.ppq;
            let has_notes = doc.data.model.note_count > 0;
            let ppq_changed = old_ppq != ppq && ppq > 0;
            let before = doc.capture_snapshot();

            if ppq_changed && has_notes {
                job.set_progress(0.0, "缩放 PPQ");
                let model = (*doc.data.model).clone();
                match yinhe_core::YinModel::rescale_ppq_with_progress(
                    model,
                    ppq,
                    job.progress(),
                    job.cancel_flag(),
                    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                ) {
                    Ok(rescaled) => doc.data.model = std::sync::Arc::new(rescaled),
                    // 取消：文档保持原样（元数据也不提交）。
                    Err(_) => return false,
                }
            }

            let mut actions: Vec<UndoAction> = Vec::new();
            let old_info = yinhe_core::SongInfo::from_meta(&doc.data.model.meta);
            if old_info != info {
                actions.push(UndoAction::SongInfo {
                    old: Box::new(old_info),
                    new: Box::new(info.clone()),
                });
                info.apply_to(&mut std::sync::Arc::make_mut(&mut doc.data.model).meta);
            }
            {
                let model = std::sync::Arc::make_mut(&mut doc.data.model);
                if ppq_changed {
                    if !has_notes {
                        model.meta.ppq = ppq;
                        model.commit_automation(&yinhe_types::automation::AutomationTarget::Tempo);
                    }
                    actions.push(UndoAction::ProjectPpq {
                        old: old_ppq,
                        new: ppq,
                        rescale: has_notes,
                    });
                }
            }
            if actions.is_empty() {
                return false;
            }
            let action = if actions.len() == 1 {
                let Some(action) = actions.pop() else {
                    return false;
                };
                action
            } else {
                UndoAction::Composite(actions)
            };
            doc.push_undo(action, "Edit project meta", before);
            doc.edit.pianoroll_view.base.dirty = true;
            log::info!("工程元数据已更新");
            true
        },
        None::<fn()>,
    )
}

/// 本工程默认 BPM / 拍号 JSON：`{"bpm":120,"num":4,"den":4}`（den = 实际分母）。
///
/// 这两个值只在 conductor 没有原生 Tempo/拍号事件时生效
/// （原生事件优先级更高，可完全代替它们）。
pub fn project_defaults_json() -> String {
    let (bpm, num, den_pow) = scene()
        .lock()
        .ok()
        .and_then(|s| s.model.clone())
        .map(|m| {
            (
                m.meta.default_bpm,
                m.meta.default_time_sig.0,
                m.meta.default_time_sig.1,
            )
        })
        .unwrap_or((120.0, 4, 2));
    let den = 1u32 << den_pow.min(8);
    format!("{{\"bpm\":{bpm},\"num\":{num},\"den\":{den}}}")
}

/// 设置本工程默认 BPM / 拍号（无原生事件时的回退）。成功 1。
pub fn set_project_defaults(bpm: f32, num: u8, den_pow: u8) -> i32 {
    let bpm = if bpm.is_finite() {
        bpm.clamp(0.0, 60_000_000.0)
    } else {
        120.0
    };
    let num = num.clamp(1, 32);
    let den_pow = den_pow.min(6);
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let model = std::sync::Arc::make_mut(&mut doc.data.model);
    if model.meta.default_bpm == bpm && model.meta.default_time_sig == (num, den_pow) {
        return 0;
    }
    model.meta.default_bpm = bpm;
    model.meta.default_time_sig = (num, den_pow);
    model.rebuild_tempo_map();
    let model = doc.data.model.clone();
    refresh_snapshot(&mut s);
    drop(s);
    // tempo_map 变了：重算音符/CC 的采样时间（与 tempo 自动化编辑同机制）。
    crate::audio::reload_notes(model);
    log::info!("工程默认 BPM/拍号: {bpm:.2} BPM, {num}/{}", 1u32 << den_pow);
    1
}
