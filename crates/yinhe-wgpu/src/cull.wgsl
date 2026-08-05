// GPU compute cull for NoteInstance (12 bytes each).
//
// Per-key architecture: each MIDI key (0..127) has its own `all_notes` and
// `visible_indices` storage buffer. The host dispatches this shader once per
// key, binding that key's buffers. This removes any global visible-note
// cap — the total visible capacity equals the total note count.
//
// Output: fixed-slot sparse. Chunk c writes its visible note indices to the
// fixed sparse slots [c*256, c*256+256) of `visible_indices` (rank-1 within
// the chunk's prefix sum), and thread 0 writes the chunk's draw args
// (DrawIndexedIndirectArgs: index_count=6, instance_count=wg_total,
// first_index=0, base_vertex=0, first_instance=c*256) into `draw_args[wg]` —
// a relative index aligned with multi_draw_indexed_indirect, which reads draw
// args starting from index 0. The host draws with multi_draw_indexed_indirect
// in chunk order, so the output (z) order equals the input order = tick order
// — deterministic across frames, with no atomics and no dependence on GPU
// workgroup scheduling.
//
// Within a chunk, a workgroup prefix sum (Hillis-Steele scan) guarantees that
// visible instances are written in the same order as they appear in
// `all_instances` (= all_notes order = tick order). Overlapping notes (same
// key, same tick, different tracks) are adjacent in the input, so their
// z-order is stable across frames — no flickering.
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

// Workgroup shared memory for prefix sum.
// After the scan, wg_prefix[i] = number of visible instances in [0..=i].
var<workgroup> wg_prefix: array<u32, 256>;

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

    if in_range {
        let inst = all_instances[index];
        let start_tick = inst.start_tick;
        let end_tick = inst.end_tick;
        let packed = inst.packed;
        let key = packed & 0xFFu;
        let track = (packed >> 8u) & 0xFFFFu;
        // Track 显隐 mask：隐藏轨道的音符直接跳过。mask 始终反映当前
        // track_visible（上传数据也按构建时的 track_visible 过滤过），
        // 双重过滤无害——这是「后台重建期间显示不闪错」的保证。
        let track_visible = (track_mask[track >> 5u] & (1u << (track & 31u))) != 0u;

        // Skip zero-length notes (deleted/placeholder)
        if track_visible && end_tick > start_tick {
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

    // Phase 2: thread 0 writes this chunk's draw args at the relative index
    // `wg` (multi_draw_indexed_indirect reads args from index 0). Visible
    // threads write to fixed sparse slots (chunk * 256 + rank - 1), so the
    // output order is fully deterministic: (chunk, rank) == input order —
    // stable z-order across frames, no atomics, no scheduling dependence.
    if local_id.x == 0u {
        draw_args[wg] = DrawIndexedIndirectArgs(6u, wg_prefix[255u], 0u, 0i, chunk * 256u);
    }
    if visible == 1u {
        let dst = chunk * 256u + wg_prefix[local_id.x] - 1u;
        if dst < arrayLength(&visible_indices) {
            visible_indices[dst] = index;
        }
    }
}

