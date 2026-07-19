// GPU compute cull for NoteInstance (16 bytes each).
//
// Per-key architecture: each MIDI key (0..127) has its own `all_notes` and
// `visible_notes` storage buffer. The host dispatches this shader once per
// key, binding that key's buffers. This removes any global visible-note
// cap — the total visible capacity equals the total note count.
//
// Input:  all_instances   - one key's note buffer (bound per-dispatch)
// Output: visible_instances - that key's visible-notes buffer (bound per-dispatch)
//         indirect_args   - that key's DrawIndirectArgs (bound per-dispatch)

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

@compute @workgroup_size(256)
fn main(
    @builtin(global_invocation_id) global_id: vec3<u32>,
) {
    let MAX_X_THREADS: u32 = 65535u * 256u;
    let index = global_id.x + global_id.y * MAX_X_THREADS;
    let in_range = index < arrayLength(&all_instances);

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
                    // Visible: atomically reserve a slot and write.
                    // No cross-key contention — each key has its own indirect_args.
                    let dst = atomicAdd(&indirect_args.instance_count, 1u);
                    if dst < arrayLength(&visible_instances) {
                        visible_instances[dst] = inst;
                    }
                }
            }
        }
    }
}
