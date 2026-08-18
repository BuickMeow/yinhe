//! GPU 资源 RAII 包装：创建时自动计入显存统计（memtrace），Drop 时自动注销。
//!
//! 原代码在每处 `create_buffer` 后手动调用
//! `yinhe_memtrace::add_gpu_resource`，并在替换/释放 buffer 时手动
//! `sub_gpu_resource`——漏写就会造成显存统计漂移（例如 renderer.rs 旧版
//! `ensure_track_colors_capacity` 只 add 新 buffer 却忘记 sub 旧 buffer）。
//! 包装后记账与资源生命周期绑定，无需手工维护。

use wgpu::*;

/// 显存记账的 Buffer 包装。
///
/// `Deref<Target = wgpu::Buffer>`，因此传 `&Buffer` 的 API（`queue.write_buffer`、
/// `slice()`、`as_entire_binding()`、`size()`）都可直接使用 `&TrackedBuffer`。
/// 有意不实现 `Clone`：一份显存资源只记一次账。
pub struct TrackedBuffer {
    buffer: Buffer,
    size: u64,
}

impl TrackedBuffer {
    /// 创建 GPU buffer 并计入显存统计。
    pub fn new(device: &Device, desc: &BufferDescriptor) -> Self {
        let buffer = device.create_buffer(desc);
        yinhe_memtrace::add_gpu_resource(desc.size);
        Self {
            buffer,
            size: desc.size,
        }
    }

    /// 分配的字节数（含稀疏槽位 padding 等）。
    pub fn size(&self) -> u64 {
        self.size
    }

    /// 取出底层 `wgpu::Buffer` 的引用。
    pub fn inner(&self) -> &Buffer {
        &self.buffer
    }
}

impl std::ops::Deref for TrackedBuffer {
    type Target = Buffer;

    fn deref(&self) -> &Buffer {
        &self.buffer
    }
}

impl Drop for TrackedBuffer {
    fn drop(&mut self) {
        yinhe_memtrace::sub_gpu_resource(self.size);
    }
}

/// 显存记账的 Texture 包装（语义同 `TrackedBuffer`）。
///
/// 当前 crate 尚无纹理创建点，供未来的纹理渲染器（如轨道缩略图、波形图）
/// 直接使用。
pub struct TrackedTexture {
    texture: Texture,
    size: u64,
}

impl TrackedTexture {
    /// 创建 GPU texture 并计入显存统计（按格式块大小估算字节数）。
    pub fn new(device: &Device, desc: &TextureDescriptor) -> Self {
        let texture = device.create_texture(desc);
        // block_copy_size 对压缩格式返回 None，按 4 字节/像素保守估算。
        let pixel_size = desc.format.block_copy_size(None).unwrap_or(4) as u64;
        let size = pixel_size
            * desc.size.width as u64
            * desc.size.height as u64
            * desc.size.depth_or_array_layers as u64;
        yinhe_memtrace::add_gpu_resource(size);
        Self { texture, size }
    }

    /// 分配的字节数（估算值）。
    pub fn size(&self) -> u64 {
        self.size
    }

    /// 创建纹理视图。
    pub fn create_view(&self, desc: &TextureViewDescriptor) -> TextureView {
        self.texture.create_view(desc)
    }
}

impl std::ops::Deref for TrackedTexture {
    type Target = Texture;

    fn deref(&self) -> &Texture {
        &self.texture
    }
}

impl Drop for TrackedTexture {
    fn drop(&mut self) {
        yinhe_memtrace::sub_gpu_resource(self.size);
    }
}

/// 显存预算守卫：分配前检查，超限返回错误而非让 wgpu panic。
///
/// wgpu 的 `create_buffer` 在 size 超过 `max_buffer_size` 时会直接 panic
/// （例如单 key 音符 buffer 超过设备上限），且累计显存没有查询 API——
/// 只能靠预算登记。此处做两道硬检查（无阈值分级）：
///   1. 单次分配不超过设备 `max_buffer_size`（wgpu panic 阈值）。
///   2. 全局已分配（memtrace 统计）+ 本次分配不超过总预算。
pub struct GpuBudget {
    /// 单次分配硬上限（wgpu 单 buffer 上限）。
    per_alloc_limit: u64,
    /// 累计显存硬上限（总预算）。
    total_limit: u64,
}

/// 默认总预算：8 GiB。100M 音符全量 all_notes + visible_notes ≈ 3.2GB，
/// 留出编辑/多视图余量。
const DEFAULT_TOTAL_LIMIT: u64 = 8 * 1024 * 1024 * 1024;

impl GpuBudget {
    /// 从设备 limits 构造预算。
    pub fn new(device: &Device) -> Self {
        Self {
            per_alloc_limit: device.limits().max_buffer_size,
            total_limit: DEFAULT_TOTAL_LIMIT,
        }
    }

    /// 检查分配 `size` 字节是否安全。
    ///
    /// - 单次超过 `max_buffer_size` → `AllocationTooLarge`（wgpu 会 panic 的点）。
    /// - 累计超过总预算 → `OutOfMemory`。
    pub fn reserve(&self, size: u64) -> Result<(), GpuBudgetError> {
        if size > self.per_alloc_limit {
            return Err(GpuBudgetError::AllocationTooLarge {
                requested: size,
                limit: self.per_alloc_limit,
            });
        }
        let used = yinhe_memtrace::gpu_resource_bytes().max(0) as u64;
        if used.saturating_add(size) > self.total_limit {
            return Err(GpuBudgetError::OutOfMemory {
                requested: size,
                used,
                limit: self.total_limit,
            });
        }
        Ok(())
    }
}

/// GPU 分配预算错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuBudgetError {
    /// 单次分配超过设备单 buffer 上限（wgpu 会 panic 的点）。
    AllocationTooLarge { requested: u64, limit: u64 },
    /// 累计显存超出总预算。
    OutOfMemory {
        requested: u64,
        used: u64,
        limit: u64,
    },
}

impl std::fmt::Display for GpuBudgetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpuBudgetError::AllocationTooLarge { requested, limit } => {
                write!(f, "GPU 单次分配 {requested} 字节超过设备上限 {limit} 字节")
            }
            GpuBudgetError::OutOfMemory {
                requested,
                used,
                limit,
            } => write!(
                f,
                "GPU 显存不足：需要 {requested} 字节，已用 {used} 字节，预算 {limit} 字节"
            ),
        }
    }
}

impl std::error::Error for GpuBudgetError {}
