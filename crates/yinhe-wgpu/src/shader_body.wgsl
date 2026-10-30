// ═══ 主体：几何 / 片元（两套变体共用）═══════════════════════════════════

struct DrawInstance {
    @location(0) xywh: vec4<f32>,
    @location(1) packed: vec4<u32>,  // x=rgba(UNORM8), y=props(2xf16), z=velocity, w=tag
}

struct NoteInstance {
    @location(0) data: vec3<u32>,  // x=start_tick, y=end_tick, z=packed(key|track|vel)
}

struct VelocityBarInstance {
    @location(0) data: vec4<u32>,  // x=tick, y=length, z=packed(track|velocity), w=reserved
}

/// Curve/line/anchor instance (28 bytes).
/// See `CurveInstance` in vertex.rs for the CPU-side layout.
struct CurveInstance {
    @location(0) endp: vec4<f32>,      // (x1, y1, x2, y2) — P0, P3 端点
    @location(1) thickness: f32,       // line thickness / anchor radius
    @location(2) rgba: u32,             // UNORM8: R|G<<8|B<<16|A<<24
    @location(3) shape: u32,            // 0 = line, 1 = filled circle, 2 = filled square, 3 = hollow circle
}

struct CurveOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local: vec2<f32>,     // pixel-space position
    @location(1) p0: vec2<f32>,
    @location(2) p3: vec2<f32>,
    @location(3) thickness: f32,
    @location(4) color: vec4<f32>,
    @location(5) @interpolate(flat) shape: u32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) half_size: vec2<f32>,
    @location(3) radius: f32,
    @location(4) border_width: f32,
    /// 填充色（描边仍用 color）：PR/AR 音符按选中/力度计算，其余等于 color.rgb。
    @location(5) fill_color: vec3<f32>,
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
        // 视口左边界 = keyboard_width（见 note_geometry 同注释）。
        pixel_x = max(pixel_x, u.keyboard_width);
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
        // 视口左边界 = keyboard_width（见 note_geometry 同注释）。
        pixel_x = max(pixel_x, u.keyboard_width);
        // AR 保留 1px 可视下限：短音符不消失，但不会像 2px 那样明显拉长。
        pixel_w = max(x_offset + end_tick * ppu - pixel_x, 1.0);
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
    let ndc_x = (pixel_pos.x / u.width) * 2.0 - 1.0;
    let ndc_y = 1.0 - (pixel_pos.y / u.height) * 2.0;

    out.clip_position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);

    // 颜色：PR 音符按轨道号取调色板（tag），其余模式用实例自带 packed rgba。
    var base_color: vec4<f32>;
    if u.mode == 1u && vel > 0u {
        // PR notes: 调色板由变体前缀提供（storage buffer 或数据纹理）；
        // 轨道号越界时退回实例携带的 packed rgba。
        base_color = track_color_or(tag, unpack_rgba(instance.packed.x));
    } else {
        // AR notes, decor, grid, keyboard: use packed rgba
        base_color = unpack_rgba(instance.packed.x);
    }
    base_color = to_target_space(base_color);
    out.color = base_color;
    out.fill_color = base_color.rgb; // 非音符管线不做选中/力度着色

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

// 共享的几何计算：tick/key/track → 像素矩形 → NDC。
// 方向分支见 u.orientation：
// - 0 横向（默认）：时间轴沿 X（起点 keyboard_width），音高沿 Y。
//   视口左边界 = keyboard_width（键盘列/轨道面板列由 egui 层绘制）。
// - 1 纵向瀑布流：时间轴沿 Y（tick 0 在顶部，向下增长），音高沿 X（key 0 在左）。
fn note_geometry(
    vertex_index: u32,
    start_tick: u32,
    end_tick: u32,
    packed: u32,
) -> VertexOutput {
    var out: VertexOutput;

    // bit7 = ghost 标记（key 只有 0..127）；动画偏移只作用于 ghost 实例。
    let is_ghost = (packed & 0x80u) != 0u;
    let key = packed & 0x7Fu;
    let track = (packed >> 8u) & 0xFFFFu;
    // vel 字段拆位：bit7 = 选中（填充加深），bit0..7 = MIDI 力度。
    let vel_raw = (packed >> 24u) & 0xFFu;
    var selected = (vel_raw & 0x80u) != 0u;
    let vel = vel_raw & 0x7Fu;
    // GPU cull 全曲层的实例不随选区重传（bit31 恒 0）→ 用矩形选区 uniform
    // 实时补位；B 路径已带 CPU 精确位（含成员态）则取或。
    if u.mode == 1u && !selected {
        let gate = end_tick - min(start_tick, end_tick);
        selected = in_selection_rects(start_tick, key, track)
            && !in_exclude_table(key | (track << 8u), start_tick)
            && selection_filter_passes(key, track, vel, gate);
    }

    let ppu = u.pixels_per_tick;

    var pixel_x: f32;
    var right: f32;
    var pixel_y: f32;
    var pixel_h: f32;
    var pixel_w: f32;

    if u.orientation == 1u {
        // 纵向瀑布流：音高沿 X（key * key_height - scroll_x，从 0 开始），
        // 时间沿 Y 反转（tick 0 在底部、越大越靠上，键盘在底部 = 当前时刻）。
        pixel_x = f32(key) * u.key_height - u.scroll_x;
        right = pixel_x + u.key_height;
        pixel_w = u.key_height;
        let y_a = u.height - (f32(start_tick) * ppu - u.scroll_y);
        let y_b = u.height - (f32(end_tick) * ppu - u.scroll_y);
        pixel_y = min(y_a, y_b);
        pixel_h = max(max(y_a, y_b) - pixel_y, 2.0);
    } else {
        let x_offset = u.keyboard_width - u.scroll_x;

        // 右边界直接算（与相邻音符的左边界完全相同的表达式），杜绝 1px 缝隙。
        pixel_x = x_offset + f32(start_tick) * ppu;
        right = x_offset + f32(end_tick) * ppu;
        // 视口左边界 = keyboard_width（键盘列/轨道面板列由 egui 层绘制，
        // 音符左端 clamp 到其右缘，长音符从左侧进入时显示为被左列"切"掉）。
        pixel_x = max(pixel_x, u.keyboard_width);
        // AR 保留 1px 可视下限；PR 仍保留 2px，避免短音符消失。
        pixel_w = max(right - pixel_x, 1.0);
        if u.mode == 1u {
            pixel_w = max(pixel_w, 2.0);
        }

        if u.mode == 1u {
            // PR: key_height based vertical layout
            let bottom = 128.0 * u.key_height - u.scroll_y;
            pixel_y = bottom - (f32(key) + 1.0) * u.key_height;
            pixel_h = u.key_height;
        } else {
            // AR (mode == 2u): lane_height based vertical layout
            // 行高 per-row（通道 AR 的窄合并行）：`row_height(track)`；
            // 轨道 AR 由 CPU 上传全 lane_height，行为不变。
            let lh = row_height(track);
            let lh_per_key = lh / 128.0;
            let track_y = track_offset(track);
            pixel_y = -u.scroll_y + track_y + lh - (f32(key) + 1.0) * lh_per_key;
            pixel_h = max(lh_per_key, 1.0);
        }
    }

    // ghost 动画偏移必须在构造顶点之前叠加。
    if is_ghost {
        pixel_x += u.ghost_dx;
        pixel_y += u.ghost_dy;
    }
    right = pixel_x + pixel_w;

    // 4 顶点 + 共享 index buffer（[0,1,2, 1,3,2]）：0=TL, 1=TR, 2=BL, 3=BR。
    var pos = array<vec2<f32>, 4>(
        vec2<f32>(pixel_x, pixel_y),
        vec2<f32>(right,   pixel_y),
        vec2<f32>(pixel_x, pixel_y + pixel_h),
        vec2<f32>(right,   pixel_y + pixel_h),
    );

    var uv = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );

    let pixel_pos = pos[vertex_index];
    var ndc_x: f32;
    var ndc_y: f32;
    if u.orientation == 1u {
        ndc_x = (pixel_pos.x / u.width) * 2.0 - 1.0;
        ndc_y = 1.0 - (pixel_pos.y / u.height) * 2.0;
    } else {
        ndc_x = (pixel_pos.x / u.width) * 2.0 - 1.0;
        ndc_y = 1.0 - (pixel_pos.y / u.height) * 2.0;
    }
    out.clip_position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);

    // 颜色：轨道调色板（变体前缀提供），无该轨道时用中性灰。
    var base_color = track_color_or(track, vec4<f32>(0.5, 0.5, 0.5, 1.0));
    base_color = to_target_space(base_color);
    out.color = base_color;

    // 填充色：仅 PR（mode==1）按选中/力度变化；AR 等其它模式保持原色。
    // 力度 → 向背景方向淡化（暗色偏黑、亮色偏白），保留色相、音符仍不透明；
    // 选中 → 向与背景对比最强的一极（暗色偏白、亮色偏黑），选中态永远清晰。
    let fade_target = select(vec3<f32>(1.0), vec3<f32>(0.0), u.dark_theme != 0u);
    let selected_target = select(vec3<f32>(0.0), vec3<f32>(1.0), u.dark_theme != 0u);
    if u.mode == 1u && selected {
        out.fill_color = mix(base_color.rgb, selected_target, SELECTED_CONTRAST);
    } else if u.mode == 1u {
        let fade = (1.0 - f32(vel) / 127.0) * MAX_FILL_FADE;
        out.fill_color = mix(base_color.rgb, fade_target, fade);
    } else {
        out.fill_color = base_color.rgb;
    }

    // No rounded corners; border based on vertical dimension (key/lane height).
    // PR (mode==1): border = 0.05 * pixel_h (narrowed from 0.1).
    // AR (mode==2): border = 0.1 * pixel_h (unchanged).
    // 纵向瀑布流：音符高度是时间长度，border 改基于宽度（key_height 方向），
    // 避免长音符的竖直边缘过宽。
    out.radius = 0.0;
    let border_dim = select(pixel_h, pixel_w, u.orientation == 1u);
    let border_factor = select(0.1, 0.05, u.mode == 1u);
    out.border_width = select(0.0, max(border_factor * border_dim, u.min_border_width), u.note_outline != 0u);

    out.uv = uv[vertex_index];
    out.half_size = vec2<f32>(pixel_w, pixel_h) * 0.5;

    return out;
}
// CPU-built note layers (ghost notes / legacy path): full 12B instance data
// comes straight from the vertex buffer — no index indirection.
@vertex
fn vs_main_note_direct(
    @builtin(vertex_index) vertex_index: u32,
    instance: NoteInstance,
) -> VertexOutput {
    return note_geometry(vertex_index, instance.data.x, instance.data.y, instance.data.z);
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

    // 右边界直接算（tick+length = 音符右端，与相邻 bar 的左边界同表达式），
    // 杜绝 1px 缝隙（链式 pixel_x + pixel_w 的取整/舍入误差见 note_geometry）。
    var pixel_x = x_offset + f32(tick) * ppu;
    var right = x_offset + f32(tick + length) * ppu;
    // 视口左边界 = keyboard_width（AM 面板的 combo 列宽 = keyboard_width，
    // 力度条左端 clamp 到网格区左缘，与右侧在面板右缘被裁对称）。
    pixel_x = max(pixel_x, u.keyboard_width);
    var pixel_w = max(right - pixel_x, 2.0);

    // Y from velocity: 126 级映射（vel=1 已由构建过滤，不显示）。
    // vel 2..=127 → 高度 1..=126 单位：y = (vel-1)/126 * panel_h。
    let panel_h = u.height;
    let vel_f = f32(max(velocity, 1u) - 1u);
    let y_top = panel_h - (vel_f - u.value_scroll) * u.value_zoom / 126.0 * panel_h;
    let y_bottom = panel_h - (0.0 - u.value_scroll) * u.value_zoom / 126.0 * panel_h;
    var pixel_y = y_top;
    var pixel_h = max(y_bottom - y_top, 1.0);

    right = pixel_x + pixel_w;

    // 4 顶点 + 共享 index buffer（与 note 相同：[0,1,2, 1,3,2]）。
    var pos = array<vec2<f32>, 4>(
        vec2<f32>(pixel_x, pixel_y),
        vec2<f32>(right,   pixel_y),
        vec2<f32>(pixel_x, pixel_y + pixel_h),
        vec2<f32>(right,   pixel_y + pixel_h),
    );

    var uv = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );

    let pixel_pos = pos[vertex_index];
    let ndc_x = (pixel_pos.x / u.width) * 2.0 - 1.0;
    let ndc_y = 1.0 - (pixel_pos.y / u.height) * 2.0;

    out.clip_position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);

    // 颜色：轨道调色板（变体前缀提供），无该轨道时用中性灰。
    var base_color = track_color_or(track, vec4<f32>(0.5, 0.5, 0.5, 1.0));
    base_color = to_target_space(base_color);
    out.color = base_color;
    out.fill_color = base_color.rgb; // 力度条不做选中/力度着色

    // Unified border width: fixed 1px, independent of zoom level
    // so users can scale freely without border thickness changing.
    out.radius = 0.0;
    out.border_width = 1.0;

    out.uv = uv[vertex_index];
    out.half_size = vec2<f32>(pixel_w, pixel_h) * 0.5;

    return out;
}

// SDF rounded box
fn sd_rounded_box(p: vec2<f32>, half_size: vec2<f32>, r: f32) -> f32 {
    let d = abs(p) - half_size + r;
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0) - r;
}

// Border + fill alpha compositing — 音符全不透明（轨道色 alpha=1 时）
// 相邻音符共享边界 right==next.left 时，若外缘用 smoothstep 0.5 会与透明 clear
// 混合出 0.25 漏底细线；此处用硬边 select 保证内侧 alpha=1。
fn composite_border_fill(
    fill_a: f32,
    border_a: f32,
    base_alpha: f32,
    border_color: vec3<f32>,
    fill_color: vec3<f32>,
) -> vec4<f32> {
    let total_a = fill_a + border_a;
    if total_a <= 0.0 {
        discard;
    }
    var rgb = border_color;
    if fill_a > 0.0 {
        rgb = (fill_color * fill_a + border_color * border_a) / total_a;
    }
    return vec4(rgb, base_alpha * total_a);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let base_color = in.color;
    // 边框色：有描边时向黑加深；无描边（border_width==0，如 AR）时直接用填充色，
    // 否则抗锯齿边缘仍会混入暗化色（BORDER_DARKEN_FACTOR=0.4），窄音符整体偏暗。
    let border_color = select(
        in.fill_color,
        base_color.rgb * BORDER_DARKEN_FACTOR,
        in.border_width > 0.0,
    );

    let p = (in.uv - 0.5) * in.half_size * 2.0;

    // Fast path: no rounded corners — 外缘保守 0.5px 硬边防漏底+消 epsilon 闪，内缘平滑保留 <1px 边框平均色
    if in.radius < 0.5 {
        let d_outer = max(abs(p.x) - in.half_size.x, abs(p.y) - in.half_size.y);
        let outer_a = select(0.0, 1.0, d_outer <= 0.5);

        let inner_half = max(in.half_size - vec2(in.border_width), vec2(0.0));
        var fill_a: f32 = 0.0;
        var border_a: f32 = outer_a;

        if inner_half.x > 0.0 && inner_half.y > 0.0 {
            let d_inner = max(abs(p.x) - inner_half.x, abs(p.y) - inner_half.y);
            let inner_a = 1.0 - smoothstep(-0.5, 0.5, d_inner);
            fill_a = inner_a;
            border_a = outer_a - inner_a;
        }

        return composite_border_fill(fill_a, border_a, base_color.a, border_color, in.fill_color);
    }

    // Slow path: SDF rounded rectangle — 同上
    let d_outer = sd_rounded_box(p, in.half_size, in.radius);
    let outer_a = select(0.0, 1.0, d_outer <= 0.5);

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

    return composite_border_fill(fill_a, border_a, base_color.a, border_color, in.fill_color);
}

// Velocity bars: 仅描边、不填充、不加深（描边 = 轨道色本身）。
// 与音符共用 `VertexOutput`，但力度条刻意去掉内部填充与边框加深，
// 让重叠的力度条更轻、更易区分。
//
// 内外缘都用硬边：内部是 discard（透明），若内缘走 smoothstep 会写出
// 半透明像素，与网格背景混合后看起来像“第二条更浅的描边”。硬边保证
// 只有一圈实心描边。
@fragment
fn fs_main_velocity(in: VertexOutput) -> @location(0) vec4<f32> {
    let base_color = in.color;
    let p = (in.uv - 0.5) * in.half_size * 2.0;

    let d_outer = max(abs(p.x) - in.half_size.x, abs(p.y) - in.half_size.y);
    let outer_a = select(0.0, 1.0, d_outer <= 0.5);

    // 内部挖空：只保留 `border_width` 宽的描边；过细的条自然退化为实线。
    let inner_half = max(in.half_size - vec2(in.border_width), vec2(0.0));
    var border_a = outer_a;
    if inner_half.x > 0.0 && inner_half.y > 0.0 {
        let d_inner = max(abs(p.x) - inner_half.x, abs(p.y) - inner_half.y);
        let inner_a = select(0.0, 1.0, d_inner <= -0.5);
        border_a = outer_a - inner_a;
    }

    if border_a <= 0.0 {
        discard;
    }
    return vec4(base_color.rgb, base_color.a * border_a);
}

// ── Curve / line pipeline ─────────────────────────────────────────────────
// Renders automation segments as per-pixel SDF lines / anchors.
// CPU pushes one CurveInstance per segment; the fragment shader computes
// the per-pixel distance to the line segment via sd_line.
// Bézier curves are flattened into a polyline of line instances on the CPU
// (one per screen pixel column), so no GPU-side curve evaluation is needed.

/// Distance from point `p` to line segment `a → b`.
fn sd_line(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-8), 0.0, 1.0);
    return length(pa - ba * h);
}

@vertex
fn vs_main_curve(
    @builtin(vertex_index) vertex_index: u32,
    instance: CurveInstance,
) -> CurveOutput {
    let is_ghost = (instance.shape & 0x100u) != 0u;
    let ghost_offset = select(vec2<f32>(0.0), vec2<f32>(u.ghost_dx, u.ghost_dy), is_ghost);
    let p0 = instance.endp.xy + ghost_offset;  // P0
    let p3 = instance.endp.zw + ghost_offset;  // P3

    // AABB 包含 P0, P3（对 circle/square/hollow，全部重合）
    let pad = instance.thickness + 1.0;
    let min_x = min(p0.x, p3.x) - pad;
    let max_x = max(p0.x, p3.x) + pad;
    let min_y = min(p0.y, p3.y) - pad;
    let max_y = max(p0.y, p3.y) + pad;

    var pos = array<vec2<f32>, 6>(
        vec2<f32>(max_x, min_y),
        vec2<f32>(max_x, max_y),
        vec2<f32>(min_x, min_y),
        vec2<f32>(max_x, max_y),
        vec2<f32>(min_x, max_y),
        vec2<f32>(min_x, min_y),
    );

    let p = pos[vertex_index];
    let ndc_x = (p.x / u.width) * 2.0 - 1.0;
    let ndc_y = 1.0 - (p.y / u.height) * 2.0;

    var out: CurveOutput;
    out.clip_position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.local = p;
    out.p0 = p0;
    out.p3 = p3;
    out.thickness = instance.thickness;
    out.shape = instance.shape & 0xFFu;

    out.color = to_target_space(unpack_rgba(instance.rgba));
    return out;
}

@fragment
fn fs_main_curve(in: CurveOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color.rgb, in.color.a * curve_coverage(in));
}

// 折叠预览叠加层：预乘 alpha 输出，配合 Max 混合实现「整层 0.35 不透明度、
// 重叠不加深」。同一轨所有折叠 lane 共享 track 颜色，Max 即取并集覆盖。
@fragment
fn fs_main_curve_premul(in: CurveOutput) -> @location(0) vec4<f32> {
    let a = in.color.a * curve_coverage(in);
    return vec4<f32>(in.color.rgb * a, a);
}

/// 曲线/线段/锚点的覆盖度 alpha（0..1，含 1px AA）。
fn curve_coverage(in: CurveOutput) -> f32 {
    let p = in.local;

    var d: f32;
    if (in.shape == 1u) {
        // Filled circle: distance from center minus radius.
        d = length(p - in.p0) - in.thickness;
    } else if (in.shape == 2u) {
        // Filled square (axis-aligned box SDF).
        let q = abs(p - in.p0) - in.thickness;
        d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0);
    } else if (in.shape == 3u) {
        // Hollow circle (ring): outer radius = thickness, ring width = 1.5px.
        let r = length(p - in.p0);
        let ring_width = 1.5;
        let inner = in.thickness - ring_width;
        d = max(r - in.thickness, inner - r);
    } else {
        // shape == 0: line segment (Bézier curves are flattened on CPU).
        d = sd_line(p, in.p0, in.p3);
    }

    // 约 1 个物理像素的抗锯齿。`in.local` / uniforms 使用逻辑坐标，
    // fwidth(d) 会随 DPR 自动换算；固定 1.0 在高 DPI 上会把小圆环糊得过宽。
    let fw = max(fwidth(d), 0.0001);
    let aa = fw * 0.5;
    // shape 0（曲线/线段）: d 是到曲线的 unsigned 距离（≥0）。
    // shape 1/2/3（圆/方/环）: d 是 signed distance（d=0 在几何边界）。
    if (in.shape == 0u) {
        return 1.0 - smoothstep(in.thickness - aa, in.thickness + aa, d);
    }
    return 1.0 - smoothstep(-aa, aa, d);
}