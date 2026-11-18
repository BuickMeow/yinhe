use super::*;

/// 应用 MIDI Clip 编辑命令：逐条执行 Document 操作并 push undo。
/// 每条命令一条 undo（一次拖动 = 一次撤销）。
pub(crate) fn apply_clip_edit_commands(doc: &mut Document, commands: Vec<view_ui::ClipEditCmd>) {
    for cmd in commands {
        let before = doc.capture_snapshot();
        let label = t!("undo.edit_clip").to_string();
        let action = match cmd {
            view_ui::ClipEditCmd::Move {
                track,
                id,
                delta_ticks,
            } => doc.move_clip(track, id, delta_ticks),
            view_ui::ClipEditCmd::LoopLeft {
                track,
                id,
                new_start_tick,
            } => doc.loop_clip_left(track, id, new_start_tick),
            view_ui::ClipEditCmd::LoopRight {
                track,
                id,
                new_end_tick,
            } => doc.loop_clip_right(track, id, new_end_tick),
        };
        if let Some(action) = action {
            doc.push_undo(action, &label, before);
        }
    }
}

/// 应用音频片段编辑命令：逐条执行 Document 操作并 push undo。
/// 每条命令一条 undo（一次拖动 = 一次撤销）。
pub(crate) fn apply_audio_edit_commands(
    doc: &mut Document,
    library: &crate::app::audio_library::AudioLibrary,
    commands: Vec<view_ui::AudioEditCmd>,
) {
    for cmd in commands {
        let before = doc.capture_snapshot();
        let label = t!("undo.edit_audio").to_string();
        let action = match cmd {
            view_ui::AudioEditCmd::Move {
                track,
                ids,
                delta_seconds,
            } => doc.move_audio_clips(track, &ids, delta_seconds),
            view_ui::AudioEditCmd::TrimStart {
                track,
                id,
                new_start_seconds,
            } => doc.trim_audio_clip_start(track, id, new_start_seconds),
            view_ui::AudioEditCmd::TrimEnd {
                track,
                id,
                new_end_seconds,
            } => doc.trim_audio_clip_end(track, id, new_end_seconds),
            view_ui::AudioEditCmd::SetFades {
                track,
                id,
                fade_in_seconds,
                fade_out_seconds,
            } => {
                let base = clip_params(doc, track, id);
                match base {
                    Some((gain, _, _)) => doc.set_audio_clip_params(
                        track,
                        id,
                        gain,
                        fade_in_seconds,
                        fade_out_seconds,
                    ),
                    None => None,
                }
            }
            view_ui::AudioEditCmd::Split {
                track,
                id,
                at_seconds,
            } => doc.split_audio_clip(track, id, at_seconds).map(|(a, _)| a),
            view_ui::AudioEditCmd::Duplicate {
                track,
                ids,
                delta_seconds,
            } => doc
                .duplicate_audio_clips(track, &ids, delta_seconds)
                .map(|(a, _)| a),
            view_ui::AudioEditCmd::Delete { track, ids } => {
                let action = doc.delete_audio_clips(track, &ids);
                // 删除后清掉失效的选择项。
                doc.edit
                    .selected_audio_clips
                    .retain(|(t, id)| *t as usize != track || !ids.contains(id));
                action
            }
            view_ui::AudioEditCmd::Reverse { track, ids } => {
                doc.toggle_audio_clips_reverse(track, &ids)
            }
            view_ui::AudioEditCmd::Normalize { track, id } => {
                let peak = doc
                    .model()
                    .tracks
                    .get(track)
                    .and_then(|t| t.audio_clips.iter().find(|c| c.id == id))
                    .and_then(|c| library.get(&c.source))
                    .map(|d| d.peaks.peak_max())
                    .unwrap_or(0.0);
                doc.normalize_audio_clip(track, id, peak)
            }
            view_ui::AudioEditCmd::GainDelta {
                track,
                id,
                delta_db,
            } => {
                let base = clip_params(doc, track, id);
                match base {
                    Some((gain, fi, fo)) => {
                        let new_gain = gain * 10f32.powf(delta_db / 20.0);
                        doc.set_audio_clip_params(track, id, new_gain, fi, fo)
                    }
                    None => None,
                }
            }
            view_ui::AudioEditCmd::GainReset { track, id } => {
                let base = clip_params(doc, track, id);
                match base {
                    Some((_, fi, fo)) => doc.set_audio_clip_params(track, id, 1.0, fi, fo),
                    None => None,
                }
            }
        };
        if let Some(action) = action {
            doc.push_undo(action, &label, before);
        }
    }
}

/// 读取片段当前 (gain, fade_in, fade_out)（只读一次性拷贝，避免借用冲突）。
fn clip_params(doc: &Document, track: usize, id: u32) -> Option<(f32, f64, f64)> {
    doc.model()
        .tracks
        .get(track)
        .and_then(|t| t.audio_clips.iter().find(|c| c.id == id))
        .map(|c| (c.gain, c.fade_in_seconds, c.fade_out_seconds))
}
