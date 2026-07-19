// GPU compute cull for NoteInstance (16 bytes each).
//
// Per-key architecture: each MIDI key (0..127) has its own `all_notes` and
// `visible_notes` storage buffer. The host dispatches this shader once per
// key, binding that key's buffers. This removes any global visible-note
// cap — the total visible capacity equals the total note count.
//
// Z-order stability: uses workgroup prefix sum (Hillis-Steele scan) instead
// of per-instance atomicAdd. This guarantees that within each workgroup,
// visible instances are written to the output buffer in the same order as
// they appear in `all_instances` (= all_notes order = tick order). Since
// overlapping notes (same key, same tick, different tracks) are adjacent in
// the buffer and typically within the same workgroup (256 instances), their
// z-order is deterministic across frames — no flickering.

struct Uniforms {
    width: f32,
    height: f32,
    scroll_x: f32,
    scroll_y: f32,
    pixels_per_tick: f32,
    key_height: f32,
    keyboard_width: f32,
    mode: u32,
    scroll_frac: f32,
    scroll_mode: u32,
    min_border_width: f32,
    track_count: u32,
    sel_rect_count: u32,
    note_outline: u32,
    lane_height: f32,
    value_zoom: f32,
    value_scroll: f32,
};

struct NoteInstance {
    data: vec4<u32>, // start_tick, end_tick, packed(key|track|vel), reserved
};

struct DrawIndirectArgs {
    vertex_count: u32,     // 6 (two triangles per note)
    instance_count: atomic<u32>,
    first_vertex: u32,     // 0
    first_instance: u32,   // 0
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var<storage, read> all_instances: array<NoteInstance>;
@group(0) @binding(2) var<storage, read_write> visible_instances: array<NoteInstance>;
@group(0) @binding(3) var<storage, read_write> indirect_args: DrawIndirectArgs;

// Workgroup shared memory for prefix sum.
// After the scan, wg_prefix[i] = number of visible instances in [0..=i].
var<workgroup> wg_prefix: array<u32, 256>;
// Base offset of this workgroup in visible_instances (set by thread 0).
var<workgroup> wg_base: u32;

@compute @workgroup_size(256)
fn main(
    @builtin(global_invocation_id) global_id: vec3<u32>,
    @builtin(local_invocation_id) local_id: vec3<u32>,
) {
    let MAX_X_THREADS: u32 = 65535u * 256u;
    let index = global_id.x + global_id.y * MAX_X_THREADS;
    let in_range = index < arrayLength(&all_instances);

    var visible: u32 = 0u;

    if in_range {
        let inst = all_instances[index];
        let start_tick = inst.data.x;
        let end_tick = inst.data.y;
        let packed = inst.data.z;
        let key = packed & 0xFFu;

        // Skip zero-length notes (deleted/placeholder)
        if end_tick > start_tick {
            let ppu = u.pixels_per_tick;
            let x_offset = u.keyboard_width - u.scroll_x;

            // X bounds in pixels
            let pixel_x = x_offset + f32(start_tick) * ppu;
            let pixel_right = x_offset + f32(end_tick) * ppu;

            if pixel_right >= 0.0 && pixel_x <= u.width {
                // Y bounds in pixels
                var pixel_y: f32;
                var pixel_bottom: f32;

                if u.mode == 1u {
                    // PR: key_height based
                    let bottom = 128.0 * u.key_height - u.scroll_y;
                    pixel_bottom = bottom - f32(key) * u.key_height;
                    pixel_y = bottom - (f32(key) + 1.0) * u.key_height;
                } else {
                    // AR: lane_height based
                    let track = (packed >> 8u) & 0xFFFFu;
                    let lh = u.lane_height;
                    let lh_per_key = lh / 128.0;
                    pixel_bottom = -u.scroll_y + lh - f32(key) * lh_per_key + f32(track) * lh;
                    pixel_y = -u.scroll_y + lh - (f32(key) + 1.0) * lh_per_key + f32(track) * lh;
                }

                if pixel_bottom >= 0.0 && pixel_y <= u.height {
                    visible = 1u;
                }
            }
        }
    }

    // Phase 1: inclusive prefix sum (Hillis-Steele scan, 8 steps for 256 threads).
    // wg_prefix[i] = count of visible instances in [0..=i] within this workgroup.
    wg_prefix[local_id.x] = visible;
    workgroupBarrier();

    var stride: u32 = 1u;
    while stride < 256u {
        var val: u32 = 0u;
        if local_id.x >= stride {
            val = wg_prefix[local_id.x - stride];
        }
        workgroupBarrier();
        wg_prefix[local_id.x] += val;
        workgroupBarrier();
        stride *= 2u;
    }

    // Phase 2: thread 0 reserves a contiguous block for this workgroup.
    if local_id.x == 0u {
        let wg_total = wg_prefix[255];
        wg_base = atomicAdd(&indirect_args.instance_count, wg_total);
    }
    workgroupBarrier();

    // Phase 3: write visible instances in deterministic order.
    // For a visible thread i, its position = wg_base + prefix[i] - 1
    // (inclusive scan: prefix[i] = count in [0..=i], so 0-indexed pos = prefix[i] - 1).
    if visible == 1u {
        let dst = wg_base + wg_prefix[local_id.x] - 1u;
        if dst < arrayLength(&visible_instances) {
            visible_instances[dst] = all_instances[index];
        }
    }
}
