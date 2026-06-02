use std::collections::HashSet;
use wgpu::*;

use crate::instances;
use crate::pipeline::RenderPipelineState;
use crate::vertex::{NoteInstance, Uniforms};
use crate::view::PianoRollView;

/// Maximum instances per draw call.
const MAX_INSTANCE_COUNT: usize = 6_000_000;
/// Minimum instance buffer capacity (in number of instances).
const MIN_INSTANCE_BUFFER_CAPACITY: usize = 4096;

/// Instance buffer with tracking of capacity.
struct InstanceBufferSlot {
    buffer: Buffer,
    capacity_instances: usize,
}

fn create_instance_buffer_slot(device: &Device, instance_size: u64, capacity: usize) -> InstanceBufferSlot {
    let buffer = device.create_buffer(&BufferDescriptor {
        label: Some("instance_buffer"),
        size: instance_size * capacity as u64,
        usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    InstanceBufferSlot {
        buffer,
        capacity_instances: capacity,
    }
}

fn next_instance_capacity(required: usize) -> usize {
    let mut cap = MIN_INSTANCE_BUFFER_CAPACITY;
    while cap < required {
        cap *= 2;
    }
    cap
}

/// Piano roll wgpu renderer.
pub struct PianorollRenderer {
    device: Device,
    queue: Queue,
    render: RenderPipelineState,
    instance_buffers: Vec<InstanceBufferSlot>,
    instance_scratch: Vec<NoteInstance>,
    current_batch_counts: Vec<usize>,
}

impl PianorollRenderer {
    pub fn new(device: Device, queue: Queue, format: TextureFormat) -> Self {
        let render_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("pianoroll_shader"),
            source: ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let render = RenderPipelineState::new(&device, format, &render_shader);

        Self {
            device,
            queue,
            render,
            instance_buffers: Vec::new(),
            instance_scratch: Vec::new(),
            current_batch_counts: Vec::new(),
        }
    }

    /// Prepare rendering data for the current frame.
    pub fn prepare(
        &mut self,
        width: u32,
        height: u32,
        midi: Option<&dyn yinhe_types::NoteSource>,
        view: &PianoRollView,
        selected: &HashSet<(u16, u32)>,
    ) {
        let uniforms = Uniforms {
            width: width as f32,
            height: height as f32,
            scroll_x: view.scroll_x,
            scroll_y: view.scroll_y,
            pixels_per_tick: view.pixels_per_tick,
            key_height: view.key_height,
            keyboard_width: view.keyboard_width,
            _pad: 0.0,
        };
        self.queue
            .write_buffer(&self.render.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let mut instances = std::mem::take(&mut self.instance_scratch);
        instances.clear();

        instances::build_instances(&mut instances, width, height, midi, view, selected);

        let instance_size = std::mem::size_of::<NoteInstance>() as u64;
        let batches: Vec<&[NoteInstance]> = if instances.is_empty() {
            Vec::new()
        } else {
            instances.chunks(MAX_INSTANCE_COUNT).collect()
        };

        self.current_batch_counts.clear();
        for batch in &batches {
            self.current_batch_counts.push(batch.len());
        }

        while self.instance_buffers.len() > batches.len() {
            self.instance_buffers.pop();
        }
        while self.instance_buffers.len() < batches.len() {
            self.instance_buffers
                .push(create_instance_buffer_slot(&self.device, instance_size, MIN_INSTANCE_BUFFER_CAPACITY));
        }
        for (i, batch) in batches.iter().enumerate() {
            let required_instances = batch.len().max(1);
            if self.instance_buffers[i].capacity_instances < required_instances {
                self.instance_buffers[i] = create_instance_buffer_slot(
                    &self.device,
                    instance_size,
                    next_instance_capacity(required_instances),
                );
            }
            self.queue
                .write_buffer(&self.instance_buffers[i].buffer, 0, bytemuck::cast_slice(batch));
        }

        instances.clear();
        self.instance_scratch = instances;
    }

    /// Draw the prepared instances into the given render target.
    pub fn draw(
        &self,
        encoder: &mut CommandEncoder,
        target: &TextureView,
        width: u32,
        height: u32,
    ) {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("pianoroll_pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: Operations {
                    load: LoadOp::Clear(Color {
                        r: 0.12,
                        g: 0.12,
                        b: 0.14,
                        a: 1.0,
                    }),
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });

        pass.set_viewport(0.0, 0.0, width as f32, height as f32, 0.0, 1.0);

        if !self.instance_buffers.is_empty() && !self.current_batch_counts.is_empty() {
            pass.set_pipeline(&self.render.pipeline);
            pass.set_bind_group(0, &self.render.bind_group, &[]);
            for (i, &count) in self.current_batch_counts.iter().enumerate() {
                pass.set_vertex_buffer(0, self.instance_buffers[i].buffer.slice(..));
                pass.draw(0..6, 0..count as u32);
            }
        }
    }

    /// Total number of note instances prepared for the current frame.
    pub fn total_instances(&self) -> usize {
        self.current_batch_counts.iter().sum()
    }
}
