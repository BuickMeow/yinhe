// GPU audio voice rendering — two-pass architecture（分段渲染）:
//  - vs_main (pass 1): 每个线程一个 voice，串行推进**本渲染段**内所有帧
//    （逐帧推进 envelope 阶段 + 立体声采样 + 插值 + per-voice biquad 滤波器），
//    直写 partial[vid][fi]（无 workgroup 同步）。
//  - mix_main (pass 2): 每帧一个 workgroup，把该帧所有 voice 的 partial
//    按通道归约到 channel_mix[ch][mix_offset + fi]。
//  外层块（如 4096 帧）在 renderer 内切成若干段（RENDER_SEGMENT_FRAMES），
//  每段独立跑两 pass 并 submit；partial 只需 voices × 段长（32MB 级），
//  voice 状态经 voice_states 在段间传递，CPU↔GPU 往返仍为一次。
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
    seg_count: u32,      // 段内段数（段边界 = CC 事件位置）
    release_count: u32,  // release/kill 指令总数
    env_update_count: u32, // CC72/73/121 包络更新指令总数
    // partial 缓冲的每 voice 帧 stride（= 段长上界；末日段短于该值时也用它）
    partial_stride: u32,
    // 整块 channel_mix 的帧数（pass2 写入 stride；= 外层块的帧数）
    channel_mix_frames: u32,
    // 本渲染段的帧在整块 channel_mix 中的起始偏移（pass2 写入位置）
    mix_offset: u32,
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
    // 声像：音色库基础声像（通道音量/声像已迁至 yinhe-dsp 效果器）
    base_pan_l: f32,
    base_pan_r: f32,
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
/// 紧凑活跃状态：pass1 写 env_stage（CPU 只读回这一个数组做 voice 清理，
/// 不再读回全字段状态 —— 状态常驻 GPU）。
@group(0) @binding(15) var<storage, read_write> voice_stage: array<u32>;

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

/// 推进 1 帧 envelope（与 CPU 参考实现 cpu_ref::advance_env_cpu 逐帧等价）。
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
    // 段 0 的通道更新（段起点 CC 事件）在初始化时应用。
    // 只应用于**已开始**的 voice（start_offset == 0）：尚未开始的 voice
    // （段内稍后/后续段才发声，但已在渲染前上传）的创建值已是"其开始帧的
    // 通道状态"，套用段 0 的旧 mult 会在 speed 修正里引入一次性 time 偏差。
    // 段起点（fi=0）换速需补偿：上一段的末尾推进用的是旧速度外推一帧，
    // 而逐帧语义下本帧位置 = 上一帧位置 + 新速度 → time += 新 - 旧。
    if is_active && params.seg_count > 0u && st.start_offset == 0u {
        let off = segs[0].ch_off;
        let cnt = segs[0].ch_count;
        for (var ui: u32 = off; ui < off + cnt; ui++) {
            let cu = ch_updates[ui];
            if cu.ch == st.channel {
                let old_speed = st.speed;
                st.speed = st.base_speed * cu.speed_mult;
                st.time += st.speed - old_speed;
            }
        }
    }
    var seg_idx = 0u;

    for (var fi: u32 = 0u; fi < fc; fi++) {
        // 跨段：应用该段边界的通道状态更新
        while seg_idx + 1u < params.seg_count && fi >= segs[seg_idx + 1u].start_frame {
            seg_idx += 1u;
            // 只对已开始的 voice 应用通道更新（同初始化：未开始的 voice
            // 创建值已是其开始帧的通道状态）。
            if is_active && fi >= st.start_offset {
                let off = segs[seg_idx].ch_off;
                let cnt = segs[seg_idx].ch_count;
                for (var ui: u32 = off; ui < off + cnt; ui++) {
                    let cu = ch_updates[ui];
                    if cu.ch == st.channel {
                        let old_speed = st.speed;
                        st.speed = st.base_speed * cu.speed_mult;
                        // speed 跳变（弯音/调音）时修正 time，保持采样位置连续：
                        // 解析式 t = time + (fi - start_offset)*speed 在换速处产生
                        // 跳变，回退到"上一帧末 + 新速度"的连续位置。修正量
                        // = (fi - start_offset - 1) * (old - new)（fi == start_offset
                        // 即音符首帧时无前帧可比，修正量为 -Δ 保持解析式基准连续）。
                        let n = f32(fi - st.start_offset);
                        st.time += (n - 1.0) * (old_speed - st.speed);
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
                    if rc.mode == 6u {
                        // kill：1ms 淡出（xsynth ReleaseType::Kill；硬切有 click）
                        st.release_frames = 0.001 * f32(params.sample_rate);
                        st.env_stage = 5u;
                    } else {
                        st.env_stage = rc.mode;
                    }
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
            // 通道音量/声像（CC7/10/11）已迁至 yinhe-dsp 效果器，
            // 此处只用音色库基础增益/声像。
            let ch_gain = st.base_gain;
            let ch_pan_l = st.base_pan_l;
            let ch_pan_r = st.base_pan_r;

            // 采样位置（帧索引；立体声样本交错存储，位置 = 帧 * 2）
            let t = st.time + f32(fi - st.start_offset) * st.speed;
            var idx = u32(t);
            let frac = t - f32(idx);
            let max_idx = st.sample_length - 1u;

            // 循环处理（与 xsynth 一致）——loop_mode：0=NoLoop, 1=LoopContinuous,
            // 2=LoopSustain, 3=OneShot：
            // - Continuous：恒循环；Sustain：仅未 release（env_stage < 5）时循环，
            //   release 后从当前位置继续播到尾；NoLoop/OneShot：不循环，播完即结束
            // - 回绕公式与 xsynth 一致：idx > loop_end 才回绕（loop 区间含 end），
            //   (idx - end - 1) % len + start（不能 >=，否则差一个采样位置）
            let released = st.env_stage >= 5u;
            let loop_cont = st.loop_mode == 1u;
            let loop_sus = st.loop_mode == 2u && !released;
            let has_loop = (loop_cont || loop_sus) && st.loop_end > st.loop_start;
            if has_loop && idx > st.loop_end {
                let loop_len = st.loop_end - st.loop_start;
                if loop_len > 0u {
                    idx = (idx - st.loop_end - 1u) % loop_len + st.loop_start;
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
            st = advance_env(st);
        }

        // 直写自己的 slot（pass2 按通道归约；无 workgroup 同步）。
        // stride 用 partial_stride（段长上界）而非 fc：分段渲染时各段共用
        // 同一 partial 区域，保证索引不串位。
        partial[vid * params.partial_stride * 2u + fi * 2u] = my_l;
        partial[vid * params.partial_stride * 2u + fi * 2u + 1u] = my_r;
    }

    // 全字段写回（CPU 读回为下一块起点状态；flt_* 亦在其中）。
    // 与 CPU 参考实现 cpu_ref 一致：消耗一次性 start_offset、推进 time。
    // 分段渲染：voice 的 start_offset 为**段内相对**偏移（跨段时由 else 分支
    // 右移 fc）；音符在后续段才开始时不能推进 time、也不能清零（否则下段
    // 会被误判为"已开始"而提前发声）。
    if is_active {
        if fc > st.start_offset {
            let act_frames = fc - st.start_offset;
            st.start_offset = 0u;
            if st.env_stage < 6u {
                st.time += st.speed * f32(act_frames);
                // 跨块推进后回绕（与 xsynth 同一点：> loop_end），避免 f32 time
                // 无限增长（2^24 采样 ≈ 6 分钟后丢精度，黑乐谱长曲必须回绕）。
                // LoopSustain release 后不回绕：从当前位置继续播到尾。
                let looped = (st.loop_mode == 1u
                    || (st.loop_mode == 2u && st.env_stage < 5u))
                    && st.loop_end > st.loop_start;
                if looped && st.time > f32(st.loop_end) {
                    let loop_len = f32(st.loop_end - st.loop_start);
                    // 回绕到回绕区 [end+1, end+len]（不落回恒等区 [start, end]）：
                    // xsynth 的原始位置永不回绕，回绕环 = len 样本（不含 end，恒等区
                    // 只在第一遍出现）；若回绕进恒等区，块边界后相位会漂移
                    // （多播 end 一次，循环周期变 len+1，长音符逐步失同步）。
                    let off = (st.time - f32(st.loop_end) - 1.0) % loop_len;
                    st.time = f32(st.loop_end) + 1.0 + off;
                }
            }
        } else {
            // 音符在后续渲染段才开始：start_offset（段内相对）右移到下一段。
            st.start_offset -= fc;
        }
        voice_states[vid] = st;
        // 紧凑状态：CPU 只读回 env_stage（voice 结束清理用）。
        voice_stage[vid] = st.env_stage;
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
            let base = vid * params.partial_stride * 2u + fi * 2u;
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
        // 写整块 channel_mix 的对应帧区间（分段渲染：mix_offset 为该段起始帧，
        // fc 为整块帧数而非段长）
        let base = (ch * params.channel_mix_frames + params.mix_offset + fi) * 2u;
        channel_mix[base] = shared_l[ch * 8u];
        channel_mix[base + 1u] = shared_r[ch * 8u];
    }
}
