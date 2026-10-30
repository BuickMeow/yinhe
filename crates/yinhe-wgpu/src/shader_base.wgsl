// ═══ 共享前缀：常量、uniform 结构、选区判定 ═══════════════════════════
// 由 pipeline.rs 与变体前缀（shader_storage/shader_texture）和主体
// （shader_body.wgsl）拼接成完整 WGSL。两套变体只在「调色板/行高表」
// 的取数方式上不同：storage buffer（GLES3.1+）或数据纹理（GLES3.0）。

// ── Rendering constants ───────────────────────────────────────────────────
const BORDER_DARKEN_FACTOR: f32 = 0.4;
const MAX_SEL_RECTS: u32 = 32u;
// 填充色的力度淡化上限：力度 0 → 向背景方向混合 50%，力度 127 → 原色。
// 方向随主题翻转（暗色向黑、亮色向白），保留色相。仅用于 PR 音符（mode==1）；
// ghost 层 vel=127 保持原色。
const MAX_FILL_FADE: f32 = 0.5;
// 选中填充：向「与背景对比最强的一极」混合的比例（暗色偏白、亮色偏黑）。
const SELECTED_CONTRAST: f32 = 0.7;

// 目标附着是否为 sRGB（pipeline override：1=是）。项目内所有颜色（调色板、
// 主题、打包的装饰色）都以 sRGB 分量表示；写入 sRGB 附着前必须先转线性，
// 否则硬件会再编码一次，整体变淡。
override SRGB_TARGET: u32 = 0u;

/// sRGB 分量转线性（逐通道；alpha 不变）。
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(lo, hi, c > vec3<f32>(0.04045));
}

/// UNORM8 打包色（R|G<<8|B<<16|A<<24）→ sRGB 分量。
fn unpack_rgba(packed: u32) -> vec4<f32> {
    return vec4<f32>(
        f32((packed >> 0u) & 0xFFu) / 255.0,
        f32((packed >> 8u) & 0xFFu) / 255.0,
        f32((packed >> 16u) & 0xFFu) / 255.0,
        f32((packed >> 24u) & 0xFFu) / 255.0,
    );
}

/// 按目标附着做颜色空间转换（仅在 sRGB 附着时生效）。
fn to_target_space(c: vec4<f32>) -> vec4<f32> {
    if SRGB_TARGET == 1u {
        return vec4<f32>(srgb_to_linear(c.rgb), c.a);
    }
    return c;
}

struct Uniforms {
    width: f32,
    height: f32,
    scroll_x: f32,
    scroll_y: f32,
    pixels_per_tick: f32,
    key_height: f32,
    keyboard_width: f32,
    mode: u32, // 0=pixel, 1=PR notes(tick→pixel+rounding), 2=AR notes(tick→pixel)
    min_border_width: f32,
    track_count: u32, // number of valid tracks in track_colors
    sel_rect_count: u32, // number of valid selection rects
    note_outline: u32, // 0=no outline (saves fill rate), 1=on
    lane_height: f32, // AR: per-track lane height (PR unused)
    value_zoom: f32, // Automation panel: vertical zoom
    value_scroll: f32, // Automation panel: vertical scroll in value space
    orientation: u32, // PR 视图方向：0=横向（时间轴=X，音高=Y），1=纵向瀑布流（时间轴=Y，音高=X）

    // 选区属性筛选（与 Rust `SelectionFilter` 一致；未设置边界 = flags 位为 0）
    filter_key: u32,
    filter_track: u32,
    filter_velocity: u32,
    filter_gate_lo: u32,
    filter_gate_hi: u32,
    filter_flags: u32, // bit0=key, bit1=track, bit2=velocity, bit3=gate, bit4=invert
    exclude_mask: u32, // 排除表掩码（容量-1；0 = 禁用）
    dark_theme: u32, // 1=暗色主题（力度淡化/选中高对比方向翻转）
    ghost_dx: f32, // ghost 预览动画像素偏移（仅 GHOST_BIT 实例）
    ghost_dy: f32,
    ghost_pad0: f32,
    ghost_pad1: f32,
}

struct SelectionUniform {
    rects: array<vec4<u32>, MAX_SEL_RECTS * 2u>, // 2 vec4 per rect: (tick_start, tick_end, key_lo, key_hi) + (track_lo, track_hi, 0, 0)
}

// 选中矩形（binding 2）：GPU cull 全曲层实例不随选区重传（bit31 恒 0），
// 由此 uniform 在顶点着色器实时判定补选中位。
@group(0) @binding(2)
var<uniform> selection: SelectionUniform;

/// 选区属性筛选（与 CPU `SelectionFilter::accepts_note` 同语义）。
/// 无任何边界（flags==0）时恒通过（invert 无对象，与 CPU 一致）。
fn selection_filter_passes(key: u32, track: u32, vel: u32, gate: u32) -> bool {
    if u.filter_flags == 0u {
        return true;
    }
    var m = true;
    if (u.filter_flags & 1u) != 0u {
        m = m && key >= (u.filter_key & 0xFFu) && key <= ((u.filter_key >> 8u) & 0xFFu);
    }
    if (u.filter_flags & 2u) != 0u {
        m = m && track >= (u.filter_track & 0xFFFFu) && track <= (u.filter_track >> 16u);
    }
    if (u.filter_flags & 4u) != 0u {
        m = m && vel >= (u.filter_velocity & 0xFFu) && vel <= ((u.filter_velocity >> 8u) & 0xFFu);
    }
    if (u.filter_flags & 8u) != 0u {
        m = m && gate >= u.filter_gate_lo && gate <= u.filter_gate_hi;
    }
    let invert = (u.filter_flags & 16u) != 0u;
    return m != invert;
}
/// 矩形选区命中（tick 半开、key/track 闭区间，与 CPU `Selection::contains` 一致）。
/// GPU 侧为矩形 + 属性筛选判定；成员态另由排除表（CPU 位图差集）修正。
fn in_selection_rects(start_tick: u32, key: u32, track: u32) -> bool {
    let n = min(u.sel_rect_count, MAX_SEL_RECTS);
    for (var i = 0u; i < n; i = i + 1u) {
        let a = selection.rects[i * 2u];
        let b = selection.rects[i * 2u + 1u];
        if start_tick >= a.x && start_tick < a.y
            && key >= a.z && key <= a.w
            && track >= b.x && track <= b.y {
            return true;
        }
    }
    return false;
}
// ── 共享 uniform ─────────────────────────────────────────────────────────
@group(0) @binding(0)
var<uniform> u: Uniforms;

@group(0) @binding(2)
var<uniform> sel: SelectionUniform;
