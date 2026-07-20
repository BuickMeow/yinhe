// ── Rendering constants ───────────────────────────────────────────────────
const BORDER_DARKEN_FACTOR: f32 = 0.4;
const MAX_SEL_RECTS: u32 = 32u;

struct Uniforms {
    width: f32,
    height: f32,
    scroll_x: f32,
    scroll_y: f32,
    pixels_per_tick: f32,
    key_height: f32,
    keyboard_width: f32,
    mode: u32, // 0=pixel, 1=PR notes(tick→pixel+rounding), 2=AR notes(tick→pixel)
    scroll_frac: f32, // fractional part of scroll_x for sub-pixel NDC offset
    scroll_mode: u32, // 0=原始, 1=整数对齐, 2=子像素偏移
    min_border_width: f32,
    track_count: u32, // number of valid tracks in track_colors
    sel_rect_count: u32, // number of valid selection rects
    note_outline: u32, // 0=no outline (saves fill rate), 1=on
    lane_height: f32, // AR: per-track lane height (PR unused)
    value_zoom: f32, // Automation panel: vertical zoom
    value_scroll: f32, // Automation panel: vertical scroll in value space
}

// Track colors: runtime-sized storage buffer (allocated dynamically to actual
// track count, see pipeline.rs / renderer.rs).
@group(0) @binding(1)
var<storage> tc: array<vec4<f32>>;

struct SelectionUniform {
    rects: array<vec4<u32>, MAX_SEL_RECTS * 2u>, // 2 vec4 per rect: (tick_start, tick_end, key_lo, key_hi) + (track_lo, track_hi, 0, 0)
}

struct DrawInstance {
    @location(0) xywh: vec4<f32>,
    @location(1) packed: vec4<u32>,  // x=rgba(UNORM8), y=props(2xf16), z=velocity, w=tag
}

struct NoteInstance {
    @location(0) data: vec4<u32>,  // x=start_tick, y=end_tick, z=packed(key|track|vel), w=reserved
}

struct VelocityBarInstance {
    @location(0) data: vec4<u32>,  // x=tick, y=length, z=packed(track|velocity), w=reserved
}

/// Curve/line/anchor instance (36 bytes).
/// See `CurveInstance` in vertex.rs for the CPU-side layout.
struct CurveInstance {
    @location(0) endp: vec4<f32>,    // (x1, y1, x2, y2)
    @location(1) params: vec3<f32>,  // (thickness, ctrl_x, ctrl_y)
    @location(2) rgba: u32,          // UNORM8: R|G<<8|B<<16|A<<24
    @location(3) shape: u32,         // 0 = bezier, 1 = filled circle, 2 = filled square, 3 = hollow circle
}

struct CurveOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local: vec2<f32>,     // pixel-space position
    @location(1) p1: vec2<f32>,
    @location(2) p2: vec2<f32>,
    @location(3) thickness: f32,
    @location(4) ctrl: vec2<f32>,      // (ctrl_x, ctrl_y)
    @location(5) color: vec4<f32>,
    @location(6) shape: u32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) half_size: vec2<f32>,
    @location(3) radius: f32,
    @location(4) border_width: f32,
}

@group(0) @binding(0)
var<uniform> u: Uniforms;

// binding(1): track colors — declared above as `var<storage> tc: array<vec4<f32>>`.

@group(0) @binding(2)
var<uniform> sel: SelectionUniform;

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    instance: DrawInstance,
) -> VertexOutput {
    var out: VertexOutput;

    let vel = instance.packed.z;
    let tag = instance.packed.w;

    // Convert tick→pixel for note instances when mode is 1 or 2
    var pixel_x = instance.xywh.x;
    var pixel_y = instance.xywh.y;
    var pixel_w = instance.xywh.z;
    var pixel_h = instance.xywh.w;

    if u.mode == 1u && vel > 0u {
        // PR notes: x=start_tick, y=key_number, w=end_tick, h=unused
        // tag stores track_index (u16 in lower bits)
        // x/w → pixel via ppu + scroll_x; y/h → pixel via key_height + scroll_y
        let start_tick = pixel_x;
        let key = pixel_y;
        let end_tick = pixel_w;
        let ppu = u.pixels_per_tick;
        let x_offset = u.keyboard_width - u.scroll_x;
        pixel_x = x_offset + start_tick * ppu;
        pixel_w = max(x_offset + end_tick * ppu - pixel_x, 2.0);
        let bottom = 128.0 * u.key_height - u.scroll_y;
        pixel_y = bottom - (key + 1.0) * u.key_height;
        pixel_h = u.key_height;
    }

    if u.mode == 2u && vel > 0u {
        // AR notes: x=start_tick, w=end_tick (y/h are pixel, unchanged)
        let start_tick = pixel_x;
        let end_tick = pixel_w;
        let ppu = u.pixels_per_tick;
        let x_offset = u.keyboard_width - u.scroll_x;
        pixel_x = x_offset + start_tick * ppu;
        pixel_w = max(x_offset + end_tick * ppu - pixel_x, 2.0);
    }

    // Snap to integer pixels (模式1和2) to prevent sub-pixel jitter.
    // Use floor(end) - floor(start) for width/height so adjacent notes
    // sharing a boundary have no gap.
    if u.scroll_mode != 0u {
        let raw_x = pixel_x;
        let raw_right = pixel_x + pixel_w;
        pixel_x = floor(raw_x + 0.5);
        let raw_y = pixel_y;
        let raw_bottom = pixel_y + pixel_h;
        pixel_y = floor(raw_y + 0.5);
        pixel_w = max(floor(raw_right + 0.5) - floor(raw_x + 0.5), 1.0);
        pixel_h = max(floor(raw_bottom + 0.5) - floor(raw_y + 0.5), 1.0);
    }

    var pos = array<vec2<f32>, 6>(
        vec2<f32>(pixel_x + pixel_w, pixel_y),
        vec2<f32>(pixel_x + pixel_w, pixel_y + pixel_h),
        vec2<f32>(pixel_x,           pixel_y),
        vec2<f32>(pixel_x + pixel_w, pixel_y + pixel_h),
        vec2<f32>(pixel_x,           pixel_y + pixel_h),
        vec2<f32>(pixel_x,           pixel_y),
    );

    var uv = array<vec2<f32>, 6>(
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 0.0),
    );

    let pixel_pos = pos[vertex_index];
    // Sub-pixel NDC offset (仅模式2): scroll_frac is the fractional part of
    // scroll_x.  CPU-side positions use floor(scroll_x) so they are stable.
    // The fractional offset here makes scrolling appear smooth at sub-pixel level.
    let ndc_offset = select(0.0, u.scroll_frac, u.scroll_mode == 2u);
    let ndc_x = ((pixel_pos.x - ndc_offset) / u.width) * 2.0 - 1.0;
    let ndc_y = 1.0 - (pixel_pos.y / u.height) * 2.0;

    out.clip_position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);

    // Color: mode=1 (PR notes) uses track_index from tag, mode=2/0 uses packed rgba
    var base_color: vec4<f32>;
    if u.mode == 1u && vel > 0u {
        // PR notes: get color from track_colors uniform via track_index (tag)
        let track_idx = tag;
        if track_idx < u.track_count {
            base_color = tc[track_idx];
        } else {
            // Fallback: use packed rgba if track_index out of range
            let rgba = instance.packed.x;
            base_color.r = f32((rgba >> 0u) & 0xFFu) / 255.0;
            base_color.g = f32((rgba >> 8u) & 0xFFu) / 255.0;
            base_color.b = f32((rgba >> 16u) & 0xFFu) / 255.0;
            base_color.a = f32((rgba >> 24u) & 0xFFu) / 255.0;
        }
    } else {
        // AR notes, decor, grid, keyboard: use packed rgba
        let rgba = instance.packed.x;
        base_color.r = f32((rgba >> 0u) & 0xFFu) / 255.0;
        base_color.g = f32((rgba >> 8u) & 0xFFu) / 255.0;
        base_color.b = f32((rgba >> 16u) & 0xFFu) / 255.0;
        base_color.a = f32((rgba >> 24u) & 0xFFu) / 255.0;
    }
    out.color = base_color;

    // Unpack props from packed u32 (2x f16), or compute for PR notes
    let props = instance.packed.y;
    var radius = unpack2x16float(props).x;
    var border_width = unpack2x16float(props).y;

    if u.mode == 1u && vel > 0u {
        // PR notes: no rounded corners, border based on key height
        radius = 0.0;
        border_width = select(0.0, max(0.1 * pixel_h, u.min_border_width), u.note_outline != 0u);
    }

    out.radius = radius;
    out.border_width = border_width;

    out.uv = uv[vertex_index];
    out.half_size = vec2<f32>(pixel_w, pixel_h) * 0.5;

    return out;
}

// ── Note pipeline vertex shader ───────────────────────────────────────────
// Handles PR notes (mode==1) and AR notes (mode==2).
// CPU only stores semantic data (start_tick, end_tick, key, track);
// all pixel positions are computed here from uniforms.
// Color is fetched from track_colors storage buffer.
@vertex
fn vs_main_note(
    @builtin(vertex_index) vertex_index: u32,
    instance: NoteInstance,
) -> VertexOutput {
    var out: VertexOutput;

    let start_tick = instance.data.x;
    let end_tick = instance.data.y;
    let packed = instance.data.z;
    let key = packed & 0xFFu;
    let track = (packed >> 8u) & 0xFFFFu;

    let ppu = u.pixels_per_tick;
    let x_offset = u.keyboard_width - u.scroll_x;

    var pixel_x = x_offset + f32(start_tick) * ppu;
    var pixel_w = max(x_offset + f32(end_tick) * ppu - pixel_x, 2.0);
    var pixel_y: f32;
    var pixel_h: f32;

    if u.mode == 1u {
        // PR: key_height based vertical layout
        let bottom = 128.0 * u.key_height - u.scroll_y;
        pixel_y = bottom - (f32(key) + 1.0) * u.key_height;
        pixel_h = u.key_height;
    } else {
        // AR (mode == 2u): lane_height based vertical layout
        // scroll_y is handled here (GPU-side), so AR notes cache is stable
        // across vertical scrolling — same optimization as PR notes.
        let lh = u.lane_height;
        let lh_per_key = lh / 128.0;
        pixel_y = -u.scroll_y + lh - (f32(key) + 1.0) * lh_per_key + f32(track) * lh;
        pixel_h = max(lh_per_key, 1.0);
    }

    // Snap to integer pixels to prevent sub-pixel jitter.
    if u.scroll_mode != 0u {
        let raw_x = pixel_x;
        let raw_right = pixel_x + pixel_w;
        pixel_x = floor(raw_x + 0.5);
        let raw_y = pixel_y;
        let raw_bottom = pixel_y + pixel_h;
        pixel_y = floor(raw_y + 0.5);
        pixel_w = max(floor(raw_right + 0.5) - floor(raw_x + 0.5), 1.0);
        pixel_h = max(floor(raw_bottom + 0.5) - floor(raw_y + 0.5), 1.0);
    }

    var pos = array<vec2<f32>, 6>(
        vec2<f32>(pixel_x + pixel_w, pixel_y),
        vec2<f32>(pixel_x + pixel_w, pixel_y + pixel_h),
        vec2<f32>(pixel_x,           pixel_y),
        vec2<f32>(pixel_x + pixel_w, pixel_y + pixel_h),
        vec2<f32>(pixel_x,           pixel_y + pixel_h),
        vec2<f32>(pixel_x,           pixel_y),
    );

    var uv = array<vec2<f32>, 6>(
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 0.0),
    );

    let pixel_pos = pos[vertex_index];
    let ndc_offset = select(0.0, u.scroll_frac, u.scroll_mode == 2u);
    let ndc_x = ((pixel_pos.x - ndc_offset) / u.width) * 2.0 - 1.0;
    let ndc_y = 1.0 - (pixel_pos.y / u.height) * 2.0;

    out.clip_position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);

    // Color: from track_colors storage buffer
    var base_color: vec4<f32>;
    if track < u.track_count {
        base_color = tc[track];
    } else {
        base_color = vec4<f32>(0.5, 0.5, 0.5, 1.0);
    }
    out.color = base_color;

    // No rounded corners; border based on vertical dimension (key/lane height).
    // PR (mode==1): border = 0.05 * pixel_h (narrowed from 0.1).
    // AR (mode==2): border = 0.1 * pixel_h (unchanged).
    out.radius = 0.0;
    let border_factor = select(0.1, 0.05, u.mode == 1u);
    out.border_width = select(0.0, max(border_factor * pixel_h, u.min_border_width), u.note_outline != 0u);

    out.uv = uv[vertex_index];
    out.half_size = vec2<f32>(pixel_w, pixel_h) * 0.5;

    return out;
}

// ── Velocity bar pipeline vertex shader ───────────────────────────────────
// Renders velocity bars in the automation panel.
// CPU stores semantic data (tick, length, track, velocity);
// GPU computes pixel positions from uniforms.
// Color is fetched from track_colors storage buffer.
// Unified border-based mode: fill + border (like notes), border width = min_border_width.
@vertex
fn vs_main_velocity(
    @builtin(vertex_index) vertex_index: u32,
    instance: VelocityBarInstance,
) -> VertexOutput {
    var out: VertexOutput;

    let tick = instance.data.x;
    let length = instance.data.y;
    let packed = instance.data.z;
    let track = packed & 0xFFFFu;
    let velocity = (packed >> 16u) & 0xFFu;

    let ppu = u.pixels_per_tick;
    let x_offset = u.keyboard_width - u.scroll_x;

    var pixel_x = x_offset + f32(tick) * ppu;
    var pixel_w = max(f32(length) * ppu, 2.0);

    // Y from velocity: value_to_y(vel) = h - (vel - scroll) * zoom / 127 * h
    // Bar top = value_to_y(velocity), bar bottom = value_to_y(0)
    let panel_h = u.height;
    let vel_f = f32(velocity);
    let y_top = panel_h - (vel_f - u.value_scroll) * u.value_zoom / 127.0 * panel_h;
    let y_bottom = panel_h - (0.0 - u.value_scroll) * u.value_zoom / 127.0 * panel_h;
    var pixel_y = y_top;
    var pixel_h = max(y_bottom - y_top, 1.0);

    // Snap to integer pixels to prevent sub-pixel jitter.
    if u.scroll_mode != 0u {
        let raw_x = pixel_x;
        let raw_right = pixel_x + pixel_w;
        pixel_x = floor(raw_x + 0.5);
        let raw_y = pixel_y;
        let raw_bottom = pixel_y + pixel_h;
        pixel_y = floor(raw_y + 0.5);
        pixel_w = max(floor(raw_right + 0.5) - floor(raw_x + 0.5), 1.0);
        pixel_h = max(floor(raw_bottom + 0.5) - floor(raw_y + 0.5), 1.0);
    }

    var pos = array<vec2<f32>, 6>(
        vec2<f32>(pixel_x + pixel_w, pixel_y),
        vec2<f32>(pixel_x + pixel_w, pixel_y + pixel_h),
        vec2<f32>(pixel_x,           pixel_y),
        vec2<f32>(pixel_x + pixel_w, pixel_y + pixel_h),
        vec2<f32>(pixel_x,           pixel_y + pixel_h),
        vec2<f32>(pixel_x,           pixel_y),
    );

    var uv = array<vec2<f32>, 6>(
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 0.0),
    );

    let pixel_pos = pos[vertex_index];
    let ndc_offset = select(0.0, u.scroll_frac, u.scroll_mode == 2u);
    let ndc_x = ((pixel_pos.x - ndc_offset) / u.width) * 2.0 - 1.0;
    let ndc_y = 1.0 - (pixel_pos.y / u.height) * 2.0;

    out.clip_position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);

    // Color: from track_colors storage buffer
    var base_color: vec4<f32>;
    if track < u.track_count {
        base_color = tc[track];
    } else {
        base_color = vec4<f32>(0.5, 0.5, 0.5, 1.0);
    }
    out.color = base_color;

    // Unified border width: fixed 1px, independent of zoom level
    // so users can scale freely without border thickness changing.
    out.radius = 0.0;
    out.border_width = 0.5;

    out.uv = uv[vertex_index];
    out.half_size = vec2<f32>(pixel_w, pixel_h) * 0.5;

    return out;
}

// SDF rounded box
fn sd_rounded_box(p: vec2<f32>, half_size: vec2<f32>, r: f32) -> f32 {
    let d = abs(p) - half_size + r;
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0) - r;
}

// Border + fill alpha compositing
fn composite_border_fill(fill_a: f32, border_a: f32, base_color: vec4<f32>) -> vec4<f32> {
    let total_a = fill_a + border_a;
    let border_color = base_color.rgb * BORDER_DARKEN_FACTOR;
    var rgb = border_color;
    if fill_a > 0.0 {
        rgb = (base_color.rgb * fill_a + border_color * border_a) / total_a;
    }
    return vec4(rgb, base_color.a * total_a);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let base_color = in.color;

    let p = (in.uv - 0.5) * in.half_size * 2.0;

    // Fast path: no rounded corners
    if in.radius < 0.5 {
        let d_outer = max(abs(p.x) - in.half_size.x, abs(p.y) - in.half_size.y);
        let outer_a = 1.0 - smoothstep(-0.5, 0.5, d_outer);

        let inner_half = max(in.half_size - vec2(in.border_width), vec2(0.0));
        var fill_a: f32 = 0.0;
        var border_a: f32 = outer_a;

        if inner_half.x > 0.0 && inner_half.y > 0.0 {
            let d_inner = max(abs(p.x) - inner_half.x, abs(p.y) - inner_half.y);
            let inner_a = 1.0 - smoothstep(-0.5, 0.5, d_inner);
            fill_a = inner_a;
            border_a = outer_a - inner_a;
        }

        return composite_border_fill(fill_a, border_a, base_color);
    }

    // Slow path: SDF rounded rectangle
    let d_outer = sd_rounded_box(p, in.half_size, in.radius);
    let outer_a = 1.0 - smoothstep(-0.5, 0.5, d_outer);

    let inner_half = max(in.half_size - vec2(in.border_width), vec2(0.0));
    let inner_r = max(in.radius - in.border_width, 0.0);

    var fill_a: f32 = 0.0;
    var border_a: f32 = outer_a;

    if inner_half.x > 0.0 && inner_half.y > 0.0 {
        let d_inner = sd_rounded_box(p, inner_half, inner_r);
        let inner_a = 1.0 - smoothstep(-0.5, 0.5, d_inner);
        fill_a = inner_a;
        border_a = outer_a - inner_a;
    }

    return composite_border_fill(fill_a, border_a, base_color);
}

// ── Curve / line pipeline ─────────────────────────────────────────────────
// Renders automation segments as parameterized curves with per-pixel SDF.
// CPU pushes one CurveInstance per segment; the fragment shader numerically
// solves the nearest point on the curve via 4 Newton iterations.

/// Distance from point `p` to line segment `a → b`.
fn sd_line(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-8), 0.0, 1.0);
    return length(pa - ba * h);
}

/// Distance from point `p` to the quadratic Bézier curve `(a, b, c)`.
///
///   B(u) = (1-u)²·a + 2(1-u)u·b + u²·c,   u ∈ [0, 1]
///   B'(u) = 2(1-u)·(b-a) + 2u·(c-b)
///   B''(u) = 2·(a - 2b + c)
///
/// Minimizes |B(u) - p|² via 6 Newton iterations on its derivative.
/// Seeded by projecting p onto the chord a→c (exact when b is on the chord).
fn sd_bezier(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, c: vec2<f32>) -> f32 {
    let chord = c - a;
    var u = clamp(dot(p - a, chord) / max(dot(chord, chord), 1e-8), 0.0, 1.0);

    for (var i = 0; i < 6; i = i + 1) {
        let u1 = 1.0 - u;
        let bu = u1 * u1 * a + 2.0 * u1 * u * b + u * u * c;
        let du = 2.0 * u1 * (b - a) + 2.0 * u * (c - b);
        let ddu = 2.0 * (a - 2.0 * b + c);
        let f = bu - p;
        // g(u)  = d/du |f|² = 2 · f · f'
        // g'(u) = 2 · (f'·f' + f·f'')
        let g = dot(f, du);
        let g_dt = 2.0 * (dot(du, du) + dot(f, ddu));
        u = u - g / max(g_dt, 1e-6);
        u = clamp(u, 0.0, 1.0);
    }

    let u1 = 1.0 - u;
    let bu = u1 * u1 * a + 2.0 * u1 * u * b + u * u * c;
    return length(bu - p);
}

@vertex
fn vs_main_curve(
    @builtin(vertex_index) vertex_index: u32,
    instance: CurveInstance,
) -> CurveOutput {
    let p1 = instance.endp.xy;  // P0
    let p2 = instance.endp.zw;  // P2
    // 控制点绝对位置（归一化 ctrl_x/ctrl_y → 屏幕坐标）
    let ctrl_pt = vec2<f32>(
        p1.x + (p2.x - p1.x) * instance.params.y,
        p1.y + (p2.y - p1.y) * instance.params.z,
    );

    // AABB 包含 P0, P2, 控制点（对 circle/square/hollow，P0==P2 且 ctrl_pt 落在 P0）
    let pad = instance.params.x + 1.0;
    let min_x = min(min(p1.x, p2.x), ctrl_pt.x) - pad;
    let max_x = max(max(p1.x, p2.x), ctrl_pt.x) + pad;
    let min_y = min(min(p1.y, p2.y), ctrl_pt.y) - pad;
    let max_y = max(max(p1.y, p2.y), ctrl_pt.y) + pad;

    var pos = array<vec2<f32>, 6>(
        vec2<f32>(max_x, min_y),
        vec2<f32>(max_x, max_y),
        vec2<f32>(min_x, min_y),
        vec2<f32>(max_x, max_y),
        vec2<f32>(min_x, max_y),
        vec2<f32>(min_x, min_y),
    );

    let p = pos[vertex_index];
    let ndc_offset = select(0.0, u.scroll_frac, u.scroll_mode == 2u);
    let ndc_x = ((p.x - ndc_offset) / u.width) * 2.0 - 1.0;
    let ndc_y = 1.0 - (p.y / u.height) * 2.0;

    var out: CurveOutput;
    out.clip_position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.local = p;
    out.p1 = p1;
    out.p2 = p2;
    out.thickness = instance.params.x;
    out.ctrl = instance.params.yz;
    out.shape = instance.shape;

    let rgba = instance.rgba;
    out.color = vec4<f32>(
        f32((rgba >> 0u)  & 0xFFu) / 255.0,
        f32((rgba >> 8u)  & 0xFFu) / 255.0,
        f32((rgba >> 16u) & 0xFFu) / 255.0,
        f32((rgba >> 24u) & 0xFFu) / 255.0,
    );
    return out;
}

@fragment
fn fs_main_curve(in: CurveOutput) -> @location(0) vec4<f32> {
    let p = in.local;

    var d: f32;
    if (in.shape == 1u) {
        // Filled circle: distance from center minus radius.
        d = length(p - in.p1) - in.thickness;
    } else if (in.shape == 2u) {
        // Filled square (axis-aligned box SDF).
        let q = abs(p - in.p1) - in.thickness;
        d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0);
    } else if (in.shape == 3u) {
        // Hollow circle (ring): outer radius = thickness, ring width = 1.5px.
        let r = length(p - in.p1);
        let ring_width = 1.5;
        let inner = in.thickness - ring_width;
        d = max(r - in.thickness, inner - r);
    } else {
        // shape == 0: bezier curve segment.
        let is_linear = abs(in.ctrl.x - 0.5) < 1e-4 && abs(in.ctrl.y - 0.5) < 1e-4;
        if (is_linear) {
            // Straight line fast path.
            d = sd_line(p, in.p1, in.p2);
        } else {
            // Quadratic Bézier. Reconstruct control point in screen space.
            let ctrl_pt = vec2<f32>(
                in.p1.x + (in.p2.x - in.p1.x) * in.ctrl.x,
                in.p1.y + (in.p2.y - in.p1.y) * in.ctrl.y,
            );
            d = sd_bezier(p, in.p1, ctrl_pt, in.p2);
        }
    }

    // 1px anti-aliased stroke around `thickness`.
    let aa = 1.0;
    let alpha = 1.0 - smoothstep(in.thickness - aa, in.thickness + aa, d);
    return vec4<f32>(in.color.rgb, in.color.a * alpha);
}