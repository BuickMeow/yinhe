use wgpu::*;

use crate::vertex::NoteInstance;

const MIN_CAPACITY: usize = 4096;

fn grow_capacity(required: usize) -> usize {
    let mut cap = MIN_CAPACITY;
    while cap < required {
        cap *= 2;
    }
    cap
}

/// A single GPU instance buffer layer with built-in caching and scratch reuse.
///
/// Each layer holds its own GPU buffer, a scratch `Vec<NoteInstance>` for
/// building, and a `cache_key` that controls whether `upload()` actually
/// rebuilds.  Layers are drawn in index order (lowest index = bottom).
pub struct LayerSlot {
    buffer: Buffer,
    capacity: usize,
    size_bytes: u64,
    scratch: Vec<NoteInstance>,
    cache_key: u64,
    count: usize,
}

impl Drop for LayerSlot {
    fn drop(&mut self) {
        yinhe_memtrace::sub_gpu_resource(self.size_bytes);
    }
}

impl LayerSlot {
    pub fn new(device: &Device) -> Self {
        let instance_size = std::mem::size_of::<NoteInstance>() as u64;
        let cap = MIN_CAPACITY;
        let size = instance_size * cap as u64;
        let buffer = yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Gpu, || {
            device.create_buffer(&BufferDescriptor {
                label: Some("layer_buffer"),
                size,
                usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        yinhe_memtrace::add_gpu_resource(size);
        Self {
            buffer,
            capacity: cap,
            size_bytes: size,
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
        build: impl FnOnce(&mut Vec<NoteInstance>),
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
        build: impl FnOnce(&mut Vec<NoteInstance>),
    ) {
        self.scratch.clear();
        build(&mut self.scratch);
        self.count = self.scratch.len();
        self.flush(device, queue);
    }

    /// Direct write from an existing slice (no closure, no cache).
    pub fn upload_slice(&mut self, device: &Device, queue: &Queue, instances: &[NoteInstance]) {
        self.count = instances.len();
        if self.count == 0 {
            return;
        }
        self.ensure_buffer(device, self.count);
        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(instances));
    }

    fn ensure_buffer(&mut self, device: &Device, required: usize) {
        if required <= self.capacity {
            return;
        }
        let instance_size = std::mem::size_of::<NoteInstance>() as u64;
        let new_cap = grow_capacity(required);
        let new_size = instance_size * new_cap as u64;
        let new_buffer = yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Gpu, || {
            device.create_buffer(&BufferDescriptor {
                label: Some("layer_buffer"),
                size: new_size,
                usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        yinhe_memtrace::add_gpu_resource(new_size);
        yinhe_memtrace::sub_gpu_resource(self.size_bytes);
        self.buffer = new_buffer;
        self.capacity = new_cap;
        self.size_bytes = new_size;
    }

    fn flush(&mut self, device: &Device, queue: &Queue) {
        if self.count == 0 {
            return;
        }
        self.ensure_buffer(device, self.count);
        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.scratch[..self.count]));
    }

    /// Draw this layer into an active render pass.
    pub fn draw<'a>(&self, pass: &mut RenderPass<'a>, vertex_slot: u32) {
        if self.count == 0 {
            return;
        }
        pass.set_vertex_buffer(vertex_slot, self.buffer.slice(..));
        pass.draw(0..6, 0..self.count as u32);
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
