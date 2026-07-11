// GPU audio voice rendering — single-pass, per-frame workgroup, read-only voice states.
// 7 阶段 envelope: Delay→Attack→Hold→Decay→Sustain→Release→Finished
// Attack=线性, Decay/Release=指数(1-t)^8（与 XSynth 默认一致）

struct RenderParams {
    frame_count: u32,
    voice_count: u32,
    sample_rate: u32,
    sample_chunk_count: u32,
};

struct VoiceState {
    // Sample playback
    sample_offset: u32,
    sample_length: u32,
    speed: f32,
    gain: f32,
    time: f32,
    start_offset: u32,
    // Envelope state at start of block
    envelope: f32,
    env_stage: u32,      // 0=Delay..6=Finished
    stage_progress: f32,
    // Envelope parameters
    env_level: f32,
    sustain_level: f32,
    env_start: f32,
    // Stage durations (frames)
    delay_frames: f32,
    attack_frames: f32,
    hold_frames: f32,
    decay_frames: f32,
    release_frames: f32,
    // Pan
    pan_left: f32,
    pan_right: f32,
    // Loop
    loop_start: u32,
    loop_end: u32,
    loop_mode: u32,
};

@group(0) @binding(0) var<uniform> params: RenderParams;
@group(0) @binding(1) var<storage, read> voice_states: array<VoiceState>;
@group(0) @binding(2) var<storage, read_write> final_output: array<f32>;
@group(0) @binding(3) var<storage, read> chunk_0: array<f32>;
@group(0) @binding(4) var<storage, read> chunk_1: array<f32>;
@group(0) @binding(5) var<storage, read> chunk_2: array<f32>;
@group(0) @binding(6) var<storage, read> chunk_3: array<f32>;
@group(0) @binding(7) var<storage, read> chunk_4: array<f32>;

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

/// 用解析公式计算 frame_offset 处的 envelope 值（内联到调用处）。
/// 7 阶段: 0=Delay, 1=Attack(线性), 2=Hold, 3=Decay(指数), 4=Sustain, 5=Release(指数), 6=Finished
fn envelope_at(
    stage: u32, progress: f32,
    env_start: f32, env_level: f32, sustain_level: f32,
    delay_frames: f32, attack_frames: f32, hold_frames: f32,
    decay_frames: f32, release_frames: f32,
) -> f32 {
    if stage >= 6u { return 0.0; }

    let peak = env_level;
    let sus = sustain_level * peak;

    switch stage {
        case 0u: { return env_start; } // Delay
        case 1u: { // Attack: 线性
            let t = select(1.0, progress / attack_frames, attack_frames > 0.0);
            return env_start + (peak - env_start) * min(t, 1.0);
        }
        case 2u: { return peak; } // Hold
        case 3u: { // Decay: 指数 (1-t)^8
            let t = select(1.0, progress / decay_frames, decay_frames > 0.0);
            return sus + (peak - sus) * pow(1.0 - min(t, 1.0), 8.0);
        }
        case 4u: { return sus; } // Sustain
        case 5u: { // Release: 指数 (1-t)^8，从 release 起始值衰减到 0
            let t = select(1.0, progress / release_frames, release_frames > 0.0);
            return env_start * pow(1.0 - min(t, 1.0), 8.0);
        }
        default: { return 0.0; }
    }
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
        if (*voice).env_stage >= 6u { continue; }

        if fi < (*voice).start_offset { continue; }
        let frame_in_voice = fi - (*voice).start_offset;
        let progress = (*voice).stage_progress + f32(frame_in_voice);

        let t = (*voice).time + f32(frame_in_voice) * (*voice).speed;
        var idx = u32(t);
        let frac = t - f32(idx);
        let max_idx = (*voice).sample_length - 1u;

        // 循环处理
        let has_loop = (*voice).loop_mode > 0u && (*voice).loop_end > (*voice).loop_start;
        if has_loop && idx >= (*voice).loop_end {
            let loop_len = (*voice).loop_end - (*voice).loop_start;
            if loop_len > 0u {
                idx = (*voice).loop_start + ((idx - (*voice).loop_start) % loop_len);
            }
        }

        if idx >= (*voice).sample_length { continue; }

        let a = sample_at((*voice).sample_offset + min(idx, max_idx));
        let b = sample_at((*voice).sample_offset + min(idx + 1u, max_idx));
        let env = envelope_at(
            (*voice).env_stage, progress,
            (*voice).env_start, (*voice).env_level, (*voice).sustain_level,
            (*voice).delay_frames, (*voice).attack_frames, (*voice).hold_frames,
            (*voice).decay_frames, (*voice).release_frames,
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
        final_output[fi * 2u] = shared_left[0];
        final_output[fi * 2u + 1u] = shared_right[0];
    }
}
