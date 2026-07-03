use wgpu::*;

use crate::vertex::{DrawInstance, NoteInstance, Uniforms, TrackColorsUniform, SelectionUniform};

/// Owns the render pipelines and their shared uniform buffers / bind group.
///
/// Two pipelines share the same uniform buffer, track-colors buffer, selection
/// buffer, and bind group:
///   - `pipeline`: decor pipeline (32-byte `DrawInstance` vertex layout, `vs_main`)
///   - `note_pipeline`: note pipeline (16-byte `NoteInstance` vertex layout, `vs_main_note`)
pub struct RenderPipelineState {
    pub pipeline: RenderPipeline,
    pub note_pipeline: RenderPipeline,
    pub uniform_buffer: Buffer,
    pub track_colors_buffer: Buffer,
    pub selection_buffer: Buffer,
    pub bind_group: BindGroup,
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

        // Track colors buffer
        let track_colors_size = std::mem::size_of::<TrackColorsUniform>() as u64;
        let track_colors_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("track_colors"),
            size: track_colors_size,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
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
                        ty: BufferBindingType::Uniform,
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

        Self {
            pipeline,
            note_pipeline,
            uniform_buffer,
            track_colors_buffer,
            selection_buffer,
            bind_group,
        }
    }
}
