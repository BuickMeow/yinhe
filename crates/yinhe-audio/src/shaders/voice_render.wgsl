// GPU audio voice rendering — single-pass, per-frame workgroup, read-only voice states.
// 每个workgroup渲染一个frame，所有voice state只读，无数据竞争。
// ADSR/pan/volume 参数从 SFZ 读取，通过 voice state 传入。

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
    attack_rate: f32,
    decay_rate: f32,
    sustain_level: f32,
    release_rate: f32,
    pan_left: f32,
    pan_right: f32,
};

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

/// 用解析公式计算 frame_offset 处的 envelope 值。
/// ADSR 每个阶段都是线性斜坡，通过计算阶段转折帧数来直接得出结果。
fn envelope_at(
    initial_env: f32, initial_stage: u32, env_level: f32,
    attack_rate: f32, decay_rate: f32, sustain_level: f32, release_rate: f32,
    frame_offset: u32,
) -> f32 {
    if frame_offset == 0u { return initial_env; }

    var env = initial_env;
    var remaining = f32(frame_offset);

    // Attack: 从 env 上升到 env_level
    if initial_stage <= 0u {
        let env_to_peak = max(env_level - env, 0.0);
        let attack_frames = env_to_peak / attack_rate;
        if remaining <= attack_frames {
            return env + attack_rate * remaining;
        }
        env = env_level;
        remaining -= attack_frames;
    }

    // Decay: 从 env_level 下降到 sustain * env_level
    if initial_stage <= 1u {
        let sus = sustain_level * env_level;
        let env_to_sus = max(env - sus, 0.0);
        let decay_frames = env_to_sus / decay_rate;
        if remaining <= decay_frames {
            return env - decay_rate * remaining;
        }
        env = sus;
        remaining -= decay_frames;
    }

    // Sustain: 保持不变
    if initial_stage <= 2u {
        let sus = sustain_level * env_level;
        if initial_stage == 2u {
            return sus; // sustain 永远不变（release 由 NoteOff 触发）
        }
        // 刚从 decay 进入 sustain
        return sus;
    }

    // Release: 从 env 下降到 0
    // initial_stage == 3
    let release_frames = env / release_rate;
    if remaining <= release_frames {
        return env - release_rate * remaining;
    }
    return 0.0;
}

@compute @workgroup_size(256)
fn vs_main(@builtin(workgroup_id) wid: vec3<u32>,
           @builtin(local_invocation_id) lid: vec3<u32>) {
    let fi = wid.x;
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

        if fi < (*voice).start_offset { continue; }
        let frame_in_voice = fi - (*voice).start_offset;

        let t = (*voice).time + f32(frame_in_voice) * (*voice).speed;
        let idx = u32(t);
        let frac = t - f32(idx);
        let max_idx = (*voice).sample_length - 1u;
        if idx >= (*voice).sample_length { continue; }

        let a = sample_at((*voice).sample_offset + min(idx, max_idx));
        let b = sample_at((*voice).sample_offset + min(idx + 1u, max_idx));
        let env = envelope_at(
            (*voice).envelope, (*voice).env_stage, (*voice).env_level,
            (*voice).attack_rate, (*voice).decay_rate,
            (*voice).sustain_level, (*voice).release_rate,
            frame_in_voice,
        );
        let s = mix(a, b, frac) * (*voice).gain * env;
        my_left += s * (*voice).pan_left;
        my_right += s * (*voice).pan_right;
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
        // hard clip 限幅
        let l = clamp(shared_left[0], -1.0, 1.0);
        let r = clamp(shared_right[0], -1.0, 1.0);
        final_output[fi * 2u] = l;
        final_output[fi * 2u + 1u] = r;
    }
}
