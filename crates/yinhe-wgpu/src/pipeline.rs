use wgpu::*;

use crate::vertex::{CurveInstance, DrawInstance, NoteInstance, Uniforms, SelectionUniform, VelocityBarInstance};

/// Owns the render pipelines and their shared uniform buffers / bind group.
///
/// Four pipelines share the same uniform buffer, track-colors buffer, selection
/// buffer, and bind group:
///   - `pipeline`: decor pipeline (32-byte `DrawInstance` vertex layout, `vs_main`)
///   - `note_pipeline`: note pipeline (16-byte `NoteInstance` vertex layout, `vs_main_note`)
///   - `velocity_pipeline`: velocity bar pipeline (16-byte `VelocityBarInstance` vertex
///     layout, `vs_main_velocity` — unified border-based mode)
///   - `curve_pipeline`: automation curve pipeline (32-byte `CurveInstance` vertex layout,
///     `vs_main_curve`/`fs_main_curve` — per-pixel SDF line/curve)
pub struct RenderPipelineState {
    pub pipeline: RenderPipeline,
    pub note_pipeline: RenderPipeline,
    pub velocity_pipeline: RenderPipeline,
    pub curve_pipeline: RenderPipeline,
    pub uniform_buffer: Buffer,
    pub track_colors_buffer: Buffer,
    /// Current capacity (in vec4 entries) of `track_colors_buffer`.
    pub track_colors_capacity: u32,
    pub selection_buffer: Buffer,
    pub bind_group: BindGroup,
    pub bind_group_layout: BindGroupLayout,
}

impl RenderPipelineState {
    pub fn new(device: &Device, format: TextureFormat, render_shader: &ShaderModule) -> Self {
        // Main uniforms buffer
        let uniform_size = std::mem::size_of::<Uniforms>() as u64;
        let uniform_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("uniforms"),
            size: uniform_size,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        yinhe_memtrace::add_gpu_resource(uniform_size);

        // Track colors buffer — dynamically sized to actual track count.
        // Start with 1 entry (16B); grows on demand via `ensure_track_colors_capacity`.
        // Bound as a read-only storage buffer (runtime-sized array in WGSL).
        let track_colors_size = 16; // 1 × vec4<f32>
        let track_colors_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("track_colors"),
            size: track_colors_size,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        yinhe_memtrace::add_gpu_resource(track_colors_size);

        // Selection rects buffer
        let selection_size = std::mem::size_of::<SelectionUniform>() as u64;
        let selection_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("selection"),
            size: selection_size,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        yinhe_memtrace::add_gpu_resource(selection_size);

        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("render_bind_group_layout"),
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
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
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
            ],
        });

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("render_bind_group"),
            layout: &bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: track_colors_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: selection_buffer.as_entire_binding(),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        // Decor pipeline: 32-byte DrawInstance vertex layout
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("decor_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: render_shader,
                entry_point: Some("vs_main"),
                buffers: &[VertexBufferLayout {
                    array_stride: std::mem::size_of::<DrawInstance>() as u64,
                    step_mode: VertexStepMode::Instance,
                    attributes: &vertex_attr_array![
                        0 => Float32x4,
                        1 => Uint32x4,
                    ],
                }],
                compilation_options: PipelineCompilationOptions::default(),
            },
            fragment: Some(FragmentState {
                module: render_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: PipelineCompilationOptions::default(),
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

        // Note pipeline: 16-byte NoteInstance vertex layout, shares uniforms/bind group
        let note_pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("note_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: render_shader,
                entry_point: Some("vs_main_note"),
                buffers: &[VertexBufferLayout {
                    array_stride: std::mem::size_of::<NoteInstance>() as u64,
                    step_mode: VertexStepMode::Instance,
                    attributes: &vertex_attr_array![
                        0 => Uint32x4,
                    ],
                }],
                compilation_options: PipelineCompilationOptions::default(),
            },
            fragment: Some(FragmentState {
                module: render_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: PipelineCompilationOptions::default(),
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

        // Curve pipeline: 32-byte CurveInstance vertex layout, shares uniforms/bind group.
        // Renders automation segments as per-pixel SDF lines/curves.
        let curve_pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("curve_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: render_shader,
                entry_point: Some("vs_main_curve"),
                buffers: &[VertexBufferLayout {
                    array_stride: std::mem::size_of::<CurveInstance>() as u64,
                    step_mode: VertexStepMode::Instance,
                    attributes: &vertex_attr_array![
                        0 => Float32x4,  // endp (x1, y1, x2, y2) @ offset 0
                        1 => Float32x2,  // params (thickness, tension) @ offset 16
                        2 => Uint32,     // rgba_packed @ offset 24
                        3 => Uint32,     // shape @ offset 28
                    ],
                }],
                compilation_options: PipelineCompilationOptions::default(),
            },
            fragment: Some(FragmentState {
                module: render_shader,
                entry_point: Some("fs_main_curve"),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: PipelineCompilationOptions::default(),
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
        // Unified border-based mode (fill + border), reuses `fs_main` (same VertexOutput).
        let velocity_pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("velocity_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: render_shader,
                entry_point: Some("vs_main_velocity"),
                buffers: &[VertexBufferLayout {
                    array_stride: std::mem::size_of::<VelocityBarInstance>() as u64,
                    step_mode: VertexStepMode::Instance,
                    attributes: &vertex_attr_array![
                        0 => Uint32x4,
                    ],
                }],
                compilation_options: PipelineCompilationOptions::default(),
            },
            fragment: Some(FragmentState {
                module: render_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: PipelineCompilationOptions::default(),
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
            velocity_pipeline,
            curve_pipeline,
            uniform_buffer,
            track_colors_buffer,
            track_colors_capacity: 1,
            selection_buffer,
            bind_group,
            bind_group_layout,
        }
    }
}
