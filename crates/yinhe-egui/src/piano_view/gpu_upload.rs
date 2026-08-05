//! GPU cull 模式下的音符 buffer 上传逻辑。
//!
//! 策略：先尝试增量 per-key 上传，失败则回退全量上传。
//! - hidden_notes 变了 → 必须全量上传（影响 per-key 内容）
//! - revision 变了且 per-key revision 匹配 → 跳过（已上传）
//! - revision 变了且部分 key 不同 → 尝试增量（count 必须匹配）
//! - revision 变了且 count 不匹配 → 全量上传
//! - 仅 track_visible 变了 → 后台重建：`build_all_notes` 挪后台线程，
//!   完成后 UI 线程分帧上传（每帧 `KEYS_PER_FRAME` 个 key），期间 GPU
//!   用 track_mask 过滤旧 buffer，显示不闪错、UI 不卡顿。

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use yinhe_core::YinModel;
use yinhe_types::NoteSource;
use yinhe_wgpu::{InstanceRenderer, NoteInstance};

/// 后台构建的产物：全量过滤后的音符 + per-key offsets + 构建时的 key revisions。
pub(crate) struct BuildResult {
    pub notes: Vec<NoteInstance>,
    pub offsets: [u32; 129],
    pub revisions: [u64; 128],
}

/// Track 显隐后台重建状态机。
///
/// 仅 track_visible 变化（revision/hidden_notes 未变）时进入：
/// `Building`（后台线程 `build_all_notes`）→ `Uploading`（UI 线程分帧上传，
/// 每帧 `KEYS_PER_FRAME` 个 key）→ 完成。期间 `upload_track_mask` 已把当前
/// track_visible 发给 cull shader，旧 buffer 被双重过滤，显示正确。
///
/// revision 或 hidden_notes 变化 → 数据过期，调用方直接丢弃整个 pending
/// （后台线程 send 失败自动退出），后续帧走增量/全量路径。
pub(crate) enum CullRebuild {
    Building {
        rx: Receiver<BuildResult>,
        revision: u64,
        hidden_hash: u64,
        tv_hash: u64,
    },
    Uploading {
        /// 大载荷（全量音符 + offsets + key revisions）装箱，避免 enum 实例膨胀。
        data: Box<UploadData>,
        revision: u64,
        hidden_hash: u64,
        tv_hash: u64,
        next_key: u8,
    },
}

/// 分帧上传阶段持有的载荷。
pub(crate) struct UploadData {
    pub notes: Vec<NoteInstance>,
    pub offsets: [u32; 129],
    pub revisions: [u64; 128],
}

/// 每帧上传的 key 数量。1.64 亿音符全量约 2GB，128 key 分 32 帧传完，
/// 每帧 ~60MB memcpy + tick 索引重建，单帧开销控制在 ~10ms 内。
const KEYS_PER_FRAME: u8 = 4;

impl CullRebuild {
    pub(crate) fn revision(&self) -> u64 {
        match self {
            CullRebuild::Building { revision, .. } | CullRebuild::Uploading { revision, .. } => {
                *revision
            }
        }
    }

    pub(crate) fn hidden_hash(&self) -> u64 {
        match self {
            CullRebuild::Building { hidden_hash, .. }
            | CullRebuild::Uploading { hidden_hash, .. } => *hidden_hash,
        }
    }
}

/// 启动后台全量重建。构建在独立线程执行（`build_all_notes` 内部 rayon 并行），
/// 完成后通过 channel 送回 UI 线程分帧上传。构建线程持有 `model` 的 Arc，
/// 期间模型被替换/关闭时旧数据仍安全（revision 变化会让 pending 被丢弃）。
pub(crate) fn start_rebuild(
    model: Arc<YinModel>,
    hidden_notes: HashSet<(u16, u32, u8)>,
    track_visible: Vec<bool>,
    note_revisions: [u64; 128],
    revision: u64,
    hidden_hash: u64,
    tv_hash: u64,
) -> CullRebuild {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("yinhe-cull-rebuild".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let (notes, offsets) =
                yinhe_wgpu::build_all_notes(model.as_ref(), &hidden_notes, &track_visible);
            let _ = tx.send(BuildResult {
                notes,
                offsets,
                revisions: note_revisions,
            });
        })
        .expect("failed to spawn cull rebuild thread");
    CullRebuild::Building {
        rx,
        revision,
        hidden_hash,
        tv_hash,
    }
}

/// 重建状态机的推进结果。
pub(crate) enum Advance {
    /// 仍在构建/上传中，本帧结束。
    InProgress,
    /// 全部 key 上传完成，携带构建时的 tv_hash（调用方对比当前值决定是否重启）。
    Done(u64),
    /// 后台线程异常退出（数据不完整），调用方应丢弃 pending 走同步路径。
    Failed,
}

/// 推进重建状态机一帧：
/// - `Building`：poll 后台线程的构建结果，收到后本帧立即转入上传。
/// - `Uploading`：上传 `KEYS_PER_FRAME` 个 key（`try_incremental_key_upload`
///   内部 GPU 写与 tick 索引重建并行），全部传完返回 `Done`。
pub(crate) fn advance_rebuild(
    rebuild: &mut CullRebuild,
    pianoroll: &mut InstanceRenderer,
) -> Advance {
    loop {
        match rebuild {
            CullRebuild::Building {
                rx,
                revision,
                hidden_hash,
                tv_hash,
            } => match rx.try_recv() {
                Ok(result) => {
                    *rebuild = CullRebuild::Uploading {
                        data: Box::new(UploadData {
                            notes: result.notes,
                            offsets: result.offsets,
                            revisions: result.revisions,
                        }),
                        revision: *revision,
                        hidden_hash: *hidden_hash,
                        tv_hash: *tv_hash,
                        next_key: 0,
                    };
                    continue; // 本帧立即开始上传
                }
                Err(mpsc::TryRecvError::Empty) => return Advance::InProgress,
                Err(mpsc::TryRecvError::Disconnected) => return Advance::Failed,
            },
            CullRebuild::Uploading {
                data,
                tv_hash,
                next_key,
                ..
            } => {
                let mut n = 0u8;
                while n < KEYS_PER_FRAME && *next_key < 128 {
                    let key = *next_key;
                    let slice = &data.notes[data.offsets[key as usize] as usize
                        ..data.offsets[key as usize + 1] as usize];
                    if !pianoroll.try_incremental_key_upload(
                        key,
                        slice,
                        data.revisions[key as usize],
                    ) {
                        // key buffer 不存在（pending 前必有全量上传，正常不会到）；
                        // 防御：标记完成，让调用方走同步路径。
                        *next_key = 128;
                        break;
                    }
                    *next_key += 1;
                    n += 1;
                }
                if *next_key >= 128 {
                    return Advance::Done(*tv_hash);
                }
                return Advance::InProgress;
            }
        }
    }
}

/// GPU cull 上传所需的状态（含跨帧缓存的 revision/hash）。
pub struct GpuUploadState<'a> {
    pub pianoroll: &'a mut InstanceRenderer,
    pub midi: Option<&'a dyn NoteSource>,
    /// 与 `midi` 同源的 `Arc<YinModel>`，供后台重建线程 clone。
    pub midi_arc: Option<&'a Arc<YinModel>>,
    pub revision: u64,
    pub note_revisions: &'a [u64; 128],
    pub track_visible: &'a [bool],
    pub hidden_notes: &'a HashSet<(u16, u32, u8)>,
    /// 跨帧缓存：上次完整上传的 note_key.value()。变化时触发上传。
    pub last_cull_revision: &'a mut u64,
    /// 跨帧缓存：上次 revision（用于增量检测）。
    pub last_cull_revision_only: &'a mut u64,
    /// 跨帧缓存：上次 hidden_notes hash（用于增量检测）。
    pub last_hidden_hash: &'a mut u64,
    /// 跨帧缓存：上次 track_visible hash（track_mask 变化检测）。
    pub last_tv_hash: &'a mut u64,
    /// 跨帧：track 显隐后台重建状态机（None = 无进行中的重建）。
    pub rebuild: &'a mut Option<CullRebuild>,
}

/// 执行 GPU cull buffer 上传（仅 `use_gpu_cull = true` 时调用）。
pub fn upload(state: GpuUploadState) {
    let GpuUploadState {
        pianoroll,
        midi,
        midi_arc,
        revision,
        note_revisions,
        track_visible,
        hidden_notes,
        last_cull_revision,
        last_cull_revision_only,
        last_hidden_hash,
        last_tv_hash,
        rebuild,
    } = state;

    let tv_hash = yinhe_wgpu::hash_bools(track_visible);
    let hidden_hash = yinhe_wgpu::hash_hidden(hidden_notes);
    let note_key = yinhe_wgpu::NoteBufferKey::new(revision, track_visible, hidden_notes);

    // 1. Track 显隐 mask 同步：任何变化立即上传（~8KB 写入，让 cull shader
    //    立刻过滤隐藏轨道的音符）。mask 始终等于当前 track_visible，与
    //    buffer 数据的「构建时 track_visible 过滤」双重过滤无害。
    if tv_hash != *last_tv_hash {
        pianoroll.upload_track_mask(track_visible);
        *last_tv_hash = tv_hash;
    }

    // 2. 推进（或丢弃）进行中的后台重建。
    if let Some(rb) = rebuild.as_mut() {
        let stale = revision != rb.revision() || hidden_hash != rb.hidden_hash();
        if stale {
            // revision/hidden 变化 → 数据过期，丢弃 pending（后台线程
            // send 失败自动退出），本帧落入下方正常路径处理。
            *rebuild = None;
        } else {
            match advance_rebuild(rb, pianoroll) {
                Advance::InProgress => return, // 还在重建，本帧不做其他上传
                Advance::Done(done_tv) => {
                    *rebuild = None;
                    if tv_hash == done_tv {
                        // 重建期间 track_visible 未再变化：收尾。
                        *last_cull_revision = note_key.value();
                        *last_cull_revision_only = revision;
                        *last_hidden_hash = hidden_hash;
                        return;
                    }
                    // 重建期间 track_visible 又变了：数据基于旧 mask，本帧
                    // 不 return，落入下方正常路径重启（用新 tv_hash）。
                }
                Advance::Failed => {
                    *rebuild = None;
                    // 落入下方正常路径 → 同步全量兜底。
                }
            }
        }
    }

    // If cull isn't ready yet (e.g. just enabled, or MIDI just loaded),
    // force a full upload by invalidating the last revision.
    let cull_was_ready = pianoroll.cull_ready();
    if !cull_was_ready {
        *last_cull_revision = 0;
    }
    if note_key.value() == *last_cull_revision {
        return;
    }

    let Some(midi_src) = midi else {
        *last_cull_revision = note_key.value();
        *last_cull_revision_only = revision;
        *last_hidden_hash = hidden_hash;
        return;
    };

    if !cull_was_ready {
        // First-time upload or MIDI just loaded: force full upload.
        let (all_notes, offsets) =
            yinhe_wgpu::build_all_notes(midi_src, hidden_notes, track_visible);
        pianoroll.upload_all_notes_for_cull(&all_notes, &offsets, note_revisions);
    } else {
        let revision_changed = revision != *last_cull_revision_only;
        let hidden_changed = hidden_hash != *last_hidden_hash;

        if hidden_changed && !revision_changed {
            // Only hidden_notes changed → must full upload
            let (all_notes, offsets) =
                yinhe_wgpu::build_all_notes(midi_src, hidden_notes, track_visible);
            pianoroll.upload_all_notes_for_cull(&all_notes, &offsets, note_revisions);
        } else if revision_changed {
            // Revision changed → try incremental per-key upload
            let uploaded = pianoroll.uploaded_key_revisions();
            let dirty_keys: Vec<u8> = (0u8..128)
                .filter(|&k| note_revisions[k as usize] != uploaded[k as usize])
                .collect();

            if !dirty_keys.is_empty() {
                // Try incremental: build + upload each dirty key
                let mut all_ok = true;
                for &key in &dirty_keys {
                    let key_notes =
                        yinhe_wgpu::build_key_notes(midi_src, key, hidden_notes, track_visible);
                    if !pianoroll.try_incremental_key_upload(
                        key,
                        &key_notes,
                        note_revisions[key as usize],
                    ) {
                        all_ok = false;
                        break;
                    }
                }

                if !all_ok {
                    // Fallback: full upload (some key's count changed)
                    let (all_notes, offsets) =
                        yinhe_wgpu::build_all_notes(midi_src, hidden_notes, track_visible);
                    pianoroll.upload_all_notes_for_cull(&all_notes, &offsets, note_revisions);
                }
            }
            // dirty_keys.is_empty(): revision bumped but no key revisions changed
            // (e.g. conductor-only edit) → 只更新 tracking，不重传。
        } else {
            // Only track_visible changed (note_key differs but revision and
            // hidden_notes are unchanged) → background full rebuild:
            // build on a worker thread, upload incrementally over frames.
            let Some(model) = midi_arc else {
                // 无 Arc 句柄（理论上只有 midi 为 None 时）→ 同步全量兜底。
                let (all_notes, offsets) =
                    yinhe_wgpu::build_all_notes(midi_src, hidden_notes, track_visible);
                pianoroll.upload_all_notes_for_cull(&all_notes, &offsets, note_revisions);
                return;
            };
            *rebuild = Some(start_rebuild(
                Arc::clone(model),
                hidden_notes.clone(),
                track_visible.to_vec(),
                *note_revisions,
                revision,
                hidden_hash,
                tv_hash,
            ));
            // 不更新 last_cull_revision：pending 完成时更新。
            return;
        }
    }

    *last_cull_revision = note_key.value();
    *last_cull_revision_only = revision;
    *last_hidden_hash = hidden_hash;
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_test_helpers::make_stress_model;

    /// Headless GPU renderer for state-machine integration tests.
    /// Returns None when no adapter is available (e.g. CI without a GPU).
    fn headless_renderer() -> Option<InstanceRenderer> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).ok()?;
        Some(InstanceRenderer::new(
            device,
            queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ))
    }

    /// 后台构建产物必须与同步全量构建一致（含 track_visible 过滤），
    /// 否则分帧上传会写进错误的数据。
    #[test]
    fn rebuild_build_matches_sync_build() {
        let model = Arc::new(make_stress_model(4, 2000));
        let hidden = HashSet::new();
        let tv = vec![true, false, true, true]; // 隐藏轨道 1
        let mut revisions = [0u64; 128];
        for (i, r) in revisions.iter_mut().enumerate() {
            *r = i as u64 + 1;
        }

        let (sync_notes, sync_offsets) = yinhe_wgpu::build_all_notes(model.as_ref(), &hidden, &tv);
        let mut rb = start_rebuild(model, hidden, tv, revisions, 42, 7, 9);
        let result = match &mut rb {
            CullRebuild::Building { rx, .. } => match rx.recv() {
                Ok(r) => r,
                Err(_) => panic!("rebuild thread send failed"),
            },
            _ => panic!("unexpected variant"),
        };
        assert_eq!(result.notes, sync_notes);
        assert_eq!(result.offsets, sync_offsets);
        assert_eq!(result.revisions, revisions);
    }

    /// 完整状态机：Building → 分帧上传（每帧 KEYS_PER_FRAME 个 key）→ Done，
    /// 最终所有 key 的 uploaded_key_revisions 都推进到构建时的值。
    #[test]
    fn rebuild_upload_state_machine_roundtrip() {
        let Some(mut renderer) = headless_renderer() else {
            return;
        };
        let model = Arc::new(make_stress_model(4, 2000));
        let hidden = HashSet::new();
        let tv = vec![true, false, true, true];
        let mut revisions = [0u64; 128];
        for (i, r) in revisions.iter_mut().enumerate() {
            *r = i as u64 + 1;
        }

        // 首帧全量上传（模拟初次加载：128 个 key 都有 GPU buffer）。
        let (all_notes, offsets) = yinhe_wgpu::build_all_notes(model.as_ref(), &hidden, &tv);
        renderer.upload_all_notes_for_cull(&all_notes, &offsets, &revisions);

        // 模拟 track_visible 变化 → 启动后台重建（tv_hash = 99）。
        let tv2 = vec![true, true, true, true];
        let mut rb = start_rebuild(model, hidden, tv2, revisions, 42, 7, 99);

        let mut guard = 0u32;
        let done_tv = loop {
            match advance_rebuild(&mut rb, &mut renderer) {
                Advance::Done(tv) => break tv,
                Advance::Failed => panic!("rebuild failed"),
                Advance::InProgress => {
                    guard += 1;
                    assert!(guard < 10_000, "状态机推进 10000 次仍未完成");
                    std::thread::yield_now();
                }
            }
        };
        assert_eq!(done_tv, 99);
        // 所有 key 都已按构建时数据重新上传。
        assert_eq!(*renderer.uploaded_key_revisions(), revisions);
    }
}
