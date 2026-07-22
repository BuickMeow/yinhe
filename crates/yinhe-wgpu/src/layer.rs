use bytemuck::Pod;
use wgpu::*;

use crate::vertex::{CurveInstance, DrawInstance, NoteInstance, VelocityBarInstance};

const MIN_CAPACITY: usize = 4096;
/// Maximum instances per GPU buffer chunk.
/// 4M × 32 bytes = 128 MB, well under the 256 MB wgpu limit.
const MAX_PER_CHUNK: usize = 4_000_000;

fn grow_capacity(required: usize) -> usize {
    crate::util::next_capacity(required, MIN_CAPACITY)
}

struct BufferChunk {
    buffer: Buffer,
    capacity: usize,
    size_bytes: u64,
}

impl Drop for BufferChunk {
    fn drop(&mut self) {
        yinhe_memtrace::sub_gpu_resource(self.size_bytes);
    }
}

impl BufferChunk {
    fn ensure_capacity(&mut self, device: &Device, required: usize, instance_size: u64) {
        if required <= self.capacity {
            return;
        }
        let new_cap = grow_capacity(required).min(MAX_PER_CHUNK);
        let new_size = instance_size * new_cap as u64;
        let new_buffer = crate::util::create_vertex_buffer(device, "layer_buffer", new_size);
        yinhe_memtrace::sub_gpu_resource(self.size_bytes);
        self.buffer = new_buffer;
        self.capacity = new_cap;
        self.size_bytes = new_size;
    }
}

/// A single GPU instance buffer layer with built-in caching and scratch reuse.
///
/// Each layer holds chunked GPU buffers, a scratch `Vec<T>` for building, and a
/// `cache_key` that controls whether `upload()` actually rebuilds.  When
/// instances exceed `MAX_PER_CHUNK`, additional chunks are created
/// automatically.  Layers are drawn in index order (lowest = bottom).
///
/// Generic over the instance type `T` (e.g. `DrawInstance` for decor, or
/// `NoteInstance` for notes) so that both 32-byte and 16-byte instance layouts
/// share the same caching/chunking logic.
pub struct LayerSlot<T: Pod> {
    chunks: Vec<BufferChunk>,
    scratch: Vec<T>,
    cache_key: u64,
    count: usize,
}

impl<T: Pod> LayerSlot<T> {
    pub fn new(device: &Device) -> Self {
        let instance_size = std::mem::size_of::<T>() as u64;
        let cap = MIN_CAPACITY;
        let size = instance_size * cap as u64;
        let buffer = crate::util::create_vertex_buffer(device, "layer_buffer", size);
        Self {
            chunks: vec![BufferChunk {
                buffer,
                capacity: cap,
                size_bytes: size,
            }],
            scratch: Vec::new(),
            cache_key: 0,
            count: 0,
        }
    }

    /// Upload with cache: skips rebuild if `cache_key` matches the previous call.
    /// Returns `true` if the layer was actually rebuilt.
    pub fn upload(
        &mut self,
        device: &Device,
        queue: &Queue,
        cache_key: u64,
        build: impl FnOnce(&mut Vec<T>),
    ) -> bool {
        if cache_key == self.cache_key {
            return false;
        }
        self.scratch.clear();
        build(&mut self.scratch);
        self.cache_key = cache_key;
        self.count = self.scratch.len();
        self.flush(device, queue);
        true
    }

    /// Force rebuild (ignore cache).
    pub fn upload_force(
        &mut self,
        device: &Device,
        queue: &Queue,
        build: impl FnOnce(&mut Vec<T>),
    ) {
        self.scratch.clear();
        build(&mut self.scratch);
        self.count = self.scratch.len();
        self.flush(device, queue);
    }

    fn ensure_chunks(&mut self, device: &Device, required: usize) {
        let instance_size = std::mem::size_of::<T>() as u64;
        let needed = required.div_ceil(MAX_PER_CHUNK);
        while self.chunks.len() > needed {
            self.chunks.pop();
        }
        while self.chunks.len() < needed {
            let cap = MIN_CAPACITY;
            let size = instance_size * cap as u64;
            let buffer = crate::util::create_vertex_buffer(device, "layer_buffer", size);
            self.chunks.push(BufferChunk {
                buffer,
                capacity: cap,
                size_bytes: size,
            });
        }
    }

    fn flush(&mut self, device: &Device, queue: &Queue) {
        if self.count == 0 {
            return;
        }
        let instance_size = std::mem::size_of::<T>() as u64;
        self.ensure_chunks(device, self.count);
        for (i, chunk_instances) in self.scratch[..self.count].chunks(MAX_PER_CHUNK).enumerate() {
            let chunk = &mut self.chunks[i];
            chunk.ensure_capacity(device, chunk_instances.len(), instance_size);
            queue.write_buffer(&chunk.buffer, 0, bytemuck::cast_slice(chunk_instances));
        }
    }

    /// Draw this layer into an active render pass.
    pub fn draw<'a>(&self, pass: &mut RenderPass<'a>, vertex_slot: u32) {
        if self.count == 0 {
            return;
        }
        let mut remaining = self.count;
        for chunk in &self.chunks {
            let batch_count = remaining.min(MAX_PER_CHUNK);
            if batch_count == 0 {
                break;
            }
            pass.set_vertex_buffer(vertex_slot, chunk.buffer.slice(..));
            pass.draw(0..6, 0..batch_count as u32);
            remaining -= batch_count;
        }
    }

    pub fn instance_count(&self) -> usize {
        self.count
    }
}

/// Combine a list of u64 values into a single cache key.
///
/// Callers use this to produce a key that captures all the dependencies
/// of a layer.  Example:
///
/// ```ignore
/// let key = layer_cache_key(&[
///     scroll_x.to_bits() as u64,
///     pixels_per_tick.to_bits() as u64,
///     time_sig_hash,
/// ]);
/// ```
pub fn layer_cache_key(parts: &[u64]) -> u64 {
    let mut h: u64 = 0;
    for &p in parts {
        h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(p);
    }
    h
}

// ── Type-erased layer ──

/// Layer kind: decor (32B `DrawInstance`), note (16B `NoteInstance`),
/// velocity (16B `VelocityBarInstance`), or curve (32B `CurveInstance` — automation SDF lines/curves).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerKind {
    Decor,
    Note,
    Velocity,
    Curve,
}

/// Type-erased layer slot that can hold `DrawInstance`, `NoteInstance`, `VelocityBarInstance`, or `CurveInstance`.
pub enum AnyLayer {
    Decor(LayerSlot<DrawInstance>),
    Note(LayerSlot<NoteInstance>),
    Velocity(LayerSlot<VelocityBarInstance>),
    Curve(LayerSlot<CurveInstance>),
}

impl AnyLayer {
    pub(crate) fn new(device: &Device, kind: LayerKind) -> Self {
        match kind {
            LayerKind::Decor => AnyLayer::Decor(LayerSlot::new(device)),
            LayerKind::Note => AnyLayer::Note(LayerSlot::new(device)),
            LayerKind::Velocity => AnyLayer::Velocity(LayerSlot::new(device)),
            LayerKind::Curve => AnyLayer::Curve(LayerSlot::new(device)),
        }
    }

    pub(crate) fn kind(&self) -> LayerKind {
        match self {
            AnyLayer::Decor(_) => LayerKind::Decor,
            AnyLayer::Note(_) => LayerKind::Note,
            AnyLayer::Velocity(_) => LayerKind::Velocity,
            AnyLayer::Curve(_) => LayerKind::Curve,
        }
    }

    pub(crate) fn draw<'a>(&self, pass: &mut RenderPass<'a>, vertex_slot: u32) {
        match self {
            AnyLayer::Decor(l) => l.draw(pass, vertex_slot),
            AnyLayer::Note(l) => l.draw(pass, vertex_slot),
            AnyLayer::Velocity(l) => l.draw(pass, vertex_slot),
            AnyLayer::Curve(l) => l.draw(pass, vertex_slot),
        }
    }
}
