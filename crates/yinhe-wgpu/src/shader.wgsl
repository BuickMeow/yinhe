// ── Rendering constants ───────────────────────────────────────────────────
const BORDER_DARKEN_FACTOR: f32 = 0.4;
const SELECTED_DARKEN_FACTOR: f32 = 0.15;
const MAX_TRACKS: u32 = 65536u;
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
    note_selection_highlight: u32, // 0=off (no color change), 1=on
    lane_height: f32, // AR: per-track lane height (PR unused)
    note_alpha: f32, // note alpha override (PR=1.0, AR=0.85)
}

struct TrackColorsUniform {
    colors: array<vec4<f32>, MAX_TRACKS>, // RGBA for each track
}

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

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) half_size: vec2<f32>,
    @location(3) radius: f32,
    @location(4) border_width: f32,
    @location(5) sel_flag: u32, // 1 = selected note
}

@group(0) @binding(0)
var<uniform> u: Uniforms;

@group(0) @binding(1)
var<storage> tc: TrackColorsUniform;

@group(0) @binding(2)
var<uniform> sel: SelectionUniform;

// Check if a note (track, start_tick, key) is within any selection rect.
fn is_selected(track: u32, start_tick: u32, key: u32) -> bool {
    let count = u.sel_rect_count;
    for (var i = 0u; i < count; i++) {
        let r0 = sel.rects[i * 2u];
        let r1 = sel.rects[i * 2u + 1u];
        let tick_start = r0.x;
        let tick_end = r0.y;
        let key_lo = r0.z;
        let key_hi = r0.w;
        let track_lo = r1.x;
        let track_hi = r1.y;
        if (track >= track_lo && track <= track_hi
            && key >= key_lo && key <= key_hi
            && start_tick >= tick_start && start_tick < tick_end) {
            return true;
        }
    }
    return false;
}

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
            base_color = tc.colors[track_idx];
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
        border_width = max(0.1 * pixel_h, u.min_border_width);
    }

    out.radius = radius;
    out.border_width = border_width;

    out.uv = uv[vertex_index];
    out.half_size = vec2<f32>(pixel_w, pixel_h) * 0.5;

    // Selection: mode=1 (PR notes) checks selection uniform via track_index
    // mode=2 (AR notes) or decor/grid/keyboard: sel_flag = 0
    // Color change is gated by u.note_selection_highlight — when off, sel_flag
    // stays 0 so selected notes render with the same color as unselected ones.
    if u.mode == 1u && vel > 0u && u.note_selection_highlight != 0u {
        let track_idx = tag;
        let start_tick = instance.xywh.x;  // stored as tick
        let key = instance.xywh.y;          // stored as key number
        out.sel_flag = select(0u, 1u, is_selected(track_idx, u32(start_tick), u32(key)));
    } else {
        out.sel_flag = 0u;
    }

    return out;
}

// ── Note pipeline vertex shader ───────────────────────────────────────────
// Handles PR notes (mode==1) and AR notes (mode==2).
// CPU only stores semantic data (start_tick, end_tick, key, track);
// all pixel positions are computed here from uniforms.
// Color is fetched from track_colors uniform; alpha is overridden by note_alpha.
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

    // Color: from track_colors uniform, alpha overridden by note_alpha
    var base_color: vec4<f32>;
    if track < u.track_count {
        base_color = tc.colors[track];
    } else {
        base_color = vec4<f32>(0.5, 0.5, 0.5, 1.0);
    }
    base_color.a = u.note_alpha;
    out.color = base_color;

    // No rounded corners; border based on vertical dimension (key/lane height)
    out.radius = 0.0;
    out.border_width = max(0.1 * pixel_h, u.min_border_width);

    out.uv = uv[vertex_index];
    out.half_size = vec2<f32>(pixel_w, pixel_h) * 0.5;

    // Selection: only PR mode (mode==1) checks selection uniform
    if u.mode == 1u && u.note_selection_highlight != 0u {
        out.sel_flag = select(0u, 1u, is_selected(track, start_tick, key));
    } else {
        out.sel_flag = 0u;
    }

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