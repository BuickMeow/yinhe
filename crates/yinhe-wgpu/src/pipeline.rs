use wgpu::*;

use crate::resource::{TrackedBuffer, TrackedTexture};
use crate::vertex::{
    CurveInstance, DrawInstance, NoteInstance, SelectionUniform, Uniforms, VelocityBarInstance,
};

/// 数据纹理行宽（须与 `shader_texture.wgsl` 的 `TABLE_COLS` 一致）。
const TABLE_COLS: u32 = 1024;

/// WGSL 分三段拼装：共享前缀 + 变体前缀 + 主体。
/// 两套变体只在「调色板 / AR 行高表」的取数方式上不同：
///   - storage 变体：storage buffer（GLES 3.1+ / Metal / Vulkan）；
///   - texture 变体：RGBA32F 数据纹理（GLES 3.0，无 SSBO）。
const SHADER_STORAGE: &str = concat!(
    include_str!("shader_base.wgsl"),
    include_str!("shader_storage.wgsl"),
    include_str!("shader_body.wgsl"),
);
const SHADER_TEXTURE: &str = concat!(
    include_str!("shader_base.wgsl"),
    include_str!("shader_texture.wgsl"),
    include_str!("shader_body.wgsl"),
);

/// 本设备可用的 WGSL 源码（`use_storage` = 支持 shader 内 storage buffer）。
pub(crate) fn shader_source(use_storage: bool) -> &'static str {
    if use_storage {
        SHADER_STORAGE
    } else {
        SHADER_TEXTURE
    }
}

/// 一张数据纹理表：行宽固定 `TABLE_COLS`，行数按条目数增长。
struct TableTexture {
    texture: TrackedTexture,
    view: TextureView,
    /// 已分配行数（容量）。
    rows: u32,
}

impl TableTexture {
    fn with_rows(device: &Device, label: &str, rows: u32) -> Self {
        let texture = TrackedTexture::new(
            device,
            &TextureDescriptor {
                label: Some(label),
                size: Extent3d {
                    width: TABLE_COLS,
                    height: rows,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba32Float,
                usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                view_formats: &[],
            },
        );
        let view = texture.create_view(&TextureViewDescriptor::default());
        Self {
            texture,
            view,
            rows,
        }
    }

    fn new(device: &Device, label: &str) -> Self {
        Self::with_rows(device, label, 1)
    }

    /// 保证至少 `entries` 项容量；返回是否重建（重建后须重建 bind group）。
    fn ensure_entries(&mut self, device: &Device, label: &str, entries: usize) -> bool {
        let need = (entries as u32).div_ceil(TABLE_COLS).max(1);
        if need <= self.rows {
            return false;
        }
        *self = Self::with_rows(device, label, need.next_power_of_two());
        true
    }

    /// 写入整张表：`data` 长度须为 `rows × TABLE_COLS`（尾部补零）。
    fn write(&self, queue: &Queue, data: &[[f32; 4]]) {
        queue.write_texture(
            TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            bytemuck::cast_slice(data),
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(TABLE_COLS * 16),
                rows_per_image: Some(self.rows),
            },
            Extent3d {
                width: TABLE_COLS,
                height: self.rows,
                depth_or_array_layers: 1,
            },
        );
    }
}

/// GLES 3.0 降级路径的调色板 / AR 行高表（数据纹理），
/// 与 `shader_texture.wgsl` 的 @group(0) 绑定一一对应。
struct TextureTable {
    sampler: Sampler,
    colors: TableTexture,
    rows: TableTexture,
}

impl TextureTable {
    fn new(device: &Device) -> Self {
        Self {
            // 表数据不参与插值：非过滤采样器 + RGBA32F（非 filterable）。
            sampler: device.create_sampler(&SamplerDescriptor {
                label: Some("table_sampler"),
                mag_filter: FilterMode::Nearest,
                min_filter: FilterMode::Nearest,
                mipmap_filter: MipmapFilterMode::Nearest,
                ..SamplerDescriptor::default()
            }),
            colors: TableTexture::new(device, "track_colors_table"),
            rows: TableTexture::new(device, "row_table"),
        }
    }

    /// 调色板扩容；返回是否重建了纹理（需重建 bind group）。
    fn ensure_colors(&mut self, device: &Device, count: usize) -> bool {
        self.colors
            .ensure_entries(device, "track_colors_table", count)
    }

    /// AR 行高表扩容；返回是否重建了纹理。
    fn ensure_rows(&mut self, device: &Device, count: usize) -> bool {
        self.rows.ensure_entries(device, "row_table", count)
    }

    /// 上传调色板（尾部补零到整行）。
    fn write_colors(&self, queue: &Queue, colors: &[[f32; 4]]) {
        let mut data = vec![[0.0f32; 4]; self.colors.rows as usize * TABLE_COLS as usize];
        for (dst, src) in data.iter_mut().zip(colors) {
            *dst = *src;
        }
        self.colors.write(queue, &data);
    }

    /// 上传 AR 行表：每行 `(offset, height)`，由 offsets/heights 两个
    /// 平行数组合成（缺项补 0，与 storage 变体的零初始化一致）。
    fn write_rows(&self, queue: &Queue, offsets: &[f32], heights: &[f32]) {
        let mut data = vec![[0.0f32; 4]; self.rows.rows as usize * TABLE_COLS as usize];
        for (i, slot) in data.iter_mut().enumerate() {
            if i >= offsets.len() && i >= heights.len() {
                break;
            }
            slot[0] = offsets.get(i).copied().unwrap_or(0.0);
            slot[1] = heights.get(i).copied().unwrap_or(0.0);
        }
        self.rows.write(queue, &data);
    }
}

/// Owns the render pipelines and their shared uniform buffers / bind group.
///
/// Five pipelines share the uniform buffer, track-colors buffer, selection
/// buffer, and bind group:
///   - `pipeline`: decor pipeline (32-byte `DrawInstance` vertex layout, `vs_main`)
///   - `note_pipeline`: GPU-cull note pipeline (4-byte u32 index vertex layout,
///     `vs_main_note` — indirect-reads `all_instances` via @group(1) = cull bind group)
///   - `note_direct_pipeline`: CPU-built note pipeline (12-byte `NoteInstance`
///     vertex layout, `vs_main_note_direct` — ghost notes / legacy draw)
///   - `velocity_pipeline`: velocity bar pipeline (16-byte `VelocityBarInstance` vertex
///     layout, `vs_main_velocity` — unified border-based mode)
///   - `curve_pipeline`: automation curve pipeline (32-byte `CurveInstance` vertex layout,
///     `vs_main_curve`/`fs_main_curve` — per-pixel SDF line/curve)
///   - `curve_overlay_pipeline`: 折叠自动化预览叠加层（同 `CurveInstance` 布局，
///     `fs_main_curve_premul` + Max 混合 — 整层 alpha 固定，重叠不加深）
pub struct RenderPipelineState {
    pub pipeline: RenderPipeline,
    /// GPU cull 音符管线：顶点阶段经 @group(1) 读 `all_instances` storage buffer。
    /// 适配器无 storage buffer（GLES 3.0）时为 `None`，恒走 CPU 层渲染。
    pub note_pipeline: Option<RenderPipeline>,
    pub note_direct_pipeline: RenderPipeline,
    pub velocity_pipeline: RenderPipeline,
    pub curve_pipeline: RenderPipeline,
    /// 折叠自动化预览叠加层：同 `curve_pipeline` 的 SDF 曲线，但预乘 alpha +
    /// Max 混合，使整层 alpha 固定（重叠不加深），且可画在音符层之下。
    pub curve_overlay_pipeline: RenderPipeline,
    pub uniform_buffer: TrackedBuffer,
    pub track_colors_buffer: TrackedBuffer,
    /// Current capacity (in vec4 entries) of `track_colors_buffer`.
    pub track_colors_capacity: u32,
    /// AR 模式下每个音轨主行的 y 偏移表（f32 数组，shader binding 3）。
    pub track_offsets_buffer: TrackedBuffer,
    /// Current capacity (in f32 entries) of `track_offsets_buffer`.
    pub track_offsets_capacity: u32,
    /// AR 模式下每行的行高表（f32 数组，shader binding 5）。
    /// 轨道 AR 上传全 `lane_height`；通道 AR 上传每行实际行高（展开的
    /// 通道合并行变窄），shader 按行高压缩音符 key 空间。
    pub row_heights_buffer: TrackedBuffer,
    /// Current capacity (in f32 entries) of `row_heights_buffer`.
    pub row_heights_capacity: u32,
    pub selection_buffer: TrackedBuffer,
    /// 选中高亮排除表（成员态「矩形内非成员」键，`array<vec2<u32>>`，binding 4）。
    pub exclude_buffer: TrackedBuffer,
    /// `exclude_buffer` 当前容量（vec2<u32> 项数）。
    pub exclude_capacity: u32,
    pub bind_group: BindGroup,
    pub bind_group_layout: BindGroupLayout,
    /// 共享 index buffer：矩形四角 [0,1,2, 1,3,2]（0=TL,1=TR,2=BL,3=BR）。
    /// note / velocity pipeline 以 4 顶点 + 此 index 绘制（顶点 -33%）。
    pub index_buffer: TrackedBuffer,
    /// 无 storage buffer 的适配器（GLES 3.0）上的调色板 / AR 行表纹理。
    /// `Some` 时 bind group 只含 uniform + 两张表纹理 + 采样器。
    tex: Option<TextureTable>,
}

impl RenderPipelineState {
    pub fn new(
        device: &Device,
        format: TextureFormat,
        render_shader: &ShaderModule,
        all_bind_group_layout: &BindGroupLayout,
    ) -> Self {
        // 目标附着为 sRGB 时，着色器把 sRGB 颜色分量转为线性再输出，
        // 避免硬件二次编码导致整体变淡（与 egui/调色板显示值一致）。
        let srgb_constants = [("SRGB_TARGET", if format.is_srgb() { 1.0 } else { 0.0 })];
        let compilation_options = || PipelineCompilationOptions {
            constants: &srgb_constants,
            ..PipelineCompilationOptions::default()
        };
        // 无 SSBO 的适配器（GLES 3.0）：调色板/行表改走数据纹理，
        // 且 GPU cull 管线（需顶点 storage）不创建。
        let use_storage = crate::util::has_storage_buffers(device);

        // Main uniforms buffer
        let uniform_size = std::mem::size_of::<Uniforms>() as u64;
        let uniform_buffer = TrackedBuffer::new(
            device,
            &BufferDescriptor {
                label: Some("uniforms"),
                size: uniform_size,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );

        // Track colors buffer — dynamically sized to actual track count.
        // Start with 1 entry (16B); grows on demand via `ensure_track_colors_capacity`.
        // Bound as a read-only storage buffer (runtime-sized array in WGSL).
        let track_colors_size = 16; // 1 × vec4<f32>
        let track_colors_buffer = TrackedBuffer::new(
            device,
            &BufferDescriptor {
                label: Some("track_colors"),
                size: track_colors_size,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );

        // Track offsets buffer（AR 每轨主行 y 偏移）——初始 4 个 f32（16B），
        // 随轨道数按需增长，见 renderer.rs ensure_track_offsets_capacity。
        let track_offsets_size = 16; // 4 × f32
        let track_offsets_buffer = TrackedBuffer::new(
            device,
            &BufferDescriptor {
                label: Some("track_offsets"),
                size: track_offsets_size,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );

        // Row heights buffer（AR 每行行高；通道 AR 的窄合并行依赖它）——
        // 初始 4 个 f32（16B），随行数按需增长，见 renderer.rs
        // ensure_row_heights_capacity。
        let row_heights_buffer = TrackedBuffer::new(
            device,
            &BufferDescriptor {
                label: Some("row_heights"),
                size: 16, // 4 × f32
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );

        // Selection rects buffer
        let selection_size = std::mem::size_of::<SelectionUniform>() as u64;
        let selection_buffer = TrackedBuffer::new(
            device,
            &BufferDescriptor {
                label: Some("selection"),
                size: selection_size,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );

        // 排除表 buffer（初始容量 16 项，按需扩容重建 bind group）
        let exclude_capacity = 16u32;
        let exclude_buffer = TrackedBuffer::new(
            device,
            &BufferDescriptor {
                label: Some("selection_exclude"),
                size: (exclude_capacity as u64) * 8,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );

        // 共享 group 0：storage 变体 = uniform + 3 个 storage buffer；
        // texture 变体（GLES 3.0 无 SSBO）= uniform + 调色板/行表纹理 + 采样器。
        let tex = if use_storage {
            None
        } else {
            Some(TextureTable::new(device))
        };
        let bind_group_layout = if tex.is_some() {
            create_texture_layout(device)
        } else {
            create_storage_layout(device)
        };
        let bind_group = create_bind_group(
            device,
            &bind_group_layout,
            tex.as_ref(),
            &uniform_buffer,
            &track_colors_buffer,
            &selection_buffer,
            &track_offsets_buffer,
            &exclude_buffer,
            &row_heights_buffer,
        );

        // 共享 index buffer：两个三角形共享矩形四角。
        // 顶点顺序与 shader 的 pos/uv 数组一致（0=TL, 1=TR, 2=BL, 3=BR）。
        let index_buffer = TrackedBuffer::new(
            device,
            &BufferDescriptor {
                label: Some("shared_index_buffer"),
                size: 6 * std::mem::size_of::<u32>() as u64,
                usage: BufferUsages::INDEX | BufferUsages::COPY_DST,
                mapped_at_creation: true,
            },
        );
        {
            let idx: [u32; 6] = [0, 1, 2, 1, 3, 2];
            let Ok(mut mapped) = index_buffer.slice(..).get_mapped_range_mut() else {
                // mapped_at_creation buffer 映射失败 = 设备不可用，无法继续渲染
                panic!("index buffer not mappable at creation");
            };
            mapped.copy_from_slice(bytemuck::cast_slice(&idx));
        }
        index_buffer.unmap();

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        // GPU-cull note pipeline layout: group 0 = shared render bind group
        // (uniform/track_colors/selection), group 1 = per-key all_instances
        // bind group so the vertex shader can indirect-read the full
        // NoteInstance from the 4-byte visible index.
        let cull_note_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("cull_note_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout), Some(all_bind_group_layout)],
            immediate_size: 0,
        });

        // Decor pipeline: 32-byte DrawInstance vertex layout
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("decor_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: render_shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(VertexBufferLayout {
                    array_stride: std::mem::size_of::<DrawInstance>() as u64,
                    step_mode: VertexStepMode::Instance,
                    attributes: &vertex_attr_array![
                        0 => Float32x4,
                        1 => Uint32x4,
                    ],
                })],
                compilation_options: compilation_options(),
            },
            fragment: Some(FragmentState {
                module: render_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: compilation_options(),
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                ..PrimitiveState::default()
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // GPU-cull note pipeline: 4-byte u32 index vertex layout, indirect-reads
        // all_instances via @group(1). Shares uniforms/bind group.
        // 音符全不透明（硬边，无 AA），关闭混合避免 0.5 缝隙透底 + 省 ROP。
        // 顶点阶段读 storage buffer（@group(1)）：GLES 3.0 上不可用 → None。
        let note_pipeline = if use_storage {
            Some(device.create_render_pipeline(&RenderPipelineDescriptor {
                label: Some("note_pipeline"),
                layout: Some(&cull_note_pipeline_layout),
                vertex: VertexState {
                    module: render_shader,
                    entry_point: Some("vs_main_note"),
                    buffers: &[Some(VertexBufferLayout {
                        array_stride: std::mem::size_of::<u32>() as u64,
                        step_mode: VertexStepMode::Instance,
                        attributes: &vertex_attr_array![0 => Uint32],
                    })],
                    compilation_options: compilation_options(),
                },
                fragment: Some(FragmentState {
                    module: render_shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(ColorTargetState {
                        format,
                        blend: None,
                        write_mask: ColorWrites::ALL,
                    })],
                    compilation_options: compilation_options(),
                }),
                primitive: PrimitiveState {
                    topology: PrimitiveTopology::TriangleList,
                    ..PrimitiveState::default()
                },
                depth_stencil: None,
                multisample: MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            }))
        } else {
            None
        };

        // Direct note pipeline: 12-byte NoteInstance vertex layout (ghost notes,
        // legacy CPU-built note layers). Same vertex math, no index indirection.
        let note_direct_pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("note_direct_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: render_shader,
                entry_point: Some("vs_main_note_direct"),
                buffers: &[Some(VertexBufferLayout {
                    array_stride: std::mem::size_of::<NoteInstance>() as u64,
                    step_mode: VertexStepMode::Instance,
                    attributes: &vertex_attr_array![
                        0 => Uint32x3,
                    ],
                })],
                compilation_options: compilation_options(),
            },
            fragment: Some(FragmentState {
                module: render_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: compilation_options(),
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                ..PrimitiveState::default()
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Curve pipeline: 28-byte CurveInstance vertex layout, shares uniforms/bind group.
        // Renders automation segments as per-pixel SDF lines / anchors.
        // Bézier curves are flattened into a polyline of line instances on the CPU.
        let curve_pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("curve_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: render_shader,
                entry_point: Some("vs_main_curve"),
                buffers: &[Some(VertexBufferLayout {
                    array_stride: std::mem::size_of::<CurveInstance>() as u64,
                    step_mode: VertexStepMode::Instance,
                    attributes: &vertex_attr_array![
                        0 => Float32x4,  // endp (x1, y1, x2, y2) @ offset 0
                        1 => Float32,    // thickness @ offset 16
                        2 => Uint32,     // rgba_packed @ offset 20
                        3 => Uint32,     // shape @ offset 24
                    ],
                })],
                compilation_options: compilation_options(),
            },
            fragment: Some(FragmentState {
                module: render_shader,
                entry_point: Some("fs_main_curve"),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: compilation_options(),
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                ..PrimitiveState::default()
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Curve overlay pipeline：与 curve_pipeline 同顶点布局/entry，但输出
        // 预乘 alpha（fs_main_curve_premul），并用 Max 混合。目标纹理在折叠
        // 音轨行处为透明，Max 取并集 → 多条 lane 重叠时 alpha 恒为层 alpha，
        // 不会叠加变深。整体由 egui 按预乘 alpha 合成到背景。
        let curve_overlay_pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("curve_overlay_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: render_shader,
                entry_point: Some("vs_main_curve"),
                buffers: &[Some(VertexBufferLayout {
                    array_stride: std::mem::size_of::<CurveInstance>() as u64,
                    step_mode: VertexStepMode::Instance,
                    attributes: &vertex_attr_array![
                        0 => Float32x4,
                        1 => Float32,
                        2 => Uint32,
                        3 => Uint32,
                    ],
                })],
                compilation_options: compilation_options(),
            },
            fragment: Some(FragmentState {
                module: render_shader,
                entry_point: Some("fs_main_curve_premul"),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(BlendState {
                        color: BlendComponent {
                            src_factor: BlendFactor::One,
                            dst_factor: BlendFactor::One,
                            operation: BlendOperation::Max,
                        },
                        alpha: BlendComponent {
                            src_factor: BlendFactor::One,
                            dst_factor: BlendFactor::One,
                            operation: BlendOperation::Max,
                        },
                    }),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: compilation_options(),
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                ..PrimitiveState::default()
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Velocity bar pipeline: 16-byte VelocityBarInstance vertex layout.
        // 仅描边、不填充、不加深（`fs_main_velocity`），与音符的填充+描边区分。
        let velocity_pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("velocity_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: render_shader,
                entry_point: Some("vs_main_velocity"),
                buffers: &[Some(VertexBufferLayout {
                    array_stride: std::mem::size_of::<VelocityBarInstance>() as u64,
                    step_mode: VertexStepMode::Instance,
                    attributes: &vertex_attr_array![
                        0 => Uint32x4,
                    ],
                })],
                compilation_options: compilation_options(),
            },
            fragment: Some(FragmentState {
                module: render_shader,
                entry_point: Some("fs_main_velocity"),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: compilation_options(),
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                ..PrimitiveState::default()
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            note_pipeline,
            note_direct_pipeline,
            velocity_pipeline,
            curve_pipeline,
            curve_overlay_pipeline,
            uniform_buffer,
            track_colors_buffer,
            track_colors_capacity: 1,
            track_offsets_buffer,
            track_offsets_capacity: 4,
            row_heights_buffer,
            row_heights_capacity: 4,
            selection_buffer,
            exclude_buffer,
            exclude_capacity,
            bind_group,
            bind_group_layout,
            index_buffer,
            tex,
        }
    }

    /// 重建共享 bind group（排除表/调色板/行表扩容后调用；layout 不变）。
    pub fn rebuild_bind_group(&mut self, device: &Device) {
        self.bind_group = create_bind_group(
            device,
            &self.bind_group_layout,
            self.tex.as_ref(),
            &self.uniform_buffer,
            &self.track_colors_buffer,
            &self.selection_buffer,
            &self.track_offsets_buffer,
            &self.exclude_buffer,
            &self.row_heights_buffer,
        );
    }

    /// 当前是否为数据纹理变体（无 storage buffer 的 GLES 3.0）。
    pub fn has_texture_tables(&self) -> bool {
        self.tex.is_some()
    }

    /// GLES 3.0 降级路径：调色板表扩容（返回是否重建了纹理）。
    pub fn ensure_colors_table(&mut self, device: &Device, count: usize) -> bool {
        match &mut self.tex {
            Some(t) => t.ensure_colors(device, count),
            None => false,
        }
    }

    /// GLES 3.0 降级路径：AR 行表扩容（返回是否重建了纹理）。
    pub fn ensure_rows_table(&mut self, device: &Device, count: usize) -> bool {
        match &mut self.tex {
            Some(t) => t.ensure_rows(device, count),
            None => false,
        }
    }

    /// GLES 3.0 降级路径：上传调色板纹理（`tex` 为 None 时无操作）。
    pub fn write_colors_table(&self, queue: &Queue, colors: &[[f32; 4]]) {
        if let Some(t) = &self.tex {
            t.write_colors(queue, colors);
        }
    }

    /// GLES 3.0 降级路径：上传 AR 行表纹理（offsets/heights 平行数组）。
    pub fn write_rows_table(&self, queue: &Queue, offsets: &[f32], heights: &[f32]) {
        if let Some(t) = &self.tex {
            t.write_rows(queue, offsets, heights);
        }
    }
}

/// storage 变体的 group 0 布局：uniform + 调色板/行高/排除表 storage buffer。
fn create_storage_layout(device: &Device) -> BindGroupLayout {
    let storage = |binding: u32, visibility: ShaderStages| BindGroupLayoutEntry {
        binding,
        visibility,
        ty: BindingType::Buffer {
            ty: BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let uniform = |binding: u32| BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
        ty: BindingType::Buffer {
            ty: BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("render_bind_group_layout"),
        entries: &[
            uniform(0),
            storage(1, ShaderStages::VERTEX | ShaderStages::FRAGMENT),
            uniform(2),
            storage(3, ShaderStages::VERTEX | ShaderStages::FRAGMENT),
            // 排除表只在顶点阶段读（`note_geometry` 选区判定）。
            storage(4, ShaderStages::VERTEX | ShaderStages::FRAGMENT),
            storage(5, ShaderStages::VERTEX),
        ],
    })
}

/// texture 变体（GLES 3.0）的 group 0 布局：uniform + 两张 RGBA32F 数据表 +
/// 非过滤采样器。纹理表不是 filterable 格式，采样器必须是 NonFiltering。
fn create_texture_layout(device: &Device) -> BindGroupLayout {
    let table = |binding: u32| BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::VERTEX,
        ty: BindingType::Texture {
            sample_type: TextureSampleType::Float { filterable: false },
            view_dimension: TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("render_bind_group_layout_gles3"),
        entries: &[
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            table(1),
            BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            table(3),
            BindGroupLayoutEntry {
                binding: 4,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Sampler(SamplerBindingType::NonFiltering),
                count: None,
            },
        ],
    })
}

/// 按当前变体创建 group 0（`tex` 为 Some 时用数据纹理，否则用 storage buffer）。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
fn create_bind_group(
    device: &Device,
    layout: &BindGroupLayout,
    tex: Option<&TextureTable>,
    uniform_buffer: &TrackedBuffer,
    track_colors_buffer: &TrackedBuffer,
    selection_buffer: &TrackedBuffer,
    track_offsets_buffer: &TrackedBuffer,
    exclude_buffer: &TrackedBuffer,
    row_heights_buffer: &TrackedBuffer,
) -> BindGroup {
    fn buffer_entry(binding: u32, buffer: &TrackedBuffer) -> BindGroupEntry<'_> {
        BindGroupEntry {
            binding,
            resource: buffer.as_entire_binding(),
        }
    }
    let entries = match tex {
        None => vec![
            buffer_entry(0, uniform_buffer),
            buffer_entry(1, track_colors_buffer),
            buffer_entry(2, selection_buffer),
            buffer_entry(3, track_offsets_buffer),
            buffer_entry(4, exclude_buffer),
            buffer_entry(5, row_heights_buffer),
        ],
        Some(t) => vec![
            buffer_entry(0, uniform_buffer),
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::TextureView(&t.colors.view),
            },
            buffer_entry(2, selection_buffer),
            BindGroupEntry {
                binding: 3,
                resource: BindingResource::TextureView(&t.rows.view),
            },
            BindGroupEntry {
                binding: 4,
                resource: BindingResource::Sampler(&t.sampler),
            },
        ],
    };
    device.create_bind_group(&BindGroupDescriptor {
        label: Some("render_bind_group"),
        layout,
        entries: &entries,
    })
}
