// GPU audio voice rendering — two-pass architecture:
//  - vs_main (pass 1): 每个线程一个 voice，串行推进 block 内所有帧
//    （逐帧推进 envelope 阶段 + 立体声采样 + 插值 + per-voice biquad 滤波器），
//    每帧 workgroup 树归约写入 partial 缓冲。
//  - mix_main (pass 2): 每帧一个 workgroup，归约所有 workgroup 的 partial 到最终输出。
// 7 阶段 envelope: Delay→Attack→Hold→Decay→Sustain→Release→Finished
// Attack=线性, Decay/Release=指数(1-t)^8（与 XSynth 默认一致）
// 滤波器为 DirectForm1 biquad，系数由 CPU 按 RBJ cookbook 预计算；
// IIR 状态跨 block 持久（voice_states 为 read_write，block 末写回）。

struct RenderParams {
    frame_count: u32,
    voice_count: u32,
    sample_rate: u32,
    sample_chunk_count: u32,
    voice_wg_count: u32, // pass1 workgroup 数 = ceil(voice_count / 256)
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
    // 采样布局与插值
    is_stereo: u32,      // 0=单声道样本, 1=交错立体声
    interp: u32,         // 0=Nearest, 1=Linear
    // per-voice biquad（cutoff > 0 启用）
    cutoff: f32,         // Hz
    resonance: f32,      // 线性 Q（未在 shader 使用，保留对齐 CPU 结构）
    filter_type: u32,    // 0=LowPass, 1=HighPass, 2=BandPass, 3=SinglePoleLowPass
    flt_b0: f32,
    flt_b1: f32,
    flt_b2: f32,
    flt_a1: f32,
    flt_a2: f32,
    // DirectForm1 状态（左声道）
    flt_x1: f32,
    flt_x2: f32,
    flt_y1: f32,
    flt_y2: f32,
    // DirectForm1 状态（右声道，仅立体声样本使用）
    flt_x1r: f32,
    flt_x2r: f32,
    flt_y1r: f32,
    flt_y2r: f32,
};

@group(0) @binding(0) var<uniform> params: RenderParams;
@group(0) @binding(1) var<storage, read_write> voice_states: array<VoiceState>;
@group(0) @binding(2) var<storage, read_write> final_output: array<f32>;
@group(0) @binding(3) var<storage, read> chunk_0: array<f32>;
@group(0) @binding(4) var<storage, read> chunk_1: array<f32>;
@group(0) @binding(5) var<storage, read> chunk_2: array<f32>;
@group(0) @binding(6) var<storage, read> chunk_3: array<f32>;
@group(0) @binding(7) var<storage, read> chunk_4: array<f32>;
@group(0) @binding(9) var<storage, read_write> partial: array<f32>;

struct ChunkOffsets {
    o0: u32, o1: u32, o2: u32, o3: u32, o4: u32, total: u32,
    _pad0: u32, _pad1: u32,
};

@group(0) @binding(8) var<uniform> chunk_off: ChunkOffsets;

var<workgroup> shared_l: array<f32, 256>;
var<workgroup> shared_r: array<f32, 256>;

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

/// 推进 1 帧 envelope（与 CPU advance_voices 逐帧等价）。
/// WGSL 无指针，返回推进后的整个 VoiceState。
fn advance_env(st: VoiceState) -> VoiceState {
    var s = st;
    if s.env_stage >= 6u { return s; }
    let peak = s.env_level;
    let sus = s.sustain_level * peak;
    switch s.env_stage {
        case 0u: { // Delay
            if s.stage_progress + 1.0 >= s.delay_frames {
                s.env_stage = 1u;
                s.stage_progress = 0.0;
            } else {
                s.stage_progress += 1.0;
            }
        }
        case 1u: { // Attack: 线性
            let n = s.stage_progress + 1.0;
            if n >= s.attack_frames {
                s.envelope = peak;
                s.env_stage = 2u;
                s.stage_progress = 0.0;
            } else {
                s.envelope = s.env_start
                    + (peak - s.env_start) * (n / s.attack_frames);
                s.stage_progress = n;
            }
        }
        case 2u: { // Hold
            if s.stage_progress + 1.0 >= s.hold_frames {
                s.env_stage = 3u;
                s.stage_progress = 0.0;
            } else {
                s.stage_progress += 1.0;
            }
        }
        case 3u: { // Decay: 指数 (1-t)^8
            let n = s.stage_progress + 1.0;
            if n >= s.decay_frames {
                s.envelope = sus;
                s.env_stage = 4u;
                s.stage_progress = 0.0;
            } else {
                let t = n / s.decay_frames;
                s.envelope = sus + (peak - sus) * pow(1.0 - t, 8.0);
                s.stage_progress = n;
            }
        }
        case 4u: { // Sustain
            s.envelope = sus;
        }
        case 5u: { // Release: 指数 (1-t)^8
            let n = s.stage_progress + 1.0;
            if n >= s.release_frames {
                s.envelope = 0.0;
                s.env_stage = 6u;
                s.stage_progress = 0.0;
            } else {
                let t = n / s.release_frames;
                s.envelope = s.env_start * pow(1.0 - t, 8.0);
                s.stage_progress = n;
            }
        }
        default: { }
    }
    return s;
}

/// Pass 1：每线程一个 voice，串行推进 block 内所有帧，workgroup 归约到 partial。
@compute @workgroup_size(256)
fn vs_main(@builtin(workgroup_id) wid: vec3<u32>,
           @builtin(local_invocation_id) lid: vec3<u32>) {
    let vid = wid.x * 256u + lid.x;
    let fc = params.frame_count;
    let is_active = vid < params.voice_count;

    var st: VoiceState;
    if is_active {
        st = voice_states[vid];
    }

    for (var fi: u32 = 0u; fi < fc; fi++) {
        var my_l = 0.0;
        var my_r = 0.0;
        if is_active && st.env_stage < 6u && fi >= st.start_offset {
            // 采样位置（帧索引；立体声样本交错存储，位置 = 帧 * 2）
            let t = st.time + f32(fi - st.start_offset) * st.speed;
            var idx = u32(t);
            let frac = t - f32(idx);
            let max_idx = st.sample_length - 1u;

            // 循环处理
            let has_loop = st.loop_mode > 0u && st.loop_end > st.loop_start;
            if has_loop && idx >= st.loop_end {
                let loop_len = st.loop_end - st.loop_start;
                if loop_len > 0u {
                    idx = st.loop_start + ((idx - st.loop_start) % loop_len);
                }
            }

            if idx < st.sample_length {
                let scale = 1u + st.is_stereo;
                let i = st.sample_offset + idx * scale;
                var l0 = sample_at(i);
                var r0 = l0;
                if st.is_stereo == 1u {
                    r0 = sample_at(i + 1u);
                }
                if st.interp == 1u && idx < max_idx {
                    var l1 = sample_at(i + scale);
                    var r1 = l1;
                    if st.is_stereo == 1u {
                        r1 = sample_at(i + scale + 1u);
                    }
                    l0 = mix(l0, l1, frac);
                    r0 = mix(r0, r1, frac);
                }

                var s_l = l0 * st.gain * st.envelope;
                var s_r = r0 * st.gain * st.envelope;
                if st.cutoff > 0.0 {
                    // DirectForm1 biquad：y = b0*x + b1*x1 + b2*x2 - a1*y1 - a2*y2
                    // 单声道样本只用一组滤波器，右声道复用左声道输出（与 xsynth mono 一致）
                    var x1 = st.flt_x1;
                    var x2 = st.flt_x2;
                    var y1 = st.flt_y1;
                    var y2 = st.flt_y2;
                    let out_l = st.flt_b0 * s_l + st.flt_b1 * x1 + st.flt_b2 * x2
                        - st.flt_a1 * y1 - st.flt_a2 * y2;
                    st.flt_x1 = s_l;
                    st.flt_x2 = x1;
                    st.flt_y1 = out_l;
                    st.flt_y2 = y1;
                    s_l = out_l;
                    if st.is_stereo == 1u {
                        var x1r = st.flt_x1r;
                        var x2r = st.flt_x2r;
                        var y1r = st.flt_y1r;
                        var y2r = st.flt_y2r;
                        let out_r = st.flt_b0 * s_r + st.flt_b1 * x1r + st.flt_b2 * x2r
                            - st.flt_a1 * y1r - st.flt_a2 * y2r;
                        st.flt_x1r = s_r;
                        st.flt_x2r = x1r;
                        st.flt_y1r = out_r;
                        st.flt_y2r = y1r;
                        s_r = out_r;
                    } else {
                        s_r = s_l;
                    }
                }
                my_l = s_l * st.pan_left;
                my_r = s_r * st.pan_right;
            }
            st = advance_env(st);
        }

        shared_l[lid.x] = my_l;
        shared_r[lid.x] = my_r;
        workgroupBarrier();

        var stride = 128u;
        while stride > 0u {
            if lid.x < stride {
                shared_l[lid.x] += shared_l[lid.x + stride];
                shared_r[lid.x] += shared_r[lid.x + stride];
            }
            workgroupBarrier();
            stride /= 2u;
        }

        if lid.x == 0u {
            let base = wid.x * fc * 2u + fi * 2u;
            partial[base] = shared_l[0];
            partial[base + 1u] = shared_r[0];
        }
        workgroupBarrier();
    }

    // 写回滤波器状态（跨 block 持久；其余字段由 CPU advance_voices 推进）
    if is_active {
        voice_states[vid].flt_x1 = st.flt_x1;
        voice_states[vid].flt_x2 = st.flt_x2;
        voice_states[vid].flt_y1 = st.flt_y1;
        voice_states[vid].flt_y2 = st.flt_y2;
        voice_states[vid].flt_x1r = st.flt_x1r;
        voice_states[vid].flt_x2r = st.flt_x2r;
        voice_states[vid].flt_y1r = st.flt_y1r;
        voice_states[vid].flt_y2r = st.flt_y2r;
    }
}

/// Pass 2：每帧一个 workgroup，归约所有 pass1 workgroup 的 partial。
@compute @workgroup_size(256)
fn mix_main(@builtin(workgroup_id) wid: vec3<u32>,
            @builtin(local_invocation_id) lid: vec3<u32>) {
    let fi = wid.x;
    let fc = params.frame_count;
    if fi >= fc { return; }
    let wgc = params.voice_wg_count;
    let iters = (wgc + 255u) / 256u;
    var sum_l = 0.0;
    var sum_r = 0.0;
    for (var it: u32 = 0u; it < iters; it++) {
        let wg = lid.x + it * 256u;
        if wg < wgc {
            let base = wg * fc * 2u + fi * 2u;
            sum_l += partial[base];
            sum_r += partial[base + 1u];
        }
    }

    shared_l[lid.x] = sum_l;
    shared_r[lid.x] = sum_r;
    workgroupBarrier();

    var stride = 128u;
    while stride > 0u {
        if lid.x < stride {
            shared_l[lid.x] += shared_l[lid.x + stride];
            shared_r[lid.x] += shared_r[lid.x + stride];
        }
        workgroupBarrier();
        stride /= 2u;
    }

    if lid.x == 0u {
        final_output[fi * 2u] = shared_l[0];
        final_output[fi * 2u + 1u] = shared_r[0];
    }
}
