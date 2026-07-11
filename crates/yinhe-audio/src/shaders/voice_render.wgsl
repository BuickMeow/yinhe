// GPU audio voice rendering — single-pass, TILE=1, multi-chunk samples.
// 8 storage buffer limit: voice_states + final_output + chunk_offsets(uniform) + 5 sample chunks.

struct RenderParams {
    frame_count: u32,
    voice_count: u32,
    sample_rate: u32,
    sample_chunk_count: u32,
};

struct VoiceState {
    sample_offset: u32,
    sample_length: u32,
    speed: f32,
    gain: f32,
    time: f32,
    envelope: f32,
    env_stage: u32,
    env_level: f32,
    _pad2: u32,
};

// Bindings (8 storage buffers max):
// 0: params (uniform) + chunk_offsets (packed after params)
// 1: voice_states (storage read_write)
// 2: final_output (storage read_write)
// 3-7: 5 sample chunks (storage read)

@group(0) @binding(0) var<uniform> params: RenderParams;
@group(0) @binding(1) var<storage, read_write> voice_states: array<VoiceState>;
@group(0) @binding(2) var<storage, read_write> final_output: array<f32>;
@group(0) @binding(3) var<storage, read> chunk_0: array<f32>;
@group(0) @binding(4) var<storage, read> chunk_1: array<f32>;
@group(0) @binding(5) var<storage, read> chunk_2: array<f32>;
@group(0) @binding(6) var<storage, read> chunk_3: array<f32>;
@group(0) @binding(7) var<storage, read> chunk_4: array<f32>;

// Chunk offsets in a uniform struct (max 5 chunks + 1 sentinel = 6 u32)
const MAX_CHUNKS: u32 = 5u;
const CHUNK_SIZE: u32 = 30000000u; // 30M f32 = 120MB per chunk

struct ChunkOffsets {
    o0: u32, o1: u32, o2: u32, o3: u32, o4: u32, total: u32,
};

@group(0) @binding(8) var<uniform> chunk_off: ChunkOffsets;

const ATTACK_RATE: f32 = 0.01;
const DECAY_RATE: f32 = 0.005;
const SUSTAIN_LEVEL: f32 = 0.7;
const RELEASE_RATE: f32 = 0.02;

var<workgroup> shared_left: array<f32, 256>;
var<workgroup> shared_right: array<f32, 256>;

fn chunk_offset(idx: u32) -> u32 {
    switch idx {
        case 0u: { return chunk_off.o0; }
        case 1u: { return chunk_off.o1; }
        case 2u: { return chunk_off.o2; }
        case 3u: { return chunk_off.o3; }
        case 4u: { return chunk_off.o4; }
        default: { return chunk_off.total; }
    }
}

fn sample_at(global_idx: u32) -> f32 {
    var lo = 0u;
    var hi = params.sample_chunk_count;
    while lo < hi {
        let mid = (lo + hi) / 2u;
        if chunk_offset(mid) <= global_idx { lo = mid + 1u; } else { hi = mid; }
    }
    let chunk_idx = lo - 1u;
    let local_idx = global_idx - chunk_offset(chunk_idx);

    switch chunk_idx {
        case 0u: { return chunk_0[local_idx]; }
        case 1u: { return chunk_1[local_idx]; }
        case 2u: { return chunk_2[local_idx]; }
        case 3u: { return chunk_3[local_idx]; }
        case 4u: { return chunk_4[local_idx]; }
        default: { return 0.0; }
    }
}

@compute @workgroup_size(256)
fn vs_main(@builtin(global_invocation_id) gid: vec3<u32>,
           @builtin(local_invocation_id) lid: vec3<u32>) {
    let fi = gid.x;
    let fc = params.frame_count;
    let vc = params.voice_count;
    if fi >= fc { return; }

    var my_left: f32 = 0.0;
    var my_right: f32 = 0.0;
    let threads = 256u;
    let iterations = (vc + threads - 1u) / threads;

    for (var iter: u32 = 0u; iter < iterations; iter++) {
        let v = lid.x + iter * threads;
        if v >= vc { break; }
        let voice = &voice_states[v];
        if (*voice).env_stage >= 4u { continue; }

        switch (*voice).env_stage {
            case 0u: { (*voice).envelope += ATTACK_RATE; if (*voice).envelope >= (*voice).env_level { (*voice).envelope = (*voice).env_level; (*voice).env_stage = 1u; } }
            case 1u: { (*voice).envelope -= DECAY_RATE; if (*voice).envelope <= SUSTAIN_LEVEL { (*voice).envelope = SUSTAIN_LEVEL; (*voice).env_stage = 2u; } }
            case 2u: { }
            case 3u: { (*voice).envelope -= RELEASE_RATE; if (*voice).envelope <= 0.0 { (*voice).envelope = 0.0; (*voice).env_stage = 4u; } }
            default: { continue; }
        }

        let t = (*voice).time;
        let idx = u32(t);
        let frac = t - f32(idx);
        let max_idx = (*voice).sample_length - 1u;
        let a = sample_at((*voice).sample_offset + min(idx, max_idx));
        let b = sample_at((*voice).sample_offset + min(idx + 1u, max_idx));
        let s = mix(a, b, frac) * (*voice).gain * (*voice).envelope;
        my_left += s;
        my_right += s;
        (*voice).time += (*voice).speed;
    }

    shared_left[lid.x] = my_left;
    shared_right[lid.x] = my_right;
    workgroupBarrier();

    var stride = 128u;
    while stride > 0u {
        if lid.x < stride {
            shared_left[lid.x] += shared_left[lid.x + stride];
            shared_right[lid.x] += shared_right[lid.x + stride];
        }
        workgroupBarrier();
        stride /= 2u;
    }

    if lid.x == 0u {
        final_output[fi * 2u] = shared_left[0];
        final_output[fi * 2u + 1u] = shared_right[0];
    }
}
