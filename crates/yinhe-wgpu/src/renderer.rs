//! Generic wgpu renderer for instanced rectangle drawing.
//!
//! Manages three pipelines sharing one uniform buffer:
//!   - **decor pipeline** (32B `DrawInstance`, `vs_main`): decor, grid, keyboard, cursor
//!   - **curve pipeline** (32B `CurveInstance`, `vs_main_curve`): automation SDF lines/curves
//!   - **note pipeline** (16B `NoteInstance`, `vs_main_note`): PR notes, AR notes, ghost notes
//!
//! With GPU compute cull enabled, notes are uploaded once to a persistent
//! buffer and culled on the GPU each frame instead of rebuilt on the CPU.
//!
//! Layers are stored in z-order; `draw` switches pipelines as needed when
//! traversing layers.

use wgpu::*;

use crate::cull::CullState;
use crate::layer::{AnyLayer, LayerKind};
use crate::pipeline::RenderPipelineState;
use crate::vertex::{CurveInstance, NoteInstance, SelectionUniform, Uniforms, VelocityBarInstance};

/// Per-frame timing breakdown returned by `prepare`.
#[derive(Clone, Copy, Debug, Default)]
pub struct PrepareTimings {
    /// Time spent in the user-supplied `build` closure.
    pub build_static: std::time::Duration,
    /// Total instances uploaded.
    pub instance_count: usize,
}

pub struct InstanceRenderer {
    device: Device,
    queue: Queue,
    render: RenderPipelineState,
    cached_uniforms: Option<Uniforms>,
    cached_track_colors: Option<Vec<[f32; 4]>>,
    cached_selection: Option<SelectionUniform>,
    layers: Vec<AnyLayer>,
    pub(crate) cull: CullState,
    /// 诊断：上一次打印自检的小节索引（cull_diag_bar 用）。
    pub(crate) last_diag_bar: u64,
}

/// Generates a typed `upload_*_layer` method for one layer variant.
/// Eliminates the 4× near-identical boilerplate that previously existed.
macro_rules! impl_upload_layer {
    ($method:ident, $kind:ident, $variant:ident, $T:ty) => {
        /// Upload a layer. Skips rebuild when `cache_key` matches the previous value.
        /// Pass `cache_key: 0` to force upload (always rebuilds).
        pub fn $method(
            &mut self,
            index: usize,
            cache_key: u64,
            build: impl FnOnce(&mut Vec<$T>),
        ) -> bool {
            self.ensure_layer(index, LayerKind::$kind);
            if let AnyLayer::$variant(slot) = &mut self.layers[index] {
                if cache_key == 0 {
                    slot.upload_force(&self.device, &self.queue, build);
                    true
                } else {
                    slot.upload(&self.device, &self.queue, cache_key, build)
                }
            } else {
                unreachable!()
            }
        }
    };
}

impl InstanceRenderer {
    pub fn new(device: Device, queue: Queue, format: TextureFormat) -> Self {
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Gpu, || {
            let render_shader = device.create_shader_module(ShaderModuleDescriptor {
                label: Some("pianoroll_shader"),
                source: ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
            });

            // Cull 先建：note_pipeline 的 group 1 复用其 all_instances bind
            // group layout（顶点阶段经索引间接读 all_instances）。
            let cull = CullState::new(&device);
            let render = RenderPipelineState::new(
                &device,
                format,
                &render_shader,
                &cull.all_bind_group_layout,
            );

            Self {
                device,
                queue,
                render,
                cached_uniforms: None,
                cached_track_colors: None,
                cached_selection: None,
                layers: Vec::new(),
                cull,
                last_diag_bar: u64::MAX,
            }
        })
    }

    /// Upload uniforms to the GPU.  Skips the write when the value is unchanged.
    pub fn upload_uniforms(&mut self, uniforms: Uniforms) {
        crate::util::write_uniforms_if_changed(
            &self.queue,
            &self.render.uniform_buffer,
            &mut self.cached_uniforms,
            uniforms,
        );
    }

    /// Upload track colors to the GPU.  Skips the write when the value is unchanged.
    pub fn upload_track_colors(&mut self, colors: &[[f32; 4]]) {
        self.ensure_track_colors_capacity(colors.len());
        if self.cached_track_colors.as_deref() != Some(colors) {
            let bytes = bytemuck::cast_slice(colors);
            self.queue
                .write_buffer(&self.render.track_colors_buffer, 0, bytes);
            self.cached_track_colors = Some(colors.to_vec());
        }
    }

    /// Grow the track_colors storage buffer when `count` exceeds current capacity.
    /// Recreates the buffer + bind group (cheap, happens only when track count grows).
    fn ensure_track_colors_capacity(&mut self, count: usize) {
        if count <= self.render.track_colors_capacity as usize {
            return;
        }
        let new_capacity = count.max(1) as u32;
        let new_size = new_capacity as u64 * 16;
        let new_buffer = self.device.create_buffer(&BufferDescriptor {
            label: Some("track_colors"),
            size: new_size,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        yinhe_memtrace::add_gpu_resource(new_size);
        self.render.track_colors_buffer = new_buffer;
        self.render.track_colors_capacity = new_capacity;
        // Recreate bind group with the new buffer.
        self.render.bind_group = self.device.create_bind_group(&BindGroupDescriptor {
            label: Some("render_bind_group"),
            layout: &self.render.bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: self.render.uniform_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: self.render.track_colors_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: self.render.selection_buffer.as_entire_binding(),
                },
            ],
        });
        // Invalidate cache to force re-upload with the new buffer.
        self.cached_track_colors = None;
    }

    /// Upload selection rects to the GPU.  Skips the write when the value is unchanged.
    pub fn upload_selection(&mut self, sel: &SelectionUniform) {
        if self.cached_selection.as_ref() != Some(sel) {
            self.queue
                .write_buffer(&self.render.selection_buffer, 0, bytemuck::bytes_of(sel));
            self.cached_selection = Some(*sel);
        }
    }

    /// Ensure at least `count` decor layers exist (pushing empty ones as needed).
    /// Layers created here are decor by default; call `upload_note_layer` to
    /// upgrade a layer to the note pipeline.
    pub fn ensure_layers(&mut self, count: usize) {
        while self.layers.len() < count {
            self.layers
                .push(AnyLayer::new(&self.device, LayerKind::Decor));
        }
    }

    /// Ensure layer `index` exists with the given kind.  If the layer already
    /// exists with a different kind, it is replaced (buffer is recreated).
    pub fn ensure_layer(&mut self, index: usize, kind: LayerKind) {
        while self.layers.len() <= index {
            self.layers
                .push(AnyLayer::new(&self.device, LayerKind::Decor));
        }
        if self.layers[index].kind() != kind {
            self.layers[index] = AnyLayer::new(&self.device, kind);
        }
    }

    impl_upload_layer!(upload_note_layer, Note, Note, NoteInstance);
    impl_upload_layer!(upload_curve_layer, Curve, Curve, CurveInstance);
    impl_upload_layer!(
        upload_velocity_layer,
        Velocity,
        Velocity,
        VelocityBarInstance
    );

    /// Upload the per-track visibility bitmask to the cull shader.
    /// Call whenever `track_visible` changes; the shader immediately stops
    /// emitting hidden tracks' notes even from stale (pre-rebuild) buffers.
    pub fn upload_track_mask(&mut self, track_visible: &[bool]) {
        self.cull.upload_track_mask(&self.queue, track_visible);
    }

    /// Upload ALL note instances to the persistent GPU buffer for compute cull.
    /// Call this once on MIDI load/change, NOT every frame.
    /// Also records per-key offsets and revisions for future incremental uploads.
    pub fn upload_all_notes_for_cull(
        &mut self,
        notes: &[NoteInstance],
        per_key_offsets: &[u32; 129],
        key_revisions: &[u64; 128],
    ) {
        self.cull.upload_all_notes(
            &self.device,
            &self.queue,
            &self.render.uniform_buffer,
            notes,
            per_key_offsets,
            key_revisions,
        );
    }

    /// Incrementally upload a single key's notes. Grows the key's buffer and
    /// recreates its bind group on demand, so this handles count changes too.
    /// Returns false only if the key was never uploaded before (caller should
    /// fall back to `upload_all_notes_for_cull`).
    pub fn try_incremental_key_upload(
        &mut self,
        key: u8,
        notes: &[NoteInstance],
        revision: u64,
    ) -> bool {
        if !self.cull.has_key_buffer(key) {
            return false;
        }
        self.cull.upload_one_key(
            &self.device,
            &self.queue,
            &self.render.uniform_buffer,
            key,
            notes,
        );
        self.cull.uploaded_key_revisions[key as usize] = revision;
        true
    }

    /// Get the uploaded key revisions for comparison with model.
    pub fn uploaded_key_revisions(&self) -> &[u64; 128] {
        &self.cull.uploaded_key_revisions
    }

    /// Whether GPU compute cull is ready (all notes have been uploaded).
    pub fn cull_ready(&self) -> bool {
        self.cull.is_ready()
    }

    /// Drop all per-key GPU cull buffers and reset tracking so the next render
    /// treats the document as fresh (forces full upload on the next frame).
    ///
    /// Call when the active document changes (close / switch / new project) so
    /// note buffers from the previous document don't leak into the next render.
    pub fn clear_cull(&mut self) {
        self.cull.clear_cull();
        self.last_diag_bar = u64::MAX;
    }

    /// Whether GPU cull note buffers are uploaded (draw will use compute cull).
    pub fn cull_is_ready(&self) -> bool {
        self.cull.is_ready()
    }

    /// 诊断：读回上一帧 GPU cull 的可见实例总数（同步读回，仅诊断用）。
    pub fn cull_visible_count(&self) -> u64 {
        self.cull
            .readback_total_instances(&self.device, &self.queue)
    }

    /// 诊断：读回一个 key 的 draw_args 前 `n` 个 chunk 的完整 5 字段
    /// (index_count, instance_count, first_index, base_vertex, first_instance)。
    pub fn cull_draw_args_diag(&self, key: u8, n: u32) -> Vec<u32> {
        self.cull
            .readback_draw_args(&self.device, &self.queue, key, n)
    }

    /// 诊断：读回一个 key 的 visible_indices 前 `n` 个 u32（稀疏槽位内容）。
    pub fn cull_visible_indices_diag(&self, key: u8, n: u32) -> Vec<u32> {
        self.cull
            .readback_visible_indices(&self.device, &self.queue, key, n)
    }

    /// 诊断：最小 indirect draw 验证。独立 render pass（clear 红色），用
    /// CPU 手写的 args [6,1,0,0,0] + key 60 的 vertex/bind group 画 1 个实例。
    /// 若此测试画不出（而直接 draw_indexed 能画出）→ Adreno 驱动/栈的
    /// indirect draw 失效，需换用直接 draw 方案。
    pub fn diag_mini_indirect(
        &self,
        encoder: &mut CommandEncoder,
        target: &TextureView,
        width: u32,
        height: u32,
    ) {
        let manual = self.device.create_buffer(&BufferDescriptor {
            label: Some("mini_indirect"),
            size: 20,
            // 诊断变体：与 per_key_draw_args_buffers 相同的 usage（含 STORAGE），
            // 验证「STORAGE+INDIRECT 组合 buffer」是否导致 indirect draw 失效。
            usage: BufferUsages::STORAGE
                | BufferUsages::INDIRECT
                | BufferUsages::COPY_DST
                | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let args: [u32; 5] = [6, 1, 0, 0, 0];
        self.queue
            .write_buffer(&manual, 0, bytemuck::cast_slice(&args));
        let mut pass = crate::util::begin_pianoroll_pass(
            encoder,
            target,
            &self.render.note_pipeline,
            &self.render.bind_group,
            width,
            height,
        );
        let Some(vis) = self.cull.diag_visible_buffer(60) else {
            return;
        };
        let Some(bg) = self.cull.per_key_all_bind_group(60) else {
            return;
        };
        pass.set_bind_group(1, bg, &[]);
        pass.set_vertex_buffer(0, vis.slice(..));
        pass.set_index_buffer(self.render.index_buffer.slice(..), IndexFormat::Uint32);
        pass.draw_indexed_indirect(&manual, 0);
    }

    /// 诊断：三对照 draw 实验，定位「cull 数据全对但渲染 0 像素」的断点。
    ///
    /// - **A（红底）**：CPU 手写 args [6,1,0,0,0] + `draw_indexed_indirect`
    ///   （note_pipeline + key60 的 storage 绑定组 + visible 索引顶点流）。
    ///   A=0 而 B>0 → indirect draw 本身失效。
    /// - **B（绿底）**：与 A 相同绑定状态，改 `draw_indexed` 直接画（实例数读回
    ///   自 key60 的 args）。B=0 而 C>0 → 顶点阶段 storage 读取（all_instances
    ///   / 索引顶点流）失效。
    /// - **C（蓝底）**：`note_direct_pipeline` + CPU 硬编码单实例（视口中央），
    ///   不走 storage / indirect。C=0 → uniforms / 光栅化等更底层问题。
    ///
    /// clear 为 alpha=0 的彩色：rgb 计数≈全屏说明 pass 执行且 clear 生效，
    /// alpha 计数 = 实际画出的音符像素数。返回各实验的 (alpha, rgb) 计数。
    pub fn diag_draw_experiments(&self, format: TextureFormat, width: u32, height: u32) -> String {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        // 建纹理 → 录 pass → 读回 → 统计 (非零alpha, 非零rgb)（不含行 padding）。
        let run = |clear: Color, record: &dyn Fn(&mut RenderPass<'_>)| -> Option<(u64, u64)> {
            let device = &self.device;
            let texture = device.create_texture(&TextureDescriptor {
                label: Some("diag_exp_tex"),
                size: Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&TextureViewDescriptor::default());
            let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
            {
                let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                    label: Some("diag_exp_pass"),
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: Operations {
                            load: LoadOp::Clear(clear),
                            store: StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    ..Default::default()
                });
                pass.set_viewport(0.0, 0.0, width as f32, height as f32, 0.0, 1.0);
                record(&mut pass);
            }
            let bpp = 4u32;
            // COPY_BYTES_PER_ROW_ALIGNMENT(256) 对齐，否则 submit 被拒。
            let bytes_per_row = (width * bpp).div_ceil(256) * 256;
            let readback = device.create_buffer(&BufferDescriptor {
                label: Some("diag_exp_readback"),
                size: bytes_per_row as u64 * height as u64,
                usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(bytes_per_row),
                        rows_per_image: Some(height),
                    },
                },
                Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            self.queue.submit([encoder.finish()]);
            let done = Arc::new(AtomicBool::new(false));
            let done2 = done.clone();
            readback.slice(..).map_async(MapMode::Read, move |_| {
                done2.store(true, Ordering::SeqCst);
            });
            let _ = device.poll(PollType::wait_indefinitely());
            if !done.load(Ordering::SeqCst) {
                return None;
            }
            let data = readback.slice(..).get_mapped_range().ok()?;
            let mut nonzero_alpha = 0u64;
            let mut nonzero_rgb = 0u64;
            for row in 0..height as usize {
                let start = row * bytes_per_row as usize;
                for px in data[start..start + (width * bpp) as usize].chunks_exact(4) {
                    if px[3] > 0 {
                        nonzero_alpha += 1;
                    }
                    if px[0] > 0 || px[1] > 0 || px[2] > 0 {
                        nonzero_rgb += 1;
                    }
                }
            }
            drop(data);
            readback.unmap();
            Some((nonzero_alpha, nonzero_rgb))
        };

        let total_px = u64::from(width) * u64::from(height);
        let fmt = |r: Option<(u64, u64)>| match r {
            Some((a, rgb)) => format!("alpha={a} rgb={rgb}/{total_px}"),
            None => "读回失败".to_string(),
        };

        // A：红底 + CPU 手写 args 的 indirect draw（mini indirect 同款）。
        let manual = self.device.create_buffer(&BufferDescriptor {
            label: Some("diag_exp_manual_args"),
            size: 20,
            usage: BufferUsages::INDIRECT | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue
            .write_buffer(&manual, 0, bytemuck::cast_slice(&[6u32, 1, 0, 0, 0]));
        let a = (|| {
            let vis = self.cull.diag_visible_buffer(60)?;
            let bg = self.cull.per_key_all_bind_group(60)?;
            run(
                Color {
                    r: 1.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                },
                &|pass| {
                    pass.set_pipeline(&self.render.note_pipeline);
                    pass.set_bind_group(0, &self.render.bind_group, &[]);
                    pass.set_bind_group(1, bg, &[]);
                    pass.set_vertex_buffer(0, vis.slice(..));
                    pass.set_index_buffer(self.render.index_buffer.slice(..), IndexFormat::Uint32);
                    pass.draw_indexed_indirect(&manual, 0);
                },
            )
        })();

        // B：绿底 + 同绑定状态的直接 draw_indexed（绕开 indirect 机制）。
        let n = self
            .cull
            .readback_draw_args(&self.device, &self.queue, 60, 1)
            .get(1)
            .copied()
            .unwrap_or(0);
        let b = (|| {
            let vis = self.cull.diag_visible_buffer(60)?;
            let bg = self.cull.per_key_all_bind_group(60)?;
            run(
                Color {
                    r: 0.0,
                    g: 1.0,
                    b: 0.0,
                    a: 0.0,
                },
                &|pass| {
                    pass.set_pipeline(&self.render.note_pipeline);
                    pass.set_bind_group(0, &self.render.bind_group, &[]);
                    pass.set_bind_group(1, bg, &[]);
                    pass.set_vertex_buffer(0, vis.slice(..));
                    pass.set_index_buffer(self.render.index_buffer.slice(..), IndexFormat::Uint32);
                    pass.draw_indexed(0..6, 0, 0..n);
                },
            )
        })();

        // C：蓝底 + note_direct 硬编码单实例（视口中央，不走 storage/indirect）。
        let u = self.cached_uniforms.unwrap_or_default();
        let ppu = u.pixels_per_tick.max(1e-6);
        let center_tick = ((u.scroll_x + u.width * 0.5 - u.keyboard_width) / ppu).max(0.0) as u32;
        let kh = u.key_height.max(1.0);
        let bottom = 128.0 * kh - u.scroll_y;
        let center_key = ((bottom - u.height * 0.5) / kh).clamp(0.0, 127.0) as u8;
        let inst = NoteInstance {
            start_tick: center_tick,
            end_tick: center_tick + 480,
            packed: NoteInstance::pack(center_key, 0, 100),
        };
        let vbuf = self.device.create_buffer(&BufferDescriptor {
            label: Some("diag_exp_note_vb"),
            size: std::mem::size_of::<NoteInstance>() as u64,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&vbuf, 0, bytemuck::bytes_of(&inst));
        let c = run(
            Color {
                r: 0.0,
                g: 0.0,
                b: 1.0,
                a: 0.0,
            },
            &|pass| {
                pass.set_pipeline(&self.render.note_direct_pipeline);
                pass.set_bind_group(0, &self.render.bind_group, &[]);
                pass.set_vertex_buffer(0, vbuf.slice(..));
                pass.set_index_buffer(self.render.index_buffer.slice(..), IndexFormat::Uint32);
                pass.draw_indexed(0..6, 0, 0..1);
            },
        );

        format!(
            "A(indirect): {} | B(direct n={n}): {} | C(hardcode): {}",
            fmt(a),
            fmt(b),
            fmt(c)
        )
    }

    /// 诊断：每跨过一个小节边界，打印「CPU 构建可见数 vs GPU cull 显示数」。
    /// 在 `draw`（dispatch）之后调用，读回的是本帧的 draw_args。
    /// 仅当 YIN_CULL_DIAG=1 时生效（避免正常使用时的每小节同步读回开销）。
    /// 用于定位「GPU cull 显示中断」：若 gpu 数在某小节后停止增长/骤降，
    /// 即中断点。
    pub fn cull_diag_bar(
        &mut self,
        view: &yinhe_types::PianoRollView,
        midi: Option<&dyn yinhe_types::NoteSource>,
        w: f32,
        h: f32,
        hidden_notes: &std::collections::HashSet<(u16, u32, u8)>,
        track_visible: &[bool],
    ) {
        if std::env::var_os("YIN_CULL_DIAG").is_none() {
            return;
        }
        if !self.cull.is_ready() {
            return;
        }
        let ppq = midi.and_then(|m| m.ticks_per_beat()).unwrap_or(480) as f32;
        let bar_ticks = (ppq * 4.0).max(1.0); // 4/4 一小节
        let bar = (view.base.scroll_x / (view.base.pixels_per_tick * bar_ticks)).max(0.0) as u64;
        if bar == self.last_diag_bar {
            return;
        }
        self.last_diag_bar = bar;

        // CPU 参考：非 cull 模式的构建路径
        let mut cpu = 0u64;
        if let Some(midi) = midi {
            let mut out = Vec::new();
            crate::pianoroll::build_notes(&mut out, w, h, midi, view, hidden_notes, track_visible);
            cpu = out.len() as u64;
        }
        let gpu = self
            .cull
            .readback_total_instances(&self.device, &self.queue);
        tracing::info!(
            "[cull-diag] bar={bar} scroll_x={} tick={} cpu={cpu} gpu={gpu} diff={}",
            view.base.scroll_x,
            view.base.scroll_x / view.base.pixels_per_tick,
            gpu as i64 - cpu as i64,
        );
    }

    /// Draw all layers into the given render target.
    /// Uses GPU compute cull for note layers if available, otherwise falls back
    /// to CPU-built layer data.
    pub fn draw(
        &mut self,
        encoder: &mut CommandEncoder,
        target: &TextureView,
        width: u32,
        height: u32,
    ) {
        log::info!(
            "[draw] enter cull_ready={} layers={}",
            self.cull.is_ready(),
            self.layers.len()
        );
        if self.cull.is_ready() {
            self.draw_with_cull(encoder, target, width, height);
        } else {
            self.draw_legacy(encoder, target, width, height);
        }
    }

    /// Draw decor → velocity → curve layers (shared by both draw paths).
    /// Notes are handled separately by each path (legacy: CPU-built note layers;
    /// cull: GPU compute culled notes + ghost layer).
    fn draw_static_layers(&self, pass: &mut RenderPass<'_>) {
        for layer in &self.layers {
            if layer.kind() == LayerKind::Decor {
                pass.set_pipeline(&self.render.pipeline);
                layer.draw(pass, 0, None);
            }
        }
        for layer in &self.layers {
            if layer.kind() == LayerKind::Velocity {
                pass.set_pipeline(&self.render.velocity_pipeline);
                layer.draw(pass, 0, Some(&self.render.index_buffer));
            }
        }
        for layer in &self.layers {
            if layer.kind() == LayerKind::Curve {
                pass.set_pipeline(&self.render.curve_pipeline);
                layer.draw(pass, 0, None);
            }
        }
    }

    /// Legacy draw (no GPU cull): draw all decor layers then all note layers.
    ///
    /// Z-order: decor (bg + grid) → velocity bars → curve (automation) → notes
    fn draw_legacy(
        &self,
        encoder: &mut CommandEncoder,
        target: &TextureView,
        width: u32,
        height: u32,
    ) {
        let mut pass = crate::util::begin_pianoroll_pass(
            encoder,
            target,
            &self.render.pipeline,
            &self.render.bind_group,
            width,
            height,
        );

        self.draw_static_layers(&mut pass);

        // Step 4: all note layers
        for layer in &self.layers {
            if layer.kind() == LayerKind::Note {
                pass.set_pipeline(&self.render.note_direct_pipeline);
                layer.draw(&mut pass, 0, Some(&self.render.index_buffer));
            }
        }
    }

    /// Compute the visible key range from cached uniforms (PR mode only).
    /// Returns `(lo, hi)` inclusive. For non-PR modes, returns `(0, 127)` since
    /// the Y position depends on both key and track (can't skip by key alone).
    ///
    /// Adds 1 key of padding on each side to handle notes whose top/bottom edge
    /// peeks into the viewport due to sub-pixel rounding.
    fn visible_key_range(&self) -> (u8, u8) {
        let u = match &self.cached_uniforms {
            Some(u) => u,
            None => return (0, 127),
        };
        if u.mode != 1 || u.key_height <= 0.0 {
            return (0, 127);
        }
        // PR: bottom = 128 * key_height - scroll_y
        // y_to_key(y) = ceil((bottom - y) / key_height) - 1, clamped to 0..127
        let bottom = 128.0 * u.key_height - u.scroll_y;
        let top_key = ((bottom / u.key_height).ceil() as i32 - 1).clamp(0, 127);
        let bottom_key = (((bottom - u.height) / u.key_height).ceil() as i32 - 1).clamp(0, 127);
        let lo = bottom_key.min(top_key);
        let hi = bottom_key.max(top_key);
        // 1-key padding for sub-pixel edge cases.
        let lo = lo.saturating_sub(1).clamp(0, 127);
        let hi = hi.saturating_add(1).clamp(0, 127);
        (lo as u8, hi as u8)
    }

    /// GPU compute cull draw: dispatch cull pass, then draw layers.
    ///
    /// Z-order: decor (bg + grid) → velocity bars → curve (automation) → culled notes → ghost notes.
    fn draw_with_cull(
        &mut self,
        _encoder: &mut CommandEncoder,
        target: &TextureView,
        width: u32,
        height: u32,
    ) {
        tracing::debug!(
            "[cull-draw] is_ready={} cached_scroll_x={:?} cached_ppu={:?} cached_mode={:?}",
            self.cull.is_ready(),
            self.cached_uniforms.as_ref().map(|u| u.scroll_x),
            self.cached_uniforms.as_ref().map(|u| u.pixels_per_tick),
            self.cached_uniforms.as_ref().map(|u| u.mode),
        );
        let (key_lo, key_hi) = self.visible_key_range();
        // Phase 1: Compute cull (skipped if uniforms + notes unchanged since last frame)
        let uniforms = self.cached_uniforms.unwrap_or_default();
        // 内部管理 encoder 与提交：compute dispatch 独立提交后同步读回 args
        // 到 CPU（Adreno 驱动 indirect draw 失效，只能 CPU 读回 + 直接 draw，
        // 跨 submit 读回是稳定路径）。
        let mut enc = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor::default());
        let executed = self
            .cull
            .dispatch_cull(&mut enc, &self.queue, key_lo, key_hi, &uniforms);
        if executed {
            self.queue.submit([enc.finish()]);
            self.cull.readback_args_to_cpu(&self.device, &self.queue);
        }

        // Phase 2: Single render pass
        let mut enc2 = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor::default());
        let mut pass = crate::util::begin_pianoroll_pass(
            &mut enc2,
            target,
            &self.render.pipeline,
            &self.render.bind_group,
            width,
            height,
        );

        // Step 1-3: decor → velocity → curve
        self.draw_static_layers(&mut pass);

        // Step 4: culled notes (from GPU compute cull buffer)
        self.cull.draw_visible_notes(
            &mut pass,
            &self.render.note_pipeline,
            &self.render.bind_group,
            &self.render.index_buffer,
            key_lo,
            key_hi,
        );

        // Step 5: ghost notes (last note layer, if any) — on top of everything
        let ghost = self.layers.iter().rfind(|l| l.kind() == LayerKind::Note);
        if let Some(ghost) = ghost {
            pass.set_pipeline(&self.render.note_direct_pipeline);
            ghost.draw(&mut pass, 0, Some(&self.render.index_buffer));
        }
        drop(pass);
        self.queue.submit([enc2.finish()]);
    }

    /// 当前 GPU 主题（从全局读取，随用户主题切换自动更新）。
    pub fn theme(&self) -> yinhe_theme::GpuTheme {
        yinhe_theme::current_gpu_theme()
    }
}
