// GPU audio voice rendering compute shader
// Two entry points with separate bind groups to avoid name collisions.

struct RenderParams {
    frame_count: u32,
    voice_count: u32,
    sample_rate: u32,
    _pad: u32,
};

struct VoiceState {
    sample_offset: u32,
    sample_length: u32,
    speed: f32,
    gain: f32,
    time: f32,
    envelope: f32,
    env_stage: u32,       // 0=attack, 1=decay, 2=sustain, 3=release, 4=done
    env_level: f32,
    _pad2: u32,
};

// ── Voice render pass (group 0) ──
@group(0) @binding(0) var<storage, read> voice_samples: array<f32>;
@group(0) @binding(1) var<storage, read_write> voice_states: array<VoiceState>;
@group(0) @binding(2) var<storage, read_write> voice_out_buf: array<f32>;
@group(0) @binding(3) var<uniform> voice_params: RenderParams;

const ATTACK_RATE: f32 = 0.01;
const DECAY_RATE: f32 = 0.005;
const SUSTAIN_LEVEL: f32 = 0.7;
const RELEASE_RATE: f32 = 0.02;

@compute @workgroup_size(256)
fn vs_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let voice_idx = gid.x;
    if voice_idx >= voice_params.voice_count {
        return;
    }

    let fc = voice_params.frame_count;
    let v = &voice_states[voice_idx];
    let out_off = voice_idx * fc * 2u;

    for (var i: u32 = 0u; i < fc; i++) {
        switch (*v).env_stage {
            case 0u: {
                (*v).envelope += ATTACK_RATE;
                if (*v).envelope >= (*v).env_level {
                    (*v).envelope = (*v).env_level;
                    (*v).env_stage = 1u;
                }
            }
            case 1u: {
                (*v).envelope -= DECAY_RATE;
                if (*v).envelope <= SUSTAIN_LEVEL {
                    (*v).envelope = SUSTAIN_LEVEL;
                    (*v).env_stage = 2u;
                }
            }
            case 2u: { /* sustain */ }
            case 3u: {
                (*v).envelope -= RELEASE_RATE;
                if (*v).envelope <= 0.0 {
                    (*v).envelope = 0.0;
                    (*v).env_stage = 4u;
                }
            }
            default: {
                voice_out_buf[out_off + i * 2u] = 0.0;
                voice_out_buf[out_off + i * 2u + 1u] = 0.0;
                continue;
            }
        }

        let t = (*v).time;
        let idx = u32(t);
        let frac = t - f32(idx);
        let max_idx = (*v).sample_length - 1u;
        let a = voice_samples[(*v).sample_offset + min(idx, max_idx)];
        let b = voice_samples[(*v).sample_offset + min(idx + 1u, max_idx)];
        let s = mix(a, b, frac) * (*v).gain * (*v).envelope;

        voice_out_buf[out_off + i * 2u] = s;
        voice_out_buf[out_off + i * 2u + 1u] = s;

        (*v).time += (*v).speed;
    }
}

// ── Merge pass (group 1) ──
@group(1) @binding(0) var<storage, read> merge_voice_out: array<f32>;
@group(1) @binding(1) var<storage, read_write> merge_final_out: array<f32>;
@group(1) @binding(2) var<uniform> merge_params: RenderParams;

@compute @workgroup_size(256)
fn merge_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let fi = gid.x;
    if fi >= merge_params.frame_count {
        return;
    }

    var left: f32 = 0.0;
    var right: f32 = 0.0;

    for (var v: u32 = 0u; v < merge_params.voice_count; v++) {
        let off = v * merge_params.frame_count * 2u + fi * 2u;
        left += merge_voice_out[off];
        right += merge_voice_out[off + 1u];
    }

    merge_final_out[fi * 2u] = left;
    merge_final_out[fi * 2u + 1u] = right;
}
