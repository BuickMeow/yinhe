// ═══ 变体前缀：storage buffer（GLES 3.1+ / Metal / Vulkan）═══════════════
// 真机路径：调色板、行高表、选区排除表都是 storage buffer，顶点着色器
// 直接按索引读取。GLES 3.0（鸿蒙模拟器的 ANGLE/Metal）没有 SSBO，此时
// 改用 shader_texture.wgsl 的数据纹理变体，其余主体代码完全一致。

// Track colors: runtime-sized storage buffer (allocated dynamically to actual
// track count, see pipeline.rs / renderer.rs).
@group(0) @binding(1)
var<storage> tc: array<vec4<f32>>;

// Track offsets: AR 模式下每个音轨主行的音乐坐标 y（像素），由展开状态决定；
// 未展开时等于 track * lane_height。PR 模式下不使用。
@group(0) @binding(3)
var<storage, read> track_offsets: array<f32>;

// Row heights（通道 AR）：每个「行 id」的实际行高。轨道 AR 全行 = lane_height，
// 通道 AR 展开的通道合并行变窄为半高，音符 key 空间随之压缩。PR 不使用。
@group(0) @binding(5)
var<storage, read> row_heights: array<f32>;

// 排除表（binding 4）：成员态下「矩形内但不在成员位图」的音符键
// （lo = key | track<<8，hi = start_tick），开放寻址、空槽 = u32::MAX。
// 哈希函数必须与 CPU `exclude_hash` 一致。
@group(0) @binding(4)
var<storage, read> exclude_table: array<vec2<u32>>;

fn in_exclude_table(lo: u32, hi: u32) -> bool {
    if u.exclude_mask == 0u {
        return false;
    }
    // WGSL 整数乘法即 wrapping（与 CPU 的 wrapping_mul 一致）。
    var slot = ((lo * 2654435761u) ^ (hi * 2246822519u)) & u.exclude_mask;
    for (var i = 0u; i <= u.exclude_mask; i = i + 1u) {
        let e = exclude_table[slot];
        if e.x == 0xFFFFFFFFu {
            return false; // 空槽（键的 lo 高位恒 0，不可能为 MAX）
        }
        if e.x == lo && e.y == hi {
            return true;
        }
        slot = (slot + 1u) & u.exclude_mask;
    }
    return false;
}
/// GPU storage 布局的 NoteInstance（与 cull.wgsl / Rust `NoteInstance` 一致）。
/// all_instances（@group(1)）的元素类型。
struct NoteData {
    start_tick: u32,
    end_tick: u32,
    packed: u32,
}

// GPU-cull 路径：vertex buffer 每实例 4B u32 索引，从 per-key 的顶点专用
// bind group (@group(1)) 的 all_instances 间接读回完整 NoteInstance。
// visible 槽位从 12B 降到 4B（全曲稀疏槽显存 -67%）。
@group(1) @binding(0)
var<storage, read> all_instances: array<NoteData>;

/// GPU cull 路径：4B 可见索引 → all_instances 间接读回完整实例。
@vertex
fn vs_main_note(
    @builtin(vertex_index) vertex_index: u32,
    @location(0) idx: u32,
) -> VertexOutput {
    let inst = all_instances[idx];
    return note_geometry(vertex_index, inst.start_tick, inst.end_tick, inst.packed);
}

// ── 取数访问器（纹理变体须同名同语义）────────────────────────────────────

/// 轨道色：超出有效轨道数时用 `fallback`（PR 装饰实例自带 packed rgba）。
fn track_color_or(track: u32, fallback: vec4<f32>) -> vec4<f32> {
    if track < u.track_count {
        return tc[track];
    }
    return fallback;
}

/// AR 行高（通道 AR 的窄合并行变窄）；越界时取最后一个槽位。
fn row_height(track: u32) -> f32 {
    return row_heights[min(track, arrayLength(&row_heights) - 1u)];
}

/// AR 行顶 y（音乐坐标像素）；越界时取最后一个槽位。
fn track_offset(track: u32) -> f32 {
    return track_offsets[min(track, arrayLength(&track_offsets) - 1u)];
}
