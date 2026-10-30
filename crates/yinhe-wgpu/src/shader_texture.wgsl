// ═══ 变体前缀：数据纹理（GLES 3.0：无 storage buffer）════════════════════
// 鸿蒙模拟器等只有 GLES 3.0 的适配器：顶点阶段既无 SSBO 也无 compute。
// 调色板 / AR 行高表改由 RGBA32F 数据纹理承载（CPU 每帧按需上传，
// 只在内容变化时写），索引 → 纹理坐标的换算在此集中。

/// 数据纹理行宽（须与 Rust `pipeline::TABLE_COLS` 一致）。
const TABLE_COLS: u32 = 1024u;

@group(0) @binding(1)
var track_colors_tex: texture_2d<f32>;

/// 每行 (offset, height)：AR 行顶 y 与行高。
@group(0) @binding(3)
var row_table_tex: texture_2d<f32>;

/// 两张表共用（最近邻采样，`textureSampleLevel` 显式 LOD 在顶点阶段合法）。
@group(0) @binding(4)
var table_sampler: sampler;

/// 按一维索引取数据纹理的纹素（行优先，行宽 = 纹理宽度）。
fn texel_at(t: texture_2d<f32>, idx: u32) -> vec4<f32> {
    let dims = vec2<f32>(textureDimensions(t));
    let xy = vec2<f32>(f32(idx % TABLE_COLS), f32(idx / TABLE_COLS));
    return textureSampleLevel(t, table_sampler, (xy + vec2<f32>(0.5)) / dims, 0.0);
}

// ── 取数访问器（与 shader_storage.wgsl 同名同语义）──────────────────────

fn track_color_or(track: u32, fallback: vec4<f32>) -> vec4<f32> {
    if track < u.track_count {
        return texel_at(track_colors_tex, track);
    }
    return fallback;
}

fn row_height(track: u32) -> f32 {
    return texel_at(row_table_tex, min(track, u.track_count - 1u)).y;
}

fn track_offset(track: u32) -> f32 {
    return texel_at(row_table_tex, min(track, u.track_count - 1u)).x;
}

/// GLES 3.0 路径没有排除表（1KB 级哈希表无法进 uniform）：选区命中的
/// 矩形内音符一律高亮。成员态（矩形内非成员）在模拟器上会显示为全选，
/// 真机 storage 路径不受影响。
fn in_exclude_table(lo: u32, hi: u32) -> bool {
    return false;
}
