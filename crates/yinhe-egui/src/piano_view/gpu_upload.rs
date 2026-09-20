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
use yinhe_types::{KEY_COUNT, NoteSource};
use yinhe_wgpu::{InstanceRenderer, NoteInstance};

/// 后台构建的产物：全量过滤后的音符 + per-key offsets + 构建时的 key revisions
/// + LOD 摘要（每档 `SUMMARY_BLOCK_TICKS` 一份）。
pub(crate) struct BuildResult {
    pub notes: Vec<NoteInstance>,
    pub offsets: [u32; KEY_COUNT + 1],
    pub revisions: [u64; KEY_COUNT],
    pub summaries: Vec<Option<(Vec<NoteInstance>, [u32; KEY_COUNT + 1])>>,
}

/// 构建摘要到 `max_level`（含；`None` = 不构建任何档）；更细的档位留 `None`
/// 由懒构建（1C）在首次被选中时生成。不设段数上限：细档（16）必须可用；
/// 显存由 `GpuBudget` 兜底。
pub(crate) fn build_summaries(
    notes: &[NoteInstance],
    offsets: &[u32; KEY_COUNT + 1],
    max_level: Option<usize>,
) -> Vec<Option<(Vec<NoteInstance>, [u32; KEY_COUNT + 1])>> {
    match max_level {
        Some(level) => yinhe_wgpu::build_summaries_up_to(notes, offsets, level),
        None => (0..yinhe_wgpu::SUMMARY_BLOCK_TICKS.len())
            .map(|_| None)
            .collect(),
    }
}

/// 单 key 的全档位摘要（编辑增量路径）。
fn build_key_summaries(key: u8, notes: &[NoteInstance]) -> Vec<Vec<NoteInstance>> {
    yinhe_wgpu::SUMMARY_BLOCK_TICKS
        .iter()
        .map(|&block| yinhe_wgpu::build_key_summary(key, notes, block))
        .collect()
}

/// 全量上传音符 + 摘要（保证两层数据来自同一次构建）。
///
/// 更细的懒档基于旧状态构建、已过期：丢弃进行中的懒构建并清空（下次选中
/// 时用新状态重建）。
pub(crate) fn upload_all_with_summaries(
    pianoroll: &mut InstanceRenderer,
    notes: &[NoteInstance],
    offsets: &[u32; KEY_COUNT + 1],
    revisions: &[u64; KEY_COUNT],
    max_level: Option<usize>,
    summary: &mut SummaryLoadState,
) {
    summary.load = None;
    pianoroll.set_summary_loading(None);
    pianoroll.clear_summaries_above(max_level);
    let summaries = build_summaries(notes, offsets, max_level);
    pianoroll.upload_all_notes_for_cull(notes, offsets, revisions);
    pianoroll.upload_summary_for_cull(&summaries);
}

/// 单 key 增量上传音符 + 摘要。返回 false 表示需回退全量（key 或摘要层缺失）。
pub(crate) fn upload_key_with_summaries(
    pianoroll: &mut InstanceRenderer,
    key: u8,
    notes: &[NoteInstance],
    revision: u64,
) -> bool {
    if !pianoroll.try_incremental_key_upload(key, notes, revision) {
        return false;
    }
    for (level, summary) in build_key_summaries(key, notes).iter().enumerate() {
        if !pianoroll.try_incremental_summary_key(level, key, summary) {
            return false;
        }
    }
    true
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
    pub offsets: [u32; KEY_COUNT + 1],
    pub revisions: [u64; KEY_COUNT],
    pub summaries: Vec<Option<(Vec<NoteInstance>, [u32; KEY_COUNT + 1])>>,
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

    /// 构建/上传对应的 track_visible hash。
    pub(crate) fn tv_hash(&self) -> u64 {
        match self {
            CullRebuild::Building { tv_hash, .. } | CullRebuild::Uploading { tv_hash, .. } => {
                *tv_hash
            }
        }
    }
}

/// 启动后台全量重建。构建在独立线程执行（`build_all_notes` 内部 rayon 并行），
/// 完成后通过 channel 送回 UI 线程分帧上传。构建线程持有 `model` 的 Arc，
/// 期间模型被替换/关闭时旧数据仍安全（revision 变化会让 pending 被丢弃）。
#[allow(clippy::too_many_arguments)] // 后台重建快照参数，见 AGENTS 约定
pub(crate) fn start_rebuild(
    model: Arc<YinModel>,
    hidden_notes: HashSet<(u16, u32, u8)>,
    track_visible: Vec<bool>,
    note_revisions: [u64; KEY_COUNT],
    revision: u64,
    hidden_hash: u64,
    tv_hash: u64,
    max_level: Option<usize>,
) -> CullRebuild {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("yinhe-cull-rebuild".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let (notes, offsets) =
                yinhe_wgpu::build_all_notes(model.as_ref(), &hidden_notes, &track_visible);
            let summaries = build_summaries(&notes, &offsets, max_level);
            let _ = tx.send(BuildResult {
                notes,
                offsets,
                revisions: note_revisions,
                summaries,
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

/// 懒构建（1C）的单个摘要档位载荷。
pub(crate) struct SummaryLevelData {
    pub notes: Vec<NoteInstance>,
    pub offsets: [u32; KEY_COUNT + 1],
}

/// 后台线程送回的懒档构建结果。
pub(crate) struct SummaryLevelResult {
    pub level: usize,
    pub data: SummaryLevelData,
}

/// 懒构建档位的状态机：后台构建整档 → 分帧上传 → 就绪。
///
/// 构建/上传期间 revision/hidden/track_visible 变化 → 数据过期，作废并
/// 清空该档（调用方随后按新状态重新请求）。
pub(crate) enum SummaryLevelLoad {
    Building {
        level: usize,
        rx: Receiver<SummaryLevelResult>,
        revision: u64,
        hidden_hash: u64,
        tv_hash: u64,
    },
    Uploading {
        level: usize,
        data: Box<SummaryLevelData>,
        next_key: u8,
        revision: u64,
        hidden_hash: u64,
        tv_hash: u64,
    },
}

/// 懒构建状态机的推进结果。
pub(crate) enum SummaryAdvance {
    InProgress,
    Done,
    /// 显存预算等真错误（该档已清空；调用方应记入失败黑名单）。
    Failed(usize),
    /// 数据过期或目标切换（该档已清空；可按新状态重新请求）。
    Stale,
}

/// 懒构建档位的跨帧状态（App 持有）。
pub struct SummaryLoadState {
    pub(crate) load: Option<SummaryLevelLoad>,
    /// 本轮目标档下构建失败的档位（显存预算不足），避免无限重试。
    pub(crate) failed: [bool; yinhe_wgpu::SUMMARY_BLOCK_TICKS.len()],
    pub(crate) last_target: Option<usize>,
}

impl Default for SummaryLoadState {
    fn default() -> Self {
        Self {
            load: None,
            failed: [false; yinhe_wgpu::SUMMARY_BLOCK_TICKS.len()],
            last_target: None,
        }
    }
}

/// 启动单档懒构建：后台重新构建 all_notes 并聚合该档（整曲）。
fn start_summary_level_build(
    model: Arc<YinModel>,
    hidden_notes: HashSet<(u16, u32, u8)>,
    track_visible: Vec<bool>,
    level: usize,
    revision: u64,
    hidden_hash: u64,
    tv_hash: u64,
) -> SummaryLevelLoad {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("yinhe-cull-summary".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let (notes, offsets) =
                yinhe_wgpu::build_all_notes(model.as_ref(), &hidden_notes, &track_visible);
            let block = yinhe_wgpu::SUMMARY_BLOCK_TICKS[level];
            let (snotes, soffsets) = yinhe_wgpu::build_summary(&notes, &offsets, block);
            let _ = tx.send(SummaryLevelResult {
                level,
                data: SummaryLevelData {
                    notes: snotes,
                    offsets: soffsets,
                },
            });
        })
        .expect("failed to spawn cull summary thread");
    SummaryLevelLoad::Building {
        level,
        rx,
        revision,
        hidden_hash,
        tv_hash,
    }
}

/// 推进懒构建状态机一帧（每帧最多上传 `KEYS_PER_FRAME` 个 key）。
///
/// `target` = 当前 ppu 需要的档：目标切换（缩放）时立即作废进行中的档，
/// 只构建最终停留的档。
pub(crate) fn advance_summary_level(
    load: &mut SummaryLevelLoad,
    pianoroll: &mut InstanceRenderer,
    revision: u64,
    hidden_hash: u64,
    tv_hash: u64,
    target: Option<usize>,
) -> SummaryAdvance {
    loop {
        match load {
            SummaryLevelLoad::Building {
                level,
                rx,
                revision: r,
                hidden_hash: h,
                tv_hash: t,
            } => {
                if target != Some(*level) || revision != *r || hidden_hash != *h || tv_hash != *t {
                    return SummaryAdvance::Stale;
                }
                match rx.try_recv() {
                    Ok(result) => {
                        if result.level != *level {
                            return SummaryAdvance::Stale;
                        }
                        pianoroll.set_summary_loading(Some(*level));
                        *load = SummaryLevelLoad::Uploading {
                            level: *level,
                            data: Box::new(result.data),
                            next_key: 0,
                            revision: *r,
                            hidden_hash: *h,
                            tv_hash: *t,
                        };
                        continue;
                    }
                    Err(mpsc::TryRecvError::Empty) => return SummaryAdvance::InProgress,
                    // 后台线程异常退出（发送失败）：按过期处理，允许重试。
                    // 后台线程异常退出（发送失败）：按过期处理，允许重试。
                    Err(mpsc::TryRecvError::Disconnected) => return SummaryAdvance::Stale,
                }
            }
            SummaryLevelLoad::Uploading {
                level,
                data,
                next_key,
                revision: r,
                hidden_hash: h,
                tv_hash: t,
            } => {
                if target != Some(*level) || revision != *r || hidden_hash != *h || tv_hash != *t {
                    pianoroll.clear_summary_level(*level);
                    return SummaryAdvance::Stale;
                }
                let mut n = 0u8;
                // MIDI key 范围 0..=127（KEY_COUNT=256 是 buffer 容量）。
                while n < KEYS_PER_FRAME && *next_key < 128 {
                    let key = *next_key;
                    let lo = data.offsets[key as usize] as usize;
                    let hi = data.offsets[key as usize + 1] as usize;
                    if !pianoroll.upload_summary_level_key(*level, key, &data.notes[lo..hi]) {
                        let level = *level;
                        pianoroll.clear_summary_level(level);
                        return SummaryAdvance::Failed(level);
                    }
                    *next_key += 1;
                    n += 1;
                }
                if *next_key >= 128 {
                    pianoroll.set_summary_loading(None);
                    return SummaryAdvance::Done;
                }
                return SummaryAdvance::InProgress;
            }
        }
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
                            summaries: result.summaries,
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
                    let lo = data.offsets[key as usize] as usize;
                    let hi = data.offsets[key as usize + 1] as usize;
                    if !pianoroll.try_incremental_key_upload(
                        key,
                        &data.notes[lo..hi],
                        data.revisions[key as usize],
                    ) {
                        // 真错误（显存预算失败等）：数据不完整，必须回退全量，
                        // 不能提前标记完成（否则该 key 及其后所有 key 静默缺失）。
                        return Advance::Failed;
                    }
                    // 摘要层随音符同步增量（全量上传已建立各档）。
                    let mut ok = true;
                    for (level, summary) in data.summaries.iter().enumerate() {
                        let Some((summary_notes, summary_offsets)) = summary else {
                            continue;
                        };
                        let slo = summary_offsets[key as usize] as usize;
                        let shi = summary_offsets[key as usize + 1] as usize;
                        if !pianoroll.try_incremental_summary_key(
                            level,
                            key,
                            &summary_notes[slo..shi],
                        ) {
                            ok = false;
                            break;
                        }
                    }
                    if !ok {
                        // 摘要层缺失/写入失败：摘要数据会与音符数据脱节，
                        // 不能静默继续（渲染会用到过期摘要）。回退全量。
                        return Advance::Failed;
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
    pub note_revisions: &'a [u64; KEY_COUNT],
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
    /// 跨帧缓存：上次上传时 hidden_notes 的 key 位图（hidden 增量重建的
    /// 受影响 key 判定：当前 ∪ 上次 = 需重建的 key 并集）。
    pub last_hidden_keys: &'a mut HiddenKeyMask,
    /// 跨帧：track 显隐后台重建状态机（None = 无进行中的重建）。
    pub rebuild: &'a mut Option<CullRebuild>,
    /// 当前 ppu 对应的摘要目标档（`None` = 原始层区间，无需摘要）。
    pub summary_target: Option<usize>,
    /// 跨帧：更细档的懒构建状态（1C）。
    pub summary: &'a mut SummaryLoadState,
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
        last_hidden_keys,
        rebuild,
        summary_target,
        summary,
    } = state;

    let tv_hash = yinhe_wgpu::hash_bools(track_visible);
    let hidden_hash = yinhe_wgpu::hash_hidden(hidden_notes);
    let note_key = yinhe_wgpu::NoteBufferKey::new(revision, track_visible, hidden_notes);

    // 0. LOD 细档懒构建（1C）：目标档变化时重置失败黑名单；推进进行中的
    //    构建/上传；无进行中构建且目标档不可用时启动后台构建。
    if summary.last_target != summary_target {
        summary.failed.fill(false);
        summary.last_target = summary_target;
    }
    if let Some(load) = summary.load.as_mut() {
        match advance_summary_level(
            load,
            pianoroll,
            revision,
            hidden_hash,
            tv_hash,
            summary_target,
        ) {
            SummaryAdvance::InProgress => {}
            SummaryAdvance::Done => summary.load = None,
            SummaryAdvance::Failed(level) => {
                summary.load = None;
                summary.failed[level] = true;
                tracing::error!(
                    "[cull] LOD 档 {level} 懒构建上传失败（显存预算不足），回退到更粗档"
                );
            }
            SummaryAdvance::Stale => summary.load = None,
        }
    }
    match (
        summary.load.is_none() && rebuild.is_none(),
        summary_target,
        midi_arc,
    ) {
        (true, Some(target), Some(model))
            if !summary.failed[target] && !pianoroll.summary_level_ready(target) =>
        {
            summary.load = Some(start_summary_level_build(
                Arc::clone(model),
                hidden_notes.clone(),
                track_visible.to_vec(),
                target,
                revision,
                hidden_hash,
                tv_hash,
            ));
        }
        _ => {}
    }

    // 1. Track 显隐 mask 同步：任何变化立即上传（~8KB 写入，让 cull shader
    //    立刻过滤隐藏轨道的音符）。mask 始终等于当前 track_visible，与
    //    buffer 数据的「构建时 track_visible 过滤」双重过滤无害。
    if tv_hash != *last_tv_hash {
        pianoroll.upload_track_mask(track_visible);
        *last_tv_hash = tv_hash;
    }

    // 2. 推进（或丢弃）进行中的后台重建。
    if let Some(rb) = rebuild.as_mut() {
        // track_visible 变化也必须丢弃：旧重建的数据基于旧 tv，继续上传
        // 会把 GPU 上「上次完整上传」的数据从 key 0 开始逐 key 覆盖成错误
        // 内容（表现为从下向上隐去），且完成后 note_key 可能仍等于
        // last_cull_revision 导致永不恢复。
        let stale =
            revision != rb.revision() || hidden_hash != rb.hidden_hash() || tv_hash != rb.tv_hash();
        if stale {
            // 丢弃 pending（后台线程 send 失败自动退出），并强制失效
            // last_cull_revision：GPU 数据可能已被旧重建部分污染，必须让
            // 本帧落入下方正常路径重新评估（启动基于当前 tv 的重建）。
            *rebuild = None;
            *last_cull_revision = 0;
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
                        *last_hidden_keys = hidden_key_mask(hidden_notes);
                        return;
                    }
                    // 重建数据基于旧 tv（期间切过轨）：GPU 已被旧 tv 数据
                    // 替换，强制本帧重新评估（用新 tv 重启重建）。
                    *last_cull_revision = 0;
                }
                Advance::Failed => {
                    *rebuild = None;
                    // 数据不完整：直接同步全量兜底（只清 last_cull_revision
                    // 会再次落入 tv 变化分支启动重建，失败时形成无限重启）。
                    if let Some(midi_src) = midi {
                        let (all_notes, offsets) =
                            yinhe_wgpu::build_all_notes(midi_src, hidden_notes, track_visible);
                        upload_all_with_summaries(
                            pianoroll,
                            &all_notes,
                            &offsets,
                            note_revisions,
                            summary_target,
                            summary,
                        );
                        *last_cull_revision = note_key.value();
                        *last_cull_revision_only = revision;
                        *last_hidden_hash = hidden_hash;
                        *last_hidden_keys = hidden_key_mask(hidden_notes);
                    } else {
                        *last_cull_revision = 0;
                    }
                    return;
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
    // cull 未 ready 时即使 note_key == 0（初始 last_cull_revision）也绝不早退：
    // hash_bools([true]) == 1 时单轨首帧 note_key = revision(1) ^ 1 ^ 0 = 0，
    // 0 == 0 碰撞会把「强制失效」抵消掉，导致首帧跳过全量上传而空屏。
    if cull_was_ready && note_key.value() == *last_cull_revision {
        return;
    }

    let Some(midi_src) = midi else {
        *last_cull_revision = note_key.value();
        *last_cull_revision_only = revision;
        *last_hidden_hash = hidden_hash;
        *last_hidden_keys = hidden_key_mask(hidden_notes);
        return;
    };

    if !cull_was_ready {
        // First-time upload or MIDI just loaded: force full upload.
        let (all_notes, offsets) =
            yinhe_wgpu::build_all_notes(midi_src, hidden_notes, track_visible);
        upload_all_with_summaries(
            pianoroll,
            &all_notes,
            &offsets,
            note_revisions,
            summary_target,
            summary,
        );
    } else {
        let revision_changed = revision != *last_cull_revision_only;
        let hidden_changed = hidden_hash != *last_hidden_hash;

        if hidden_changed && !revision_changed {
            // Only hidden_notes changed → rebuild only the affected keys.
            // 受影响 key = 当前 hidden ∪ 上次 hidden 的 key 并集：并集外的
            // key 在两种 hidden 下的过滤输出逐字节相同，无需重建。
            // （拖拽按下/取消时 hidden 变化但 revision 不动，旧实现会同步
            // 全量重建——亿级音符下数百 ms 冻结；这里降到 O(受影响 key)。）
            let cur_mask = hidden_key_mask(hidden_notes);
            let affected = mask_union(&cur_mask, last_hidden_keys);
            let mut all_ok = true;
            for key in 0u8..=yinhe_types::MAX_KEY {
                if !mask_contains(&affected, key) {
                    continue;
                }
                let key_notes =
                    yinhe_wgpu::build_key_notes(midi_src, key, hidden_notes, track_visible);
                if !upload_key_with_summaries(
                    pianoroll,
                    key,
                    &key_notes,
                    note_revisions[key as usize],
                ) {
                    all_ok = false;
                    break;
                }
            }

            if !all_ok {
                // Fallback: full upload (some key's buffer was never created).
                let (all_notes, offsets) =
                    yinhe_wgpu::build_all_notes(midi_src, hidden_notes, track_visible);
                upload_all_with_summaries(
                    pianoroll,
                    &all_notes,
                    &offsets,
                    note_revisions,
                    summary_target,
                    summary,
                );
            }
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
                    if !upload_key_with_summaries(
                        pianoroll,
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
                    upload_all_with_summaries(
                        pianoroll,
                        &all_notes,
                        &offsets,
                        note_revisions,
                        summary_target,
                        summary,
                    );
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
                upload_all_with_summaries(
                    pianoroll,
                    &all_notes,
                    &offsets,
                    note_revisions,
                    summary_target,
                    summary,
                );
                return;
            };
            // 懒档数据基于旧 track_visible：作废并清掉更细档（重建只覆盖
            // 0..=目标档）。
            summary.load = None;
            pianoroll.set_summary_loading(None);
            pianoroll.clear_summaries_above(summary_target);
            *rebuild = Some(start_rebuild(
                Arc::clone(model),
                hidden_notes.clone(),
                track_visible.to_vec(),
                *note_revisions,
                revision,
                hidden_hash,
                tv_hash,
                summary_target,
            ));
            // 不更新 last_cull_revision：pending 完成时更新。
            return;
        }
    }

    *last_cull_revision = note_key.value();
    *last_cull_revision_only = revision;
    *last_hidden_hash = hidden_hash;
    *last_hidden_keys = hidden_key_mask(hidden_notes);
}

/// hidden_notes 集合的 key 位图（bit k = key k 有 hidden 音符）。
/// 固定 4×u64 覆盖 KEY_COUNT 个 key，零堆分配。
pub(crate) type HiddenKeyMask = [u64; KEY_COUNT / 64];

/// 用于 hidden 增量重建的受影响 key 判定。
fn hidden_key_mask(hidden_notes: &std::collections::HashSet<(u16, u32, u8)>) -> HiddenKeyMask {
    let mut mask = [0u64; KEY_COUNT / 64];
    for &(_, _, key) in hidden_notes {
        let k = key as usize;
        mask[k / 64] |= 1u64 << (k % 64);
    }
    mask
}

/// 位图按位或（a ∪ b）。
fn mask_union(a: &HiddenKeyMask, b: &HiddenKeyMask) -> HiddenKeyMask {
    core::array::from_fn(|i| a[i] | b[i])
}

/// bit k 是否置位。
fn mask_contains(mask: &HiddenKeyMask, key: u8) -> bool {
    let k = key as usize;
    mask[k / 64] & (1u64 << (k % 64)) != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
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

    /// 渲染一帧并统计非空像素数（有无音符的粗略判断）。
    /// 返回 (蓝像素, 红像素)——track 0 蓝色、track 1 红色，用于区分显示内容。
    fn render_pixel_count(
        renderer: &mut InstanceRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::Texture,
        view: &wgpu::TextureView,
        pw: u32,
        ph: u32,
    ) -> (u64, u64) {
        // 与真实 UI 一致：先构建 uniforms（PR 视口）再渲染。
        let view_data = yinhe_types::PianoRollView {
            key_height: 20.0,
            viewport_h: ph as f32,
            orientation: yinhe_types::Orientation::Horizontal,
            base: yinhe_types::TimelineViewBase {
                pixels_per_tick: 0.1,
                scroll_x: 0.0,
                scroll_y: 2000.0, // 让 key 0..28 进入视口（音符分布在 key 0..61）
                left_panel_width: 60.0,
                dirty: true,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
                follow_target: None,
                follow_anim_start: 0.0,
                follow_anim_elapsed: 0.0,
            },
        };
        let track_colors: [[f32; 4]; 2] = [[0.2, 0.7, 1.0, 1.0], [0.9, 0.3, 0.3, 1.0]];
        let job = yinhe_wgpu::build_render_job(
            pw,
            ph,
            &view_data,
            &yinhe_core::Selection::default(),
            &track_colors,
            0.0,
            false,
        );
        renderer.upload_uniforms(job.uniforms);
        renderer.upload_track_colors(&job.track_colors);
        renderer.upload_selection(&job.selection);

        let mut enc = device.create_command_encoder(&Default::default());
        renderer.draw(&mut enc, view, pw, ph);
        queue.submit([enc.finish()]);

        let bytes_per_row = pw * 4;
        let aligned_row = bytes_per_row.div_ceil(256) * 256;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("px"),
            size: (aligned_row * ph) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(aligned_row),
                    rows_per_image: Some(ph),
                },
            },
            wgpu::Extent3d {
                width: pw,
                height: ph,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([enc.finish()]);
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done2 = done.clone();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |_| {
            done2.store(true, Ordering::SeqCst);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll failed");
        assert!(done.load(Ordering::SeqCst));
        let mapped = buffer.slice(..).get_mapped_range().expect("readback map");
        let mut blue = 0u64;
        let mut red = 0u64;
        for row in 0..ph {
            let start = (row as usize) * aligned_row as usize;
            let row_data = &mapped[start..start + bytes_per_row as usize];
            for p in row_data.chunks_exact(4) {
                if p[0] > 8 || p[1] > 8 || p[2] > 8 {
                    if p[2] > p[0] {
                        blue += 1; // track 0 蓝色（B 通道大）
                    } else {
                        red += 1; // track 1 红色（R 通道大）
                    }
                }
            }
        }
        drop(mapped);
        buffer.unmap();
        (blue, red)
    }

    /// 切轨流程回归（「从下向上隐去」bug）：切轨后旧轨道数据必须立即被
    /// mask 过滤（中间态显示空而非旧数据逐 key 消失），后台重建完成后
    /// 新轨道数据恢复显示。
    #[test]
    fn track_switch_rebuild_masks_old_track_immediately() {
        let Some((device, queue)) = (|| {
            let instance = wgpu::Instance::default();
            let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
            let (device, queue) =
                pollster::block_on(adapter.request_device(&Default::default())).ok()?;
            Some((device, queue))
        })() else {
            return;
        };
        let mut renderer = InstanceRenderer::new(
            device.clone(),
            queue.clone(),
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        let pw = 800u32;
        let ph = 600u32;
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("switch_target"),
            size: wgpu::Extent3d {
                width: pw,
                height: ph,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&Default::default());

        // 双轨模型：track 0 与 track 1 各有 1000 音符（key = n % 128）。
        let model = Arc::new(make_stress_model(2, 1000));
        let hidden = HashSet::new();
        let note_revisions = model.note_revisions;

        let mut last_cull_revision = 0u64;
        let mut last_cull_revision_only = 0u64;
        let mut last_hidden_hash = 0u64;
        let mut last_tv_hash = 0u64;
        let mut last_hidden_keys: HiddenKeyMask = [0; KEY_COUNT / 64];
        let mut rebuild: Option<CullRebuild> = None;
        let mut summary = SummaryLoadState::default();

        // 首帧：只显示 track 0（模拟打开 Master 轨）。
        let tv0 = vec![true, false];
        upload(GpuUploadState {
            pianoroll: &mut renderer,
            midi: Some(model.as_ref() as &dyn NoteSource),
            midi_arc: Some(&model),
            revision: 1,
            note_revisions: &note_revisions,
            track_visible: &tv0,
            hidden_notes: &hidden,
            last_cull_revision: &mut last_cull_revision,
            last_cull_revision_only: &mut last_cull_revision_only,
            last_hidden_hash: &mut last_hidden_hash,
            last_tv_hash: &mut last_tv_hash,
            last_hidden_keys: &mut last_hidden_keys,
            rebuild: &mut rebuild,
            summary_target: None,
            summary: &mut summary,
        });
        let px0 = render_pixel_count(
            &mut renderer,
            &device,
            &queue,
            &target,
            &target_view,
            pw,
            ph,
        );
        assert!(
            px0.0 > 0 && px0.1 == 0,
            "首次加载应显示 track 0 音符: {px0:?}"
        );

        // 切轨：只显示 track 1 → 旧数据（track 0）必须立即被 mask 过滤。
        let tv1 = vec![false, true];
        upload(GpuUploadState {
            pianoroll: &mut renderer,
            midi: Some(model.as_ref() as &dyn NoteSource),
            midi_arc: Some(&model),
            revision: 1,
            note_revisions: &note_revisions,
            track_visible: &tv1,
            hidden_notes: &hidden,
            last_cull_revision: &mut last_cull_revision,
            last_cull_revision_only: &mut last_cull_revision_only,
            last_hidden_hash: &mut last_hidden_hash,
            last_tv_hash: &mut last_tv_hash,
            last_hidden_keys: &mut last_hidden_keys,
            rebuild: &mut rebuild,
            summary_target: None,
            summary: &mut summary,
        });
        let px1 = render_pixel_count(
            &mut renderer,
            &device,
            &queue,
            &target,
            &target_view,
            pw,
            ph,
        );
        assert_eq!(
            px1,
            (0, 0),
            "切轨后旧轨道数据必须立即被 mask 过滤（而不是逐 key 隐去）: {px1:?}"
        );

        // 推进几帧，让后台重建进入 Uploading 并上传部分 track 1 数据。
        for _ in 0..4 {
            upload(GpuUploadState {
                pianoroll: &mut renderer,
                midi: Some(model.as_ref() as &dyn NoteSource),
                midi_arc: Some(&model),
                revision: 1,
                note_revisions: &note_revisions,
                track_visible: &tv1,
                hidden_notes: &hidden,
                last_cull_revision: &mut last_cull_revision,
                last_cull_revision_only: &mut last_cull_revision_only,
                last_hidden_hash: &mut last_hidden_hash,
                last_tv_hash: &mut last_tv_hash,
                last_hidden_keys: &mut last_hidden_keys,
                rebuild: &mut rebuild,
                summary_target: None,
                summary: &mut summary,
            });
        }
        let mid = render_pixel_count(
            &mut renderer,
            &device,
            &queue,
            &target,
            &target_view,
            pw,
            ph,
        );
        assert!(mid.1 > 0, "track 1 数据应已部分显示: {mid:?}");

        // 快速切回 track 0（重建 A 尚未完成）：「从下向上隐去」bug 复现点。
        // 修复前：pending 重建 A 继续上传 track 1 数据 → 红色像素扩散；
        // 修复后：pending 被丢弃（tv 变化），重建 B 上传 track 0 → 红色递减。
        let tv0b = vec![true, false];
        upload(GpuUploadState {
            pianoroll: &mut renderer,
            midi: Some(model.as_ref() as &dyn NoteSource),
            midi_arc: Some(&model),
            revision: 1,
            note_revisions: &note_revisions,
            track_visible: &tv0b,
            hidden_notes: &hidden,
            last_cull_revision: &mut last_cull_revision,
            last_cull_revision_only: &mut last_cull_revision_only,
            last_hidden_hash: &mut last_hidden_hash,
            last_tv_hash: &mut last_tv_hash,
            last_hidden_keys: &mut last_hidden_keys,
            rebuild: &mut rebuild,
            summary_target: None,
            summary: &mut summary,
        });
        let mut reds = Vec::new();
        for _ in 0..4 {
            upload(GpuUploadState {
                pianoroll: &mut renderer,
                midi: Some(model.as_ref() as &dyn NoteSource),
                midi_arc: Some(&model),
                revision: 1,
                note_revisions: &note_revisions,
                track_visible: &tv0b,
                hidden_notes: &hidden,
                last_cull_revision: &mut last_cull_revision,
                last_cull_revision_only: &mut last_cull_revision_only,
                last_hidden_hash: &mut last_hidden_hash,
                last_tv_hash: &mut last_tv_hash,
                last_hidden_keys: &mut last_hidden_keys,
                rebuild: &mut rebuild,
                summary_target: None,
                summary: &mut summary,
            });
            reds.push(
                render_pixel_count(
                    &mut renderer,
                    &device,
                    &queue,
                    &target,
                    &target_view,
                    pw,
                    ph,
                )
                .1,
            );
        }
        assert!(
            reds.windows(2).all(|w| w[1] <= w[0] + 2),
            "切回后 track 1 数据不得继续扩散（从下向上隐去 bug）: {reds:?}"
        );

        // 推进后台重建直到完成（每帧 upload 一次，模拟真实帧循环）。
        let mut guard = 0u32;
        while rebuild.is_some() {
            upload(GpuUploadState {
                pianoroll: &mut renderer,
                midi: Some(model.as_ref() as &dyn NoteSource),
                midi_arc: Some(&model),
                revision: 1,
                note_revisions: &note_revisions,
                track_visible: &tv0b,
                hidden_notes: &hidden,
                last_cull_revision: &mut last_cull_revision,
                last_cull_revision_only: &mut last_cull_revision_only,
                last_hidden_hash: &mut last_hidden_hash,
                last_tv_hash: &mut last_tv_hash,
                last_hidden_keys: &mut last_hidden_keys,
                rebuild: &mut rebuild,
                summary_target: None,
                summary: &mut summary,
            });
            guard += 1;
            assert!(guard < 100, "后台重建未在 100 帧内完成");
        }
        // 重建完成后：track 0 数据全部上传 → 显示恢复。
        let px2 = render_pixel_count(
            &mut renderer,
            &device,
            &queue,
            &target,
            &target_view,
            pw,
            ph,
        );
        assert!(
            px2.0 > 0 && px2.1 == 0,
            "重建完成后应显示 track 0 音符: {px2:?}"
        );
    }

    /// 后台构建产物必须与同步全量构建一致（含 track_visible 过滤），
    /// 否则分帧上传会写进错误的数据。
    #[test]
    fn rebuild_build_matches_sync_build() {
        let model = Arc::new(make_stress_model(4, 2000));
        let hidden = HashSet::new();
        let tv = vec![true, false, true, true]; // 隐藏轨道 1
        let mut revisions = [0u64; KEY_COUNT];
        for (i, r) in revisions.iter_mut().enumerate() {
            *r = i as u64 + 1;
        }

        let (sync_notes, sync_offsets) = yinhe_wgpu::build_all_notes(model.as_ref(), &hidden, &tv);
        let mut rb = start_rebuild(model, hidden, tv, revisions, 42, 7, 9, Some(0));
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
        assert_eq!(
            result.summaries.len(),
            yinhe_wgpu::SUMMARY_BLOCK_TICKS.len(),
            "后台构建必须产出全部摘要档位"
        );
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
        let mut revisions = [0u64; KEY_COUNT];
        for (i, r) in revisions.iter_mut().enumerate() {
            *r = i as u64 + 1;
        }

        // 首帧全量上传（模拟初次加载：128 个 key 都有 GPU buffer + 摘要层）。
        let (all_notes, offsets) = yinhe_wgpu::build_all_notes(model.as_ref(), &hidden, &tv);
        upload_all_with_summaries(
            &mut renderer,
            &all_notes,
            &offsets,
            &revisions,
            None,
            &mut SummaryLoadState::default(),
        );

        // 模拟 track_visible 变化 → 启动后台重建（tv_hash = 99）。
        let tv2 = vec![true, true, true, true];
        let mut rb = start_rebuild(model, hidden, tv2, revisions, 42, 7, 99, Some(0));

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

    /// hidden 变化（拖拽按下/取消）走按 key 增量重建：受影响 key 的音符
    /// 立即隐藏、取消后逐像素恢复（与首帧全量上传一致）；未受影响 key 的
    /// GPU 数据保持不变（否则恢复帧不会逐像素相等）。
    #[test]
    fn hidden_change_incremental_rebuild() {
        let Some((device, queue)) = (|| {
            let instance = wgpu::Instance::default();
            let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
            let (device, queue) =
                pollster::block_on(adapter.request_device(&Default::default())).ok()?;
            Some((device, queue))
        })() else {
            return;
        };
        let mut renderer = InstanceRenderer::new(
            device.clone(),
            queue.clone(),
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        let pw = 800u32;
        let ph = 600u32;
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hidden_target"),
            size: wgpu::Extent3d {
                width: pw,
                height: ph,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&Default::default());

        // 单轨模型：key = n % 128，start_tick = n * 120。视口显示 key 0..27，
        // key 10 在视口内（8 个音符）。
        let model = Arc::new(make_stress_model(1, 1000));
        let note_revisions = model.note_revisions;
        let tv = vec![true];

        let mut last_cull_revision = 0u64;
        let mut last_cull_revision_only = 0u64;
        let mut last_hidden_hash = 0u64;
        let mut last_tv_hash = 0u64;
        let mut last_hidden_keys: HiddenKeyMask = [0; KEY_COUNT / 64];
        let mut rebuild: Option<CullRebuild> = None;
        let mut summary = SummaryLoadState::default();

        // 首帧：hidden 为空 → 全量上传，全部音符可见。
        let empty = HashSet::new();
        upload(GpuUploadState {
            pianoroll: &mut renderer,
            midi: Some(model.as_ref() as &dyn NoteSource),
            midi_arc: Some(&model),
            revision: 1,
            note_revisions: &note_revisions,
            track_visible: &tv,
            hidden_notes: &empty,
            last_cull_revision: &mut last_cull_revision,
            last_cull_revision_only: &mut last_cull_revision_only,
            last_hidden_hash: &mut last_hidden_hash,
            last_tv_hash: &mut last_tv_hash,
            last_hidden_keys: &mut last_hidden_keys,
            rebuild: &mut rebuild,
            summary_target: None,
            summary: &mut summary,
        });
        let px0 = render_pixel_count(
            &mut renderer,
            &device,
            &queue,
            &target,
            &target_view,
            pw,
            ph,
        );
        assert!(px0.0 > 0, "首帧应显示音符: {px0:?}");

        // 按下帧：hidden key 10 的全部音符（拖拽开始，revision 不动）→
        // 增量重建 key 10，其余 key 不动。
        let mut hidden = HashSet::new();
        for n in (0..1000u32).filter(|n| n % 128 == 10) {
            hidden.insert((0, n * 120, 10));
        }
        upload(GpuUploadState {
            pianoroll: &mut renderer,
            midi: Some(model.as_ref() as &dyn NoteSource),
            midi_arc: Some(&model),
            revision: 1,
            note_revisions: &note_revisions,
            track_visible: &tv,
            hidden_notes: &hidden,
            last_cull_revision: &mut last_cull_revision,
            last_cull_revision_only: &mut last_cull_revision_only,
            last_hidden_hash: &mut last_hidden_hash,
            last_tv_hash: &mut last_tv_hash,
            last_hidden_keys: &mut last_hidden_keys,
            rebuild: &mut rebuild,
            summary_target: None,
            summary: &mut summary,
        });
        let px1 = render_pixel_count(
            &mut renderer,
            &device,
            &queue,
            &target,
            &target_view,
            pw,
            ph,
        );
        assert!(
            px1.0 < px0.0,
            "hidden 的音符应从显示中消失: {px1:?} vs {px0:?}"
        );

        // 取消帧：hidden 清空（拖拽取消，revision 不动）→ 受影响 key 恢复，
        // 与首帧全量上传逐像素一致（cull 输出顺序确定）。
        upload(GpuUploadState {
            pianoroll: &mut renderer,
            midi: Some(model.as_ref() as &dyn NoteSource),
            midi_arc: Some(&model),
            revision: 1,
            note_revisions: &note_revisions,
            track_visible: &tv,
            hidden_notes: &empty,
            last_cull_revision: &mut last_cull_revision,
            last_cull_revision_only: &mut last_cull_revision_only,
            last_hidden_hash: &mut last_hidden_hash,
            last_tv_hash: &mut last_tv_hash,
            last_hidden_keys: &mut last_hidden_keys,
            rebuild: &mut rebuild,
            summary_target: None,
            summary: &mut summary,
        });
        let px2 = render_pixel_count(
            &mut renderer,
            &device,
            &queue,
            &target,
            &target_view,
            pw,
            ph,
        );
        assert_eq!(
            px2, px0,
            "取消拖拽后应逐像素恢复（增量重建遗漏 key 会暴露在这里）: {px2:?}"
        );
    }

    /// 受影响 key（当前 hidden ∪ 上次 hidden 的 key 并集）的增量重建结果
    /// 与全量构建逐 key 一致；并集外 key 的重建结果与无 hidden 时不变。
    #[test]
    fn affected_key_rebuild_matches_full_build() {
        let model = Arc::new(make_stress_model(2, 500));
        let tv = vec![true, true];
        let hidden: HashSet<(u16, u32, u8)> = [(0, 120, 1), (1, 600, 5), (0, 840, 7)]
            .into_iter()
            .collect();

        // hidden 的 key 位图 = {1, 5, 7}。
        let mut expect = [0u64; KEY_COUNT / 64];
        expect[0] |= (1u64 << 1) | (1u64 << 5) | (1u64 << 7);
        assert_eq!(hidden_key_mask(&hidden), expect);

        let (full, offsets) = yinhe_wgpu::build_all_notes(model.as_ref(), &hidden, &tv);
        for &key in &[1u8, 5, 7] {
            let key_notes = yinhe_wgpu::build_key_notes(model.as_ref(), key, &hidden, &tv);
            let start = offsets[key as usize] as usize;
            let end = offsets[key as usize + 1] as usize;
            assert_eq!(
                key_notes,
                full[start..end],
                "受影响 key {key} 的增量重建必须与全量构建一致"
            );
        }

        // 并集外 key：hidden 在其中的投影为空，重建结果与无 hidden 时逐字节相同。
        let (full_empty, offsets_empty) =
            yinhe_wgpu::build_all_notes(model.as_ref(), &HashSet::new(), &tv);
        let key_notes = yinhe_wgpu::build_key_notes(model.as_ref(), 100, &hidden, &tv);
        let start = offsets_empty[100] as usize;
        let end = offsets_empty[101] as usize;
        assert_eq!(key_notes, full_empty[start..end]);
    }

    /// 回归：从「仅主轨」切回「全部轨道」时，主轨无音符的 key（其 GPU
    /// buffer 在仅主轨数据下被清空）必须恢复显示。
    ///
    /// 旧实现：重建分帧上传对「无 buffer 的 key」判定增量失败并提前标记
    /// 完成，导致切回全部轨道后这些 key 及其后所有 key 永不恢复
    /// （用户现象：切换显示其他音轨后靠下的 key 消失，CPU 路径正常）。
    #[test]
    fn switch_back_to_all_tracks_restores_cleared_keys() {
        use yinhe_core::{ConductorData, NoteEvent, ProjectMeta, TrackData, YinModel};
        let Some((device, queue)) = (|| {
            let instance = wgpu::Instance::default();
            let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
            let (device, queue) =
                pollster::block_on(adapter.request_device(&Default::default())).ok()?;
            Some((device, queue))
        })() else {
            return;
        };
        let mut renderer = InstanceRenderer::new(
            device.clone(),
            queue.clone(),
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        let pw = 800u32;
        let ph = 600u32;
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("switch_back_target"),
            size: wgpu::Extent3d {
                width: pw,
                height: ph,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&Default::default());

        // track 0（主轨）只有 key 20；track 1 占低音 key 0/1。
        // 切「仅主轨」后 key 0/1 变空并被清 buffer；切回时必须恢复。
        let note = |tick: u32, key: u8| NoteEvent {
            id: tick,
            start_tick: tick,
            end_tick: tick + 100,
            key,
            velocity: 80,
        };
        let tracks = vec![
            Arc::new(TrackData::new(0, 0)),
            Arc::new(TrackData::new(0, 1)),
        ];
        let mut model = YinModel {
            conductor: Arc::new(ConductorData::default()),
            tracks,
            meta: ProjectMeta {
                ppq: 480,
                ..ProjectMeta::default()
            },
            ..Default::default()
        };
        model.load_track_notes(vec![vec![note(0, 20)], vec![note(0, 0), note(200, 1)]]);
        model.rebuild();
        let model = Arc::new(model);
        let hidden = HashSet::new();
        let note_revisions = model.note_revisions;

        let mut last_cull_revision = 0u64;
        let mut last_cull_revision_only = 0u64;
        let mut last_hidden_hash = 0u64;
        let mut last_tv_hash = 0u64;
        let mut last_hidden_keys: HiddenKeyMask = [0; KEY_COUNT / 64];
        let mut rebuild: Option<CullRebuild> = None;
        let mut summary = SummaryLoadState::default();

        // 多帧推进（重建分帧上传 + 收尾）。
        let mut feed = |tv: &[bool],
                        renderer: &mut InstanceRenderer,
                        last_cull_revision: &mut u64,
                        last_cull_revision_only: &mut u64,
                        last_hidden_hash: &mut u64,
                        last_tv_hash: &mut u64,
                        last_hidden_keys: &mut HiddenKeyMask,
                        rebuild: &mut Option<CullRebuild>| {
            for _ in 0..100 {
                upload(GpuUploadState {
                    pianoroll: renderer,
                    midi: Some(model.as_ref() as &dyn NoteSource),
                    midi_arc: Some(&model),
                    revision: 1,
                    note_revisions: &note_revisions,
                    track_visible: tv,
                    hidden_notes: &hidden,
                    last_cull_revision,
                    last_cull_revision_only,
                    last_hidden_hash,
                    last_tv_hash,
                    last_hidden_keys,
                    rebuild,
                    summary_target: None,
                    summary: &mut summary,
                });
                // 模拟真实帧间隔：给后台重建线程完成构建的机会
                // （紧凑循环会让 Building 一直 InProgress，测不出上传路径）。
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        };

        feed(
            &[true, true],
            &mut renderer,
            &mut last_cull_revision,
            &mut last_cull_revision_only,
            &mut last_hidden_hash,
            &mut last_tv_hash,
            &mut last_hidden_keys,
            &mut rebuild,
        );
        let (_, red0) = render_pixel_count(
            &mut renderer,
            &device,
            &queue,
            &target,
            &target_view,
            pw,
            ph,
        );
        assert!(red0 > 0, "初始 track1 的低音音符应可见: {red0}");

        // 仅主轨：key 0/1 被过滤为空。
        feed(
            &[true, false],
            &mut renderer,
            &mut last_cull_revision,
            &mut last_cull_revision_only,
            &mut last_hidden_hash,
            &mut last_tv_hash,
            &mut last_hidden_keys,
            &mut rebuild,
        );
        let (_, red1) = render_pixel_count(
            &mut renderer,
            &device,
            &queue,
            &target,
            &target_view,
            pw,
            ph,
        );
        assert_eq!(red1, 0, "仅主轨时 track1 不应可见");
        // 切回全轨：低音区必须恢复。
        feed(
            &[true, true],
            &mut renderer,
            &mut last_cull_revision,
            &mut last_cull_revision_only,
            &mut last_hidden_hash,
            &mut last_tv_hash,
            &mut last_hidden_keys,
            &mut rebuild,
        );
        let (blue2, red2) = render_pixel_count(
            &mut renderer,
            &device,
            &queue,
            &target,
            &target_view,
            pw,
            ph,
        );
        assert!(blue2 > 0, "主轨音符应始终可见: {blue2}");
        assert!(
            red2 > 0,
            "切回全部轨道后低音区 track1 音符必须恢复（低音区永久空白 bug）: {red2}"
        );
    }

    /// 测试辅助：跑一帧 `upload` 并指定摘要目标档。
    #[allow(clippy::too_many_arguments)]
    fn feed_summary_target(
        renderer: &mut InstanceRenderer,
        model: &Arc<YinModel>,
        revision: u64,
        note_revisions: &[u64; KEY_COUNT],
        tv: &[bool],
        hidden: &HashSet<(u16, u32, u8)>,
        last_cull_revision: &mut u64,
        last_cull_revision_only: &mut u64,
        last_hidden_hash: &mut u64,
        last_tv_hash: &mut u64,
        last_hidden_keys: &mut HiddenKeyMask,
        rebuild: &mut Option<CullRebuild>,
        summary: &mut SummaryLoadState,
        summary_target: Option<usize>,
    ) {
        upload(GpuUploadState {
            pianoroll: renderer,
            midi: Some(model.as_ref() as &dyn NoteSource),
            midi_arc: Some(model),
            revision,
            note_revisions,
            track_visible: tv,
            hidden_notes: hidden,
            last_cull_revision,
            last_cull_revision_only,
            last_hidden_hash,
            last_tv_hash,
            last_hidden_keys,
            rebuild,
            summary_target,
            summary,
        });
    }

    /// 懒构建（1C）：初始只建粗档；首次选中细档时后台构建 + 分帧上传。
    #[test]
    fn lazy_summary_level_builds_on_demand() {
        let Some((device, queue)) = (|| {
            let instance = wgpu::Instance::default();
            let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
            let (device, queue) =
                pollster::block_on(adapter.request_device(&Default::default())).ok()?;
            Some((device, queue))
        })() else {
            return;
        };
        let mut renderer = InstanceRenderer::new(
            device.clone(),
            queue.clone(),
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        let model = Arc::new(make_stress_model(1, 4000));
        let hidden = HashSet::new();
        let note_revisions = model.note_revisions;
        let tv = vec![true];

        let (all_notes, offsets) = yinhe_wgpu::build_all_notes(model.as_ref(), &hidden, &tv);
        let mut summary = SummaryLoadState::default();
        // 模拟加载：只构建最粗档（1024），细档留给懒构建。
        upload_all_with_summaries(
            &mut renderer,
            &all_notes,
            &offsets,
            &note_revisions,
            Some(0),
            &mut summary,
        );
        assert!(renderer.summary_level_ready(0), "粗档应立即就绪");
        assert!(
            !renderer.summary_level_ready(6),
            "细档（16）应懒构建（初始未构建）"
        );

        let revision = 1u64;
        let mut last_cull_revision = yinhe_wgpu::NoteBufferKey::new(revision, &tv, &hidden).value();
        let mut last_cull_revision_only = revision;
        let mut last_hidden_hash = yinhe_wgpu::hash_hidden(&hidden);
        let mut last_tv_hash = yinhe_wgpu::hash_bools(&tv);
        let mut last_hidden_keys: HiddenKeyMask = [0; KEY_COUNT / 64];
        let mut rebuild: Option<CullRebuild> = None;

        // 目标细档：启动懒构建。
        feed_summary_target(
            &mut renderer,
            &model,
            revision,
            &note_revisions,
            &tv,
            &hidden,
            &mut last_cull_revision,
            &mut last_cull_revision_only,
            &mut last_hidden_hash,
            &mut last_tv_hash,
            &mut last_hidden_keys,
            &mut rebuild,
            &mut summary,
            Some(6),
        );
        assert!(summary.load.is_some(), "选中未构建的细档应启动懒构建");
        assert!(
            !renderer.summary_level_ready(6),
            "构建/上传完成前该档不可用"
        );

        for _ in 0..400 {
            std::thread::sleep(std::time::Duration::from_millis(2));
            feed_summary_target(
                &mut renderer,
                &model,
                revision,
                &note_revisions,
                &tv,
                &hidden,
                &mut last_cull_revision,
                &mut last_cull_revision_only,
                &mut last_hidden_hash,
                &mut last_tv_hash,
                &mut last_hidden_keys,
                &mut rebuild,
                &mut summary,
                Some(6),
            );
            if renderer.summary_level_ready(6) {
                break;
            }
        }
        assert!(renderer.summary_level_ready(6), "懒构建完成后细档应可用");
        assert!(summary.load.is_none(), "完成后状态机应清空");
    }

    /// 懒构建期间编辑（revision 变化）→ 快照过期作废，按新状态重新构建。
    #[test]
    fn lazy_summary_discards_stale_build() {
        let Some((device, queue)) = (|| {
            let instance = wgpu::Instance::default();
            let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
            let (device, queue) =
                pollster::block_on(adapter.request_device(&Default::default())).ok()?;
            Some((device, queue))
        })() else {
            return;
        };
        let mut renderer = InstanceRenderer::new(
            device.clone(),
            queue.clone(),
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        let model = Arc::new(make_stress_model(1, 4000));
        let hidden = HashSet::new();
        let note_revisions = model.note_revisions;
        let tv = vec![true];

        let (all_notes, offsets) = yinhe_wgpu::build_all_notes(model.as_ref(), &hidden, &tv);
        let mut summary = SummaryLoadState::default();
        upload_all_with_summaries(
            &mut renderer,
            &all_notes,
            &offsets,
            &note_revisions,
            Some(0),
            &mut summary,
        );

        let revision = 1u64;
        let mut last_cull_revision = yinhe_wgpu::NoteBufferKey::new(revision, &tv, &hidden).value();
        let mut last_cull_revision_only = revision;
        let mut last_hidden_hash = yinhe_wgpu::hash_hidden(&hidden);
        let mut last_tv_hash = yinhe_wgpu::hash_bools(&tv);
        let mut last_hidden_keys: HiddenKeyMask = [0; KEY_COUNT / 64];
        let mut rebuild: Option<CullRebuild> = None;

        feed_summary_target(
            &mut renderer,
            &model,
            revision,
            &note_revisions,
            &tv,
            &hidden,
            &mut last_cull_revision,
            &mut last_cull_revision_only,
            &mut last_hidden_hash,
            &mut last_tv_hash,
            &mut last_hidden_keys,
            &mut rebuild,
            &mut summary,
            Some(6),
        );
        assert!(
            matches!(
                summary.load,
                Some(SummaryLevelLoad::Building { revision: 1, .. })
            ),
            "未编辑时应以 revision 1 启动懒构建"
        );

        // 编辑：revision 与 key 60 的 note_revision 变化。
        let revision2 = 2u64;
        let mut note_revisions2 = note_revisions;
        note_revisions2[60] += 1;
        feed_summary_target(
            &mut renderer,
            &model,
            revision2,
            &note_revisions2,
            &tv,
            &hidden,
            &mut last_cull_revision,
            &mut last_cull_revision_only,
            &mut last_hidden_hash,
            &mut last_tv_hash,
            &mut last_hidden_keys,
            &mut rebuild,
            &mut summary,
            Some(6),
        );
        match &summary.load {
            Some(SummaryLevelLoad::Building { revision, .. }) => assert_eq!(
                *revision, revision2,
                "过期懒构建应被丢弃并按新 revision 重新构建"
            ),
            _ => panic!("编辑后懒构建应重新启动（当前状态缺失）"),
        }
    }
}
