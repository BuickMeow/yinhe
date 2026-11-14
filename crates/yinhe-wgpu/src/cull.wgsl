// GPU compute cull for NoteInstance (12 bytes each).
//
// Per-key architecture: each MIDI key (0..127) has its own `all_notes` and
// `visible_indices` storage buffer. The host dispatches this shader once per
// key, binding that key's buffers. This removes any global visible-note
// cap — the total visible capacity equals the total note count.
//
// Output: fixed-slot sparse, **two-way partitioned by track selection**.
// Chunk c writes its visible indices to the fixed slots [c*256, c*256+256) of
// `visible_indices`, unselected tracks first then selected tracks (each in
// input order via a workgroup prefix sum). Thread 0 writes two draw args:
//   A (unselected) at draw_args[wg]           → first_instance = c*256
//   B (selected)   at draw_args[wg + chunk_total] → first_instance = c*256 + n0
// where n0 = unselected visible count in the chunk. The host draws list A for
// all chunks first, then list B — so every selected-track note lands on top of
// every unselected note (global z priority), not just within a chunk.
//
// Within each group, the workgroup prefix sum (Hillis-Steele scan) keeps the
// input order (= all_notes order = tick order), so overlapping notes are stable
// across frames — no flickering, no atomics, no scheduling dependence.
//
// The vertex stage reads back the full NoteInstance from `all_instances`
// (bound via the same per-key bind group, @group(1) in shader.wgsl) using
// the 4-byte index — visible slots are 1/3 the size of the data itself.

struct Uniforms {
    width: f32,
    height: f32,
    scroll_x: f32,
    scroll_y: f32,
    pixels_per_tick: f32,
    key_height: f32,
    keyboard_width: f32,
    mode: u32,
    min_border_width: f32,
    track_count: u32,
    sel_rect_count: u32,
    note_outline: u32,
    lane_height: f32,
    value_zoom: f32,
    value_scroll: f32,
    orientation: u32,
    // 以下与 shader.wgsl / Rust `Uniforms` 同序（256 键 + 调式折叠）。
    filter_key: u32,
    filter_track: u32,
    filter_velocity: u32,
    filter_gate_lo: u32,
    filter_gate_hi: u32,
    filter_flags: u32,
    exclude_mask: u32,
    dark_theme: u32,
    ghost_dx: f32,
    ghost_dy: f32,
    key_count: u32,
    key_offset: u32,
    fold_enabled: u32,
    fold_root: u32,
    fold_mask: u32,
    fold_thin: f32,
    fold_pad0: f32,
    fold_pad1: f32,
    fold_pad2: f32,
    fold_pad3: f32,
};

fn pr_in_scale(pc: u32) -> bool {
    return (u.fold_mask & (1u << pc)) != 0u;
}

fn pr_units_before(key: u32) -> f32 {
    if (u.fold_enabled == 0u) {
        return f32(key);
    }
    let pc = key % 12u;
    let oct = key / 12u;
    let n_in = countOneBits(u.fold_mask);
    let n_out = 12u - n_in;
    let per_oct = f32(n_in) + f32(n_out) * u.fold_thin;
    let in_before = f32(countOneBits(u.fold_mask & ((1u << pc) - 1u)));
    let out_before = f32(pc) - in_before;
    return f32(oct) * per_oct + in_before + out_before * u.fold_thin;
}

fn pr_row(key: u32) -> vec2<f32> {
    let lo = pr_units_before(key);
    if (u.fold_enabled == 0u) {
        return vec2<f32>(lo, lo + 1.0);
    }
    let h = select(u.fold_thin, 1.0, pr_in_scale(key % 12u));
    return vec2<f32>(lo, lo + h);
}

fn pr_total_units() -> f32 {
    if (u.fold_enabled == 0u) {
        return f32(u.key_count);
    }
    return pr_units_before(u.key_offset + u.key_count) - pr_units_before(u.key_offset);
}

/// PR 音高行底部像素 y（横向：`total*kh - scroll - rel*kh`）。
fn pr_key_bottom(key: u32) -> f32 {
    let base = pr_units_before(u.key_offset);
    let rel_lo = pr_units_before(key) - base;
    // 音高轴滚动量随方向取：横向 = scroll_y，纵向 = scroll_x。
    let pitch_scroll = select(u.scroll_y, u.scroll_x, u.orientation == 1u);
    return (pr_total_units() * u.key_height - pitch_scroll) - rel_lo * u.key_height;
}

/// PR 音高行 → `(top, height)` 像素（横向 y / 纵向 x）。
fn pr_key_px(key: u32) -> vec2<f32> {
    let base = pr_units_before(u.key_offset);
    let r = pr_row(key);
    let rel_lo = r.x - base;
    let h_units = r.y - r.x;
    let h_px = max(h_units * u.key_height, 1.0);
    if u.orientation == 1u {
        // 纵向：音高沿 X，key 0 在左，x = rel*kh - scroll_x（与键盘一致）。
        let left = rel_lo * u.key_height - u.scroll_x;
        return vec2<f32>(left, h_px);
    }
    let bottom = pr_key_bottom(key);
    return vec2<f32>(bottom - h_units * u.key_height, h_px);
}


struct NoteInstance {
    start_tick: u32,
    end_tick: u32,
    packed: u32, // key|track|vel
};

struct DrawIndexedIndirectArgs {
    index_count: u32,     // 6 (two triangles per note, shared index buffer)
    instance_count: u32,
    first_index: u32,     // 0
    base_vertex: i32,     // 0
    first_instance: u32,  // chunk * 256 (first sparse slot of this chunk)
};

// Per-key dispatch info (binding 4). Host-written every frame; shares the
// 256-byte slot with the dispatch_workgroups_indirect args (first 12 bytes).
// c_lo is the first dispatched chunk of the frame: chunk c of the key covers
// notes [c*256, min((c+1)*256, count)), contiguous, so the input index is
// computed directly — no lookup table.
struct DispatchInfo {
    wg_x: u32,
    wg_y: u32,
    wg_z: u32,
    count: u32,
    c_lo: u32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var<storage, read> all_instances: array<NoteInstance>;
// 可见索引缓冲：每槽 4B（u32），存「该 key 的 all_instances 内的本地索引」。
// 顶点阶段从 @group(1) 的 all_instances 间接读回完整数据。
@group(0) @binding(2) var<storage, read_write> visible_indices: array<u32>;
@group(0) @binding(3) var<storage, read_write> draw_args: array<DrawIndexedIndirectArgs>;
@group(0) @binding(4) var<storage, read> dispatch_info: DispatchInfo;
// Per-track visibility bitmask (1 bit per track). Track 显隐变化时由宿主写入；
// track 显隐全量重建期间，旧 buffer + 此 mask 双重过滤保证显示正确。
@group(0) @binding(5) var<storage, read> track_mask: array<u32>;
// Per-track selection bitmask (1 bit per track)。选中轨道置顶：cull 把可见
// 实例二分为「未选中组 / 选中组」，宿主先画未选中、再画选中（全局置顶）。
@group(0) @binding(6) var<storage, read> track_selected_mask: array<u32>;

// Workgroup shared memory for the two prefix sums.
// After the scan, wg_prefix[i] = unselected visible count in [0..=i];
// wg_sel[i] = selected visible count in [0..=i].
var<workgroup> wg_prefix: array<u32, 256>;
var<workgroup> wg_sel: array<u32, 256>;

@compute @workgroup_size(256)
fn main(
    @builtin(workgroup_id) wg_id: vec3<u32>,
    @builtin(local_invocation_id) local_id: vec3<u32>,
) {
    // Chunk = c_lo + global workgroup id; workgroups beyond 65535 are packed
    // into wg_id.y by the host's dispatch args (wg_y = ceil(count/65535)).
    let wg = wg_id.x + wg_id.y * 65535u;
    let chunk = dispatch_info.c_lo + wg;
    let index = chunk * 256u + local_id.x;
    // `count` is the note count at upload time (host-written in the dispatch
    // args slot). The buffer capacity can exceed it (grown buffers, shrunk
    // keys), and the tail holds stale/uninitialized data — culling those would
    // render ghost notes, so the scan bound must be `count`, not arrayLength.
    let in_range = index < dispatch_info.count;

    var visible: u32 = 0u;
    var selected: u32 = 0u;

    if in_range {
        let inst = all_instances[index];
        // tick 最高位可能是 selected/ghost 标志（ghost 层不进 cull，仍防御性掩码）。
        let start_tick = inst.start_tick & 0x7FFFFFFFu;
        let end_tick = inst.end_tick & 0x7FFFFFFFu;
        let packed = inst.packed;
        // key 9 位（256 键）| track 16 位。
        let key = packed & 0x1FFu;
        let track = (packed >> 9u) & 0xFFFFu;
        // Track 显隐 mask：隐藏轨道的音符直接跳过。mask 始终反映当前
        // track_visible（上传数据也按构建时的 track_visible 过滤过），
        // 双重过滤无害——这是「后台重建期间显示不闪错」的保证。
        let track_visible = (track_mask[track >> 5u] & (1u << (track & 31u))) != 0u;

        // Skip zero-length notes (deleted/placeholder)
        if track_visible && end_tick > start_tick {
            if u.orientation == 1u {
                // 纵向瀑布流：音高沿 X（折叠后的行位置），时间沿 Y。
                let kpx = pr_key_px(key);
                let key_x = kpx.x;
                let key_x_right = key_x + kpx.y;
                let tick_y = f32(start_tick) * u.pixels_per_tick - u.scroll_y;
                let tick_y_bottom = f32(end_tick) * u.pixels_per_tick - u.scroll_y;
                if key_x_right >= 0.0 && key_x <= u.width && tick_y_bottom >= 0.0 && tick_y <= u.height
                {
                    visible = 1u;
                }
            } else {
                let ppu = u.pixels_per_tick;
                let x_offset = u.keyboard_width - u.scroll_x;

                // X bounds in pixels. 视口左边界 = keyboard_width（键盘列/轨道面板列
                // 由 egui 层绘制，音符不画到其下方，避免浪费填充率）。
                let pixel_x = x_offset + f32(start_tick) * ppu;
                let pixel_right = x_offset + f32(end_tick) * ppu;

                if pixel_right >= u.keyboard_width && pixel_x <= u.width {
                    // Y bounds in pixels
                    var pixel_y: f32;
                    var pixel_bottom: f32;

                    if u.mode == 1u {
                        // PR: 256 键 + 调式折叠后的行位置
                        pixel_bottom = pr_key_bottom(key);
                        pixel_y = pr_key_px(key).x;
                    } else {
                        // AR: lane_height based
                        // 音域与 shader.wgsl 一致：常规 128，扩展音域 256。
                        let lh = u.lane_height;
                        let lh_per_key = lh / max(f32(u.key_count), 1.0);
                        pixel_bottom = -u.scroll_y + lh - f32(key) * lh_per_key + f32(track) * lh;
                        pixel_y = -u.scroll_y + lh - (f32(key) + 1.0) * lh_per_key + f32(track) * lh;
                    }

                    if pixel_bottom >= 0.0 && pixel_y <= u.height {
                        visible = 1u;
                    }
                }
            }
        }
        if visible == 1u {
            selected = select(0u, 1u, (track_selected_mask[track >> 5u] & (1u << (track & 31u))) != 0u);
        }
    }

    // Phase 1: two inclusive prefix sums (Hillis-Steele scan, 8 steps for 256
    // threads). wg_prefix = unselected visible count in [0..=i]; wg_sel =
    // selected visible count in [0..=i]. Selected tracks are partitioned to the
    // back of the chunk's slots so the host can draw all unselected chunks
    // before all selected chunks (global selected-on-top).
    wg_prefix[local_id.x] = visible & (1u - selected);
    wg_sel[local_id.x] = visible & selected;
    workgroupBarrier();

    var stride: u32 = 1u;
    while stride < 256u {
        var val0: u32 = 0u;
        var val1: u32 = 0u;
        if local_id.x >= stride {
            val0 = wg_prefix[local_id.x - stride];
            val1 = wg_sel[local_id.x - stride];
        }
        workgroupBarrier();
        wg_prefix[local_id.x] += val0;
        wg_sel[local_id.x] += val1;
        workgroupBarrier();
        stride *= 2u;
    }

    // Phase 2: thread 0 writes this chunk's two draw args lists (A = unselected
    // at relative index `wg`, B = selected at `wg + chunk_total`). chunk_total =
    // ceil(count/256) is the per-key chunk capacity, so list B sits after list A
    // in the same buffer. Visible threads write to fixed sparse slots: unselected
    // at chunk*256 + rank - 1, selected at chunk*256 + n0 + rank - 1.
    let n0 = wg_prefix[255u];
    let n1 = wg_sel[255u];
    let chunk_total = (dispatch_info.count + 255u) / 256u;
    if local_id.x == 0u {
        draw_args[wg] = DrawIndexedIndirectArgs(6u, n0, 0u, 0i, chunk * 256u);
        draw_args[wg + chunk_total] =
            DrawIndexedIndirectArgs(6u, n1, 0u, 0i, chunk * 256u + n0);
    }
    if visible == 1u {
        var dst: u32;
        if selected == 1u {
            dst = chunk * 256u + n0 + wg_sel[local_id.x] - 1u;
        } else {
            dst = chunk * 256u + wg_prefix[local_id.x] - 1u;
        }
        if dst < arrayLength(&visible_indices) {
            visible_indices[dst] = index;
        }
    }
}

