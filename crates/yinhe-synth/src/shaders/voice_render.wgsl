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
    seg_count: u32,      // 块内段数（段边界 = CC 事件位置）
    release_count: u32,  // release/kill 指令总数
    env_update_count: u32, // CC72/73/121 包络更新指令总数
};

struct VoiceState {
    // Sample playback
    sample_offset: u32,
    sample_length: u32,
    speed: f32,
    /// 音色库基础播放倍率（段边界按通道 pitch_multiplier 重算 speed = base × mult）
    base_speed: f32,
    base_gain: f32,
    time: f32,
    start_offset: u32,
    // MIDI 通道（0..31，pass2 按通道归约到 channel_mix）
    channel: u32,
    // Envelope state at start of block
    envelope: f32,
    env_stage: u32,      // 0=Delay..6=Finished
    stage_progress: f32,
    // Envelope parameters
    env_level: f32,
    sustain_level: f32,
    env_start: f32,
    // Decay 阶段起点 amp（正常 = peak；CC72/73 重走 Decay 时 = 当前 amp）
    decay_start: f32,
    // Stage durations (frames)
    delay_frames: f32,
    attack_frames: f32,
    hold_frames: f32,
    decay_frames: f32,
    release_frames: f32,
    // 声像：音色库基础声像（通道 pan 渐变逐帧计算，见 ch_pan）
    base_pan_l: f32,
    base_pan_r: f32,
    // 通道渐变状态（xsynth ValueLerp：CC7/10/11 10ms 线性渐变，逐帧推进）
    ch_vol: f32,
    ch_vol_step: f32,
    ch_vol_frames: u32,
    ch_expr: f32,
    ch_expr_step: f32,
    ch_expr_frames: u32,
    ch_pan: f32,
    ch_pan_step: f32,
    ch_pan_frames: u32,
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

/// 段信息：块内段边界（第 0 段恒从帧 0 开始，最后一段到 frame_count）。
/// ch_off/ch_count 指向 ch_updates 中本段的通道更新区间。
struct SegInfo {
    start_frame: u32,
    ch_off: u32,
    ch_count: u32,
    _pad: u32,
};

/// 段边界处某通道的新状态（CC 事件后）。voice 在跨段时同步。
struct ChState {
    ch: u32,
    speed_mult: f32,
    ch_vol: f32,
    ch_vol_step: f32,
    ch_vol_frames: u32,
    ch_expr: f32,
    ch_expr_step: f32,
    ch_expr_frames: u32,
    ch_pan: f32,
    ch_pan_step: f32,
    ch_pan_frames: u32,
};

/// release/kill 指令：在 frame 帧对 vid 应用（mode 5=release，6=kill）。
struct ReleaseCmd {
    frame: u32,
    vid: u32,
    mode: u32,
    _pad: u32,
};

/// CC72/73/121 包络更新指令：frame 帧对 vid 重算 attack/release 时长。
struct EnvUpdateCmd {
    frame: u32,
    vid: u32,
    attack_frames: f32,
    release_frames: f32,
};

@group(0) @binding(0) var<uniform> params: RenderParams;
@group(0) @binding(1) var<storage, read_write> voice_states: array<VoiceState>;
@group(0) @binding(2) var<storage, read_write> channel_mix: array<f32>;
@group(0) @binding(3) var<storage, read> chunk_0: array<f32>;
@group(0) @binding(4) var<storage, read> chunk_1: array<f32>;
@group(0) @binding(5) var<storage, read> chunk_2: array<f32>;
@group(0) @binding(6) var<storage, read> chunk_3: array<f32>;
@group(0) @binding(7) var<storage, read> chunk_4: array<f32>;
@group(0) @binding(9) var<storage, read_write> partial: array<f32>;
@group(0) @binding(10) var<storage, read> segs: array<SegInfo>;
@group(0) @binding(11) var<storage, read> ch_updates: array<ChState>;
@group(0) @binding(12) var<storage, read> release_by_frame: array<u32>;
@group(0) @binding(13) var<storage, read> release_cmds: array<ReleaseCmd>;
@group(0) @binding(14) var<storage, read> env_cmds: array<EnvUpdateCmd>;

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
                s.decay_start = s.envelope; // 进入 Decay 的起点 = 当前 amp（= peak）
                s.stage_progress = 0.0;
            } else {
                s.stage_progress += 1.0;
            }
        }
        case 3u: { // Decay: 指数 (1-t)^8，从 decay_start 到 sustain
            let n = s.stage_progress + 1.0;
            if n >= s.decay_frames {
                s.envelope = sus;
                s.env_stage = 4u;
                s.stage_progress = 0.0;
            } else {
                let t = n / s.decay_frames;
                s.envelope = sus + (s.decay_start - sus) * pow(1.0 - t, 8.0);
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

/// Pass 1：每线程一个 voice，串行推进 block 内所有帧，每帧结果直写
/// `partial[vid][frame]`（无 workgroup 归约，避免每帧 barrier）。
///
/// 块内按段推进：段边界（CC 事件位置）应用通道状态更新（ch_updates）与
/// release/env 指令；voice 状态在块末全字段写回 voice_states（CPU 读回为下块起点）。
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
    // 段 0 的通道更新（块起点 CC 事件）在初始化时应用
    if is_active && params.seg_count > 0u {
        let off = segs[0].ch_off;
        let cnt = segs[0].ch_count;
        for (var ui: u32 = off; ui < off + cnt; ui++) {
            let cu = ch_updates[ui];
            if cu.ch == st.channel {
                st.speed = st.base_speed * cu.speed_mult;
                st.ch_vol = cu.ch_vol;
                st.ch_vol_step = cu.ch_vol_step;
                st.ch_vol_frames = cu.ch_vol_frames;
                st.ch_expr = cu.ch_expr;
                st.ch_expr_step = cu.ch_expr_step;
                st.ch_expr_frames = cu.ch_expr_frames;
                st.ch_pan = cu.ch_pan;
                st.ch_pan_step = cu.ch_pan_step;
                st.ch_pan_frames = cu.ch_pan_frames;
            }
        }
    }
    var seg_idx = 0u;
    var ended_this_block = 0u;

    for (var fi: u32 = 0u; fi < fc; fi++) {
        // 跨段：应用该段边界的通道状态更新
        while seg_idx + 1u < params.seg_count && fi >= segs[seg_idx + 1u].start_frame {
            seg_idx += 1u;
            if is_active {
                let off = segs[seg_idx].ch_off;
                let cnt = segs[seg_idx].ch_count;
                for (var ui: u32 = off; ui < off + cnt; ui++) {
                    let cu = ch_updates[ui];
                    if cu.ch == st.channel {
                        st.speed = st.base_speed * cu.speed_mult;
                        st.ch_vol = cu.ch_vol;
                        st.ch_vol_step = cu.ch_vol_step;
                        st.ch_vol_frames = cu.ch_vol_frames;
                        st.ch_expr = cu.ch_expr;
                        st.ch_expr_step = cu.ch_expr_step;
                        st.ch_expr_frames = cu.ch_expr_frames;
                        st.ch_pan = cu.ch_pan;
                        st.ch_pan_step = cu.ch_pan_step;
                        st.ch_pan_frames = cu.ch_pan_frames;
                    }
                }
            }
        }

        if is_active {
            // release/kill 指令（该帧）
            for (var ri: u32 = release_by_frame[fi]; ri < release_by_frame[fi + 1u]; ri++) {
                let rc = release_cmds[ri];
                if rc.vid == vid {
                    st.env_start = st.envelope;
                    st.env_stage = rc.mode;
                    st.stage_progress = 0.0;
                }
            }
            // CC72/73 包络更新指令（该帧）：重算时长并从当前 amp 重走当前阶段
            for (var ei: u32 = 0u; ei < params.env_update_count; ei++) {
                let ec = env_cmds[ei];
                if ec.frame == fi && ec.vid == vid {
                    st.attack_frames = ec.attack_frames;
                    st.release_frames = ec.release_frames;
                    if st.env_stage == 0u {
                        st.stage_progress = 0.0;
                    } else if st.env_stage == 1u {
                        st.env_start = st.envelope;
                        st.stage_progress = 0.0;
                    } else if st.env_stage == 2u {
                        st.stage_progress = 0.0;
                    } else if st.env_stage == 3u {
                        st.decay_start = st.envelope;
                        st.stage_progress = 0.0;
                    } else if st.env_stage == 5u {
                        st.env_start = st.envelope;
                        st.stage_progress = 0.0;
                    }
                }
            }
        }

        var my_l = 0.0;
        var my_r = 0.0;
        if is_active && st.env_stage < 6u && fi >= st.start_offset {
            // 通道渐变逐帧推进（与 xsynth ValueLerp 一致：10ms 线性渐变）
            if st.ch_vol_frames > 0u {
                st.ch_vol += st.ch_vol_step;
                st.ch_vol_frames -= 1u;
            }
            if st.ch_expr_frames > 0u {
                st.ch_expr += st.ch_expr_step;
                st.ch_expr_frames -= 1u;
            }
            if st.ch_pan_frames > 0u {
                st.ch_pan += st.ch_pan_step;
                st.ch_pan_frames -= 1u;
            }
            // 通道增益 = base × (volume×expression)²，声像 = base × cos/sin(pan·π/2)
            let ch_vol = st.ch_vol * st.ch_expr;
            let ch_gain = st.base_gain * ch_vol * ch_vol;
            let ch_ang = st.ch_pan * 1.57079632679;
            let ch_pan_l = st.base_pan_l * cos(ch_ang);
            let ch_pan_r = st.base_pan_r * sin(ch_ang);

            // 采样位置（帧索引；立体声样本交错存储，位置 = 帧 * 2）
            let t = st.time + f32(fi - st.start_offset) * st.speed;
            var idx = u32(t);
            let frac = t - f32(idx);
            let max_idx = st.sample_length - 1u;

            // 循环处理（与 xsynth 一致）——loop_mode：0=NoLoop, 1=LoopContinuous,
            // 2=LoopSustain, 3=OneShot：
            // - Continuous：恒循环；Sustain：仅未 release（env_stage < 5）时循环，
            //   release 后从当前位置继续播到尾；NoLoop/OneShot：不循环，播完即结束
            let released = st.env_stage >= 5u;
            let loop_cont = st.loop_mode == 1u;
            let loop_sus = st.loop_mode == 2u && !released;
            let has_loop = (loop_cont || loop_sus) && st.loop_end > st.loop_start;
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

                var s_l = l0 * ch_gain * st.envelope;
                var s_r = r0 * ch_gain * st.envelope;
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
                my_l = s_l * ch_pan_l;
                my_r = s_r * ch_pan_r;
            } else if !loop_cont {
                // 采样播完（NoLoop/OneShot/LoopSustain release 后）：结束 voice。
                // 与 xsynth `is_past_end` 一致（Continuous 恒循环，永不因采样结束）。
                st.env_stage = 6u;
            }
            let prev_stage = st.env_stage;
            st = advance_env(st);
            if is_active && prev_stage < 6u && st.env_stage == 6u {
                ended_this_block = 1u;
            }
        }

        // 直写自己的 slot（pass2 按通道归约；无 workgroup 同步）
        partial[vid * fc * 2u + fi * 2u] = my_l;
        partial[vid * fc * 2u + fi * 2u + 1u] = my_r;
    }

    // 全字段写回（CPU 读回为下一块起点状态；flt_* 亦在其中）。
    // 与 CPU advance_voices 一致：消耗一次性 start_offset、推进 time。
    if is_active {
        if fc > st.start_offset {
            let act_frames = fc - st.start_offset;
            st.start_offset = 0u;
            if st.env_stage < 6u {
                st.time += st.speed * f32(act_frames);
            }
        } else {
            st.start_offset = 0u;
        }
        voice_states[vid] = st;
    }
}

/// Pass 2：每帧一个 workgroup；256 线程 = 32 通道 × 8 槽位，
/// 把 pass1 的 per-voice partial 按通道归约到 channel_mix[ch][frame][2]。
@compute @workgroup_size(256)
fn mix_main(@builtin(workgroup_id) wid: vec3<u32>,
            @builtin(local_invocation_id) lid: vec3<u32>) {
    let fi = wid.x;
    let fc = params.frame_count;
    if fi >= fc { return; }
    // 线程布局：ch = lid/8，slot = lid%8；每个 slot 扫 stride 8 的 voice。
    // 32 通道 × 8 槽位 = 256 线程，所有 vid 恰好被一个线程扫描（vid ≡ s mod 8）。
    let ch = lid.x / 8u;
    let s = lid.x % 8u;
    var sum_l = 0.0;
    var sum_r = 0.0;
    for (var vid = s; vid < params.voice_count; vid += 8u) {
        if voice_states[vid].channel == ch {
            let base = vid * fc * 2u + fi * 2u;
            sum_l += partial[base];
            sum_r += partial[base + 1u];
        }
    }

    shared_l[lid.x] = sum_l;
    shared_r[lid.x] = sum_r;
    workgroupBarrier();

    // 组内（8 槽位）树归约
    var stride = 4u;
    while stride > 0u {
        if s < stride {
            shared_l[lid.x] += shared_l[lid.x + stride];
            shared_r[lid.x] += shared_r[lid.x + stride];
        }
        workgroupBarrier();
        stride /= 2u;
    }

    if s == 0u {
        let base = (ch * fc + fi) * 2u;
        channel_mix[base] = shared_l[ch * 8u];
        channel_mix[base + 1u] = shared_r[ch * 8u];
    }
}
