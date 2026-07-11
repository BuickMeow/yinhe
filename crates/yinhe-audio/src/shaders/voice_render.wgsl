// GPU audio voice rendering — single-pass, TILE=1.
// Each workgroup handles 1 output frame. 256 threads cooperate
// to sum all voices for that frame via shared memory tree reduction.
// This minimizes synchronization overhead (1 barrier per workgroup).

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
    env_stage: u32,
    env_level: f32,
    _pad2: u32,
};

@group(0) @binding(0) var<storage, read> voice_samples: array<f32>;
@group(0) @binding(1) var<storage, read_write> voice_states: array<VoiceState>;
@group(0) @binding(2) var<storage, read_write> final_output: array<f32>;
@group(0) @binding(3) var<uniform> params: RenderParams;

const ATTACK_RATE: f32 = 0.01;
const DECAY_RATE: f32 = 0.005;
const SUSTAIN_LEVEL: f32 = 0.7;
const RELEASE_RATE: f32 = 0.02;

var<workgroup> shared_left: array<f32, 256>;
var<workgroup> shared_right: array<f32, 256>;

@compute @workgroup_size(256)
fn vs_main(@builtin(global_invocation_id) gid: vec3<u32>,
           @builtin(local_invocation_id) lid: vec3<u32>) {
    let fi = gid.x;
    let fc = params.frame_count;
    let vc = params.voice_count;

    if fi >= fc {
        return;
    }

    var my_left: f32 = 0.0;
    var my_right: f32 = 0.0;

    let threads = 256u;
    let iterations = (vc + threads - 1u) / threads;

    for (var iter: u32 = 0u; iter < iterations; iter++) {
        let v = lid.x + iter * threads;
        if v >= vc {
            break;
        }

        let voice = &voice_states[v];
        if (*voice).env_stage >= 4u {
            continue;
        }

        switch (*voice).env_stage {
            case 0u: {
                (*voice).envelope += ATTACK_RATE;
                if (*voice).envelope >= (*voice).env_level {
                    (*voice).envelope = (*voice).env_level;
                    (*voice).env_stage = 1u;
                }
            }
            case 1u: {
                (*voice).envelope -= DECAY_RATE;
                if (*voice).envelope <= SUSTAIN_LEVEL {
                    (*voice).envelope = SUSTAIN_LEVEL;
                    (*voice).env_stage = 2u;
                }
            }
            case 2u: { /* sustain */ }
            case 3u: {
                (*voice).envelope -= RELEASE_RATE;
                if (*voice).envelope <= 0.0 {
                    (*voice).envelope = 0.0;
                    (*voice).env_stage = 4u;
                }
            }
            default: { continue; }
        }

        let t = (*voice).time;
        let idx = u32(t);
        let frac = t - f32(idx);
        let max_idx = (*voice).sample_length - 1u;
        let a = voice_samples[(*voice).sample_offset + min(idx, max_idx)];
        let b = voice_samples[(*voice).sample_offset + min(idx + 1u, max_idx)];
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
