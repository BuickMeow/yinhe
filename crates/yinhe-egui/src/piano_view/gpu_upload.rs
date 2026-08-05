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
                        return;
                    }
                    // 重建数据基于旧 tv（期间切过轨）：GPU 已被旧 tv 数据
                    // 替换，强制本帧重新评估（用新 tv 重启重建）。
                    *last_cull_revision = 0;
                }
                Advance::Failed => {
                    *rebuild = None;
                    // 数据不完整：强制全量兜底。
                    *last_cull_revision = 0;
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
            base: yinhe_types::TimelineViewBase {
                pixels_per_tick: 0.1,
                scroll_x: 0.0,
                scroll_y: 2000.0, // 让 key 0..28 进入视口（音符分布在 key 0..61）
                left_panel_width: 60.0,
                dirty: true,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
            },
        };
        let track_colors: [[f32; 4]; 2] = [[0.2, 0.7, 1.0, 1.0], [0.9, 0.3, 0.3, 1.0]];
        let job = yinhe_wgpu::build_render_job(
            pw,
            ph,
            &view_data,
            &yinhe_core::Selection::default(),
            &track_colors,
            0,
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
        let mapped = buffer.slice(..).get_mapped_range();
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
        let mut rebuild: Option<CullRebuild> = None;

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
            rebuild: &mut rebuild,
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
            rebuild: &mut rebuild,
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
                rebuild: &mut rebuild,
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
            rebuild: &mut rebuild,
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
                rebuild: &mut rebuild,
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
                rebuild: &mut rebuild,
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
