// ── Rendering constants ───────────────────────────────────────────────────
const BORDER_DARKEN_FACTOR: f32 = 0.4;
const SELECTED_DARKEN_FACTOR: f32 = 0.15;

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
}

struct NoteInstance {
    @location(0) xywh: vec4<f32>,
    @location(1) packed: vec4<u32>,  // x=rgba(UNORM8), y=props(2xf16), z=velocity, w=tag
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) half_size: vec2<f32>,
    @location(3) radius: f32,
    @location(4) border_width: f32,
    @location(5) sel_flag: u32, // 1 = selected note (velocity>0 && tag==1)
}

@group(0) @binding(0)
var<uniform> u: Uniforms;

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    instance: NoteInstance,
) -> VertexOutput {
    var out: VertexOutput;

    let vel = instance.packed.z;
    let tag = instance.packed.w;

    // Convert tick→pixel for note instances when mode is 1 or 2
    var pixel_x = instance.xywh.x;
    var pixel_y = instance.xywh.y;
    var pixel_w = instance.xywh.z;
    var pixel_h = instance.xywh.w;

    if (u.mode == 1u || u.mode == 2u) && vel > 0u {
        // x = start_tick, w = end_tick
        let start_tick = pixel_x;
        let end_tick = pixel_w;
        let ppu = u.pixels_per_tick;
        let x_offset = u.keyboard_width - u.scroll_x;
        let raw_x = x_offset + start_tick * ppu;
        let raw_end = x_offset + end_tick * ppu;
        pixel_x = raw_x;
        pixel_w = max(raw_end - raw_x, 2.0);
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

    // Unpack RGBA from packed u32 (4x UNORM8)
    let rgba = instance.packed.x;
    out.color.r = f32((rgba >> 0u) & 0xFFu) / 255.0;
    out.color.g = f32((rgba >> 8u) & 0xFFu) / 255.0;
    out.color.b = f32((rgba >> 16u) & 0xFFu) / 255.0;
    out.color.a = f32((rgba >> 24u) & 0xFFu) / 255.0;

    // Unpack props from packed u32 (2x f16), or compute for PR notes
    let props = instance.packed.y;
    var radius = unpack2x16float(props).x;
    var border_width = unpack2x16float(props).y;

    if u.mode == 1u && vel > 0u {
        // PR notes: compute rounding/border from pixel dimensions
        let min_dim = min(pixel_w, pixel_h);
        radius = 0.15 * min_dim;
        border_width = max(0.1 * min_dim, u.min_border_width);
    }

    out.radius = radius;
    out.border_width = border_width;

    out.uv = uv[vertex_index];
    out.half_size = vec2<f32>(pixel_w, pixel_h) * 0.5;
    // sel_flag = velocity>0 && tag==1 (selected note)
    out.sel_flag = select(0u, 1u, vel > 0u && tag == 1u);
    return out;
}

// SDF rounded box
fn sd_rounded_box(p: vec2<f32>, half_size: vec2<f32>, r: f32) -> f32 {
    let d = abs(p) - half_size + r;
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0) - r;
}

// Border + fill alpha compositing
fn composite_border_fill(fill_a: f32, border_a: f32, base_color: vec4<f32>, sel_flag: u32) -> vec4<f32> {
    let total_a = fill_a + border_a;
    let fill_color = select(base_color.rgb, base_color.rgb * SELECTED_DARKEN_FACTOR, sel_flag != 0u);
    let border_color = select(base_color.rgb * BORDER_DARKEN_FACTOR, base_color.rgb, sel_flag != 0u);
    var rgb = border_color;
    if fill_a > 0.0 {
        rgb = (fill_color * fill_a + border_color * border_a) / total_a;
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

        return composite_border_fill(fill_a, border_a, base_color, in.sel_flag);
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

    return composite_border_fill(fill_a, border_a, base_color, in.sel_flag);
}
