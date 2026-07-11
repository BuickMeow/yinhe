// GPU audio voice rendering — single-pass, per-frame workgroup, read-only voice states.
// 每个workgroup渲染一个frame，所有voice state只读，无数据竞争。

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
    start_offset: u32,
};

// Bindings:
// 0: params (uniform)
// 1: voice_states (storage read_only — CPU跨块维护)
// 2: final_output (storage read_write)
// 3-7: 5 sample chunks (storage read)
// 8: chunk_offsets (uniform)

@group(0) @binding(0) var<uniform> params: RenderParams;
@group(0) @binding(1) var<storage, read> voice_states: array<VoiceState>;
@group(0) @binding(2) var<storage, read_write> final_output: array<f32>;
@group(0) @binding(3) var<storage, read> chunk_0: array<f32>;
@group(0) @binding(4) var<storage, read> chunk_1: array<f32>;
@group(0) @binding(5) var<storage, read> chunk_2: array<f32>;
@group(0) @binding(6) var<storage, read> chunk_3: array<f32>;
@group(0) @binding(7) var<storage, read> chunk_4: array<f32>;

const MAX_CHUNKS: u32 = 5u;
const CHUNK_SIZE: u32 = 30000000u;

struct ChunkOffsets {
    o0: u32, o1: u32, o2: u32, o3: u32, o4: u32, total: u32,
    _pad0: u32, _pad1: u32,
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

/// 根据voice的初始状态和frame偏移，计算该frame的envelope值。
/// 与CPU实现完全对应：每帧推进ADSR。
fn envelope_at(initial_env: f32, initial_stage: u32, env_level: f32, frame_offset: u32) -> f32 {
    var env = initial_env;
    var stage = initial_stage;
    for (var i: u32 = 0u; i < frame_offset; i++) {
        if stage >= 4u { break; }
        switch stage {
            case 0u: {
                env += ATTACK_RATE;
                if env >= env_level { env = env_level; stage = 1u; }
            }
            case 1u: {
                env -= DECAY_RATE;
                if env <= SUSTAIN_LEVEL { env = SUSTAIN_LEVEL; stage = 2u; }
            }
            case 2u: { }
            case 3u: {
                env -= RELEASE_RATE;
                if env <= 0.0 { env = 0.0; stage = 4u; }
            }
            default: { break; }
        }
    }
    return env;
}

@compute @workgroup_size(256)
fn vs_main(@builtin(workgroup_id) wid: vec3<u32>,
           @builtin(local_invocation_id) lid: vec3<u32>) {
    let fi = wid.x;  // frame index = workgroup id
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

        // 跳过voice尚未开始的帧
        if fi < (*voice).start_offset { continue; }
        let frame_in_voice = fi - (*voice).start_offset;

        // 计算当前frame的采样位置
        let t = (*voice).time + f32(frame_in_voice) * (*voice).speed;
        let idx = u32(t);
        let frac = t - f32(idx);
        let max_idx = (*voice).sample_length - 1u;
        if idx >= (*voice).sample_length { continue; }

        let a = sample_at((*voice).sample_offset + min(idx, max_idx));
        let b = sample_at((*voice).sample_offset + min(idx + 1u, max_idx));
        let env = envelope_at((*voice).envelope, (*voice).env_stage, (*voice).env_level, frame_in_voice);
        let s = mix(a, b, frac) * (*voice).gain * env;
        my_left += s;
        my_right += s;
    }

    shared_left[lid.x] = my_left;
    shared_right[lid.x] = my_right;
    workgroupBarrier();

    // 树形归约
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
