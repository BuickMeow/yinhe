use super::*;

struct Doubler;
impl InsertProcessor for Doubler {
    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        left.iter_mut().for_each(|v| *v *= 2.0);
        right.iter_mut().for_each(|v| *v *= 2.0);
    }

    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

/// 记录 flush 调用的 insert（验证暂停参数 flush 覆盖所有链）。
struct FlushCounter(std::sync::Arc<std::sync::atomic::AtomicUsize>);
impl InsertProcessor for FlushCounter {
    fn process(&mut self, _left: &mut [f32], _right: &mut [f32]) {}

    fn flush_pending_params(&mut self, _position_samples: u64) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

#[test]
fn flush_pending_insert_params_reaches_all_chains() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let count = std::sync::Arc::new(AtomicUsize::new(0));
    let mut g = graph_with(&[StripParams::default(), StripParams::default()], 4);
    g.insert_insert(0, 0, Box::new(FlushCounter(count.clone())));
    g.insert_insert(1, 0, Box::new(FlushCounter(count.clone())));
    g.insert_master_insert(0, Box::new(FlushCounter(count.clone())));
    g.flush_pending_insert_params(123);
    assert_eq!(count.load(Ordering::Relaxed), 3);
}

/// 记录每次 process 调用长度的 insert（`max` = 声明的块长能力）。
struct SegmentRecorder {
    max: usize,
    lens: std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
}
impl InsertProcessor for SegmentRecorder {
    fn max_block_frames(&self) -> usize {
        self.max
    }

    fn process(&mut self, left: &mut [f32], _right: &mut [f32]) {
        self.lens
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(left.len());
    }

    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

/// 回归：插件能力（max_block_frames）小于引擎块长时按能力分段调用
/// （512 激活的插件遇到 GPU 4096 块曾只处理前 512 帧）。
#[test]
fn insert_capability_smaller_than_block_is_segmented() {
    let lens = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut g = graph_with(&[StripParams::default()], 10);
    g.insert_insert(
        0,
        0,
        Box::new(SegmentRecorder {
            max: 4,
            lens: lens.clone(),
        }),
    );
    g.process();
    assert_eq!(&*lens.lock().unwrap_or_else(|e| e.into_inner()), &[4, 4, 2]);
}

/// 无能力限制（默认 usize::MAX）时整块单次调用。
#[test]
fn insert_without_capability_is_single_call() {
    let lens = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut g = graph_with(&[StripParams::default()], 10);
    g.insert_insert(
        0,
        0,
        Box::new(SegmentRecorder {
            max: usize::MAX,
            lens: lens.clone(),
        }),
    );
    g.process();
    assert_eq!(&*lens.lock().unwrap_or_else(|e| e.into_inner()), &[10]);
}

fn graph_with(channels: &[StripParams], frames: usize) -> MixerGraph {
    let mut g = MixerGraph::new(frames);
    g.resize(channels.len(), frames, channels);
    g
}

fn fill(g: &mut MixerGraph, channel: usize, value: f32) {
    let b = g.channel_buffers_mut(channel).unwrap();
    b.left.iter_mut().for_each(|v| *v = value);
    b.right.iter_mut().for_each(|v| *v = value);
}

/// 通道发送（前向）：源通道 send 到序号更大的通道，同块到达。
#[test]
fn channel_send_forward_is_same_block() {
    let mut g = graph_with(&[StripParams::default(), StripParams::default()], 4);
    g.set_channel_sends(
        0,
        vec![ResolvedChannelSend {
            target: ChannelSendTarget::Channel(1),
            amount: 1.0,
            pre_fader: false,
        }],
    );
    fill(&mut g, 0, 0.5);
    let (l, _r) = g.process();
    let single = core::f32::consts::FRAC_1_SQRT_2;
    // 直达 + 经目标通道推子/声像的 send。
    let expect = 0.5 * single + 0.5 * single * single;
    assert!((l[3] - expect).abs() < 1e-6, "got {} expect {expect}", l[3]);
}

/// 通道发送（逆序）：目标在源之前处理，用上一块快照，延迟一块。
#[test]
fn channel_send_reverse_is_one_block_delayed() {
    let mut g = graph_with(&[StripParams::default(), StripParams::default()], 4);
    g.set_channel_sends(
        1,
        vec![ResolvedChannelSend {
            target: ChannelSendTarget::Channel(0),
            amount: 1.0,
            pre_fader: false,
        }],
    );
    fill(&mut g, 1, 0.5);
    let (l, _r) = g.process();
    let single = core::f32::consts::FRAC_1_SQRT_2;
    assert!(
        (l[3] - 0.5 * single).abs() < 1e-6,
        "第一块不应有逆序发送: {}",
        l[3]
    );
    fill(&mut g, 1, 0.5);
    let (l, _r) = g.process();
    let expect = 0.5 * single + 0.5 * single * single;
    assert!((l[3] - expect).abs() < 1e-6, "got {} expect {expect}", l[3]);
}

/// 推子前发送不受源推子影响。
#[test]
fn channel_send_pre_fader_ignores_source_gain() {
    let mut g = graph_with(
        &[
            StripParams {
                gain: 0.0,
                ..StripParams::default()
            },
            StripParams::default(),
        ],
        4,
    );
    g.set_channel_sends(
        0,
        vec![ResolvedChannelSend {
            target: ChannelSendTarget::Channel(1),
            amount: 1.0,
            pre_fader: true,
        }],
    );
    fill(&mut g, 0, 0.5);
    let (l, _r) = g.process();
    let expect = 0.5 * core::f32::consts::FRAC_1_SQRT_2;
    assert!((l[3] - expect).abs() < 1e-6, "got {}", l[3]);
}

/// 发送到 Master：直接累加，不经过其它通道。
#[test]
fn channel_send_to_master_bypasses_channels() {
    let mut g = graph_with(&[StripParams::default()], 4);
    g.set_channel_sends(
        0,
        vec![ResolvedChannelSend {
            target: ChannelSendTarget::Master,
            amount: 1.0,
            pre_fader: false,
        }],
    );
    fill(&mut g, 0, 0.5);
    let (l, _r) = g.process();
    let single = core::f32::consts::FRAC_1_SQRT_2;
    // 直达 + send 各一份。
    let expect = 0.5 * single * 2.0;
    assert!((l[3] - expect).abs() < 1e-6, "got {} expect {expect}", l[3]);
}

/// 通道主输出指向另一条通道：不再直达 Master。
#[test]
fn channel_main_output_replaces_master_path() {
    let mut g = graph_with(&[StripParams::default(), StripParams::default()], 4);
    g.set_channel_output(0, Some(1));
    fill(&mut g, 0, 0.5);
    let (l, _r) = g.process();
    // 通道 0 推子/声像 → 通道 1 输入 → 通道 1 推子/声像 → Master。
    let single = 0.5 * core::f32::consts::FRAC_1_SQRT_2;
    let expect = single * core::f32::consts::FRAC_1_SQRT_2;
    assert!((l[3] - expect).abs() < 1e-6, "got {} expect {expect}", l[3]);
}

#[test]
fn single_channel_unity_gain_passthrough() {
    let mut g = graph_with(&[StripParams::default()], 4);
    fill(&mut g, 0, 0.5);
    let (l, _r) = g.process();
    // 居中声像等功率：0.5 * 1.0 * √0.5，左右相同。
    assert!((l[3] - 0.5 * core::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
}

#[test]
fn mute_silences_channel() {
    let mut g = graph_with(
        &[
            StripParams {
                mute: true,
                ..StripParams::default()
            },
            StripParams::default(),
        ],
        4,
    );
    fill(&mut g, 0, 1.0);
    fill(&mut g, 1, 0.5);
    let (l, _r) = g.process();
    let expect = 0.5 * core::f32::consts::FRAC_1_SQRT_2;
    assert!((l[3] - expect).abs() < 1e-6);
}

#[test]
fn solo_excludes_other_channels() {
    let mut g = graph_with(
        &[
            StripParams {
                solo: true,
                ..StripParams::default()
            },
            StripParams::default(),
        ],
        4,
    );
    fill(&mut g, 0, 0.25);
    fill(&mut g, 1, 1.0);
    let (l, _r) = g.process();
    let expect = 0.25 * core::f32::consts::FRAC_1_SQRT_2;
    assert!((l[3] - expect).abs() < 1e-6);
}

#[test]
fn mute_wins_over_solo() {
    let mut g = graph_with(
        &[StripParams {
            solo: true,
            mute: true,
            ..StripParams::default()
        }],
        4,
    );
    fill(&mut g, 0, 1.0);
    let (l, _r) = g.process();
    assert!(l.iter().all(|&v| v == 0.0));
}

#[test]
fn insert_runs_before_fader() {
    let mut g = graph_with(&[StripParams::default()], 4);
    fill(&mut g, 0, 0.25);
    g.set_inserts(0, vec![Box::new(Doubler)]);
    let (l, _r) = g.process();
    let expect = 0.5 * core::f32::consts::FRAC_1_SQRT_2;
    assert!((l[3] - expect).abs() < 1e-6);
}

/// 空闲通道跳过：全零通道被跳过时输出与完整处理逐样本一致。
#[test]
fn idle_channel_skip_matches_full_process() {
    let params = [StripParams::default(), StripParams::default()];
    let (full_l, full_r) = {
        let mut g = graph_with(&params, 4);
        fill(&mut g, 0, 0.5);
        let (l, r) = g.process();
        (l.to_vec(), r.to_vec())
    };
    let mut g = graph_with(&params, 4);
    fill(&mut g, 0, 0.5);
    // 通道 1 缓冲全零 → 跳过。
    let (l, r) = g.process_with_active(Some(&[true, false]));
    assert_eq!(l, &full_l[..]);
    assert_eq!(r, &full_r[..]);
}

/// 带 insert 的空闲通道不能跳过：insert 的延迟/混响尾音可能还在响。
#[test]
fn idle_channel_with_insert_is_still_processed() {
    let mut g = graph_with(&[StripParams::default(), StripParams::default()], 4);
    fill(&mut g, 1, 0.25);
    g.set_inserts(1, vec![Box::new(Doubler)]);
    let (l, _r) = g.process_with_active(Some(&[true, false]));
    let expect = 0.5 * core::f32::consts::FRAC_1_SQRT_2;
    assert!(
        (l[3] - expect).abs() < 1e-6,
        "带 insert 的空闲通道必须照常处理"
    );
}

#[test]
fn master_gain_applies() {
    let mut g = graph_with(&[StripParams::default()], 4);
    fill(&mut g, 0, 1.0);
    g.set_master(MasterParams { gain: 0.5 });
    let (l, _r) = g.process();
    let expect = core::f32::consts::FRAC_1_SQRT_2 * 0.5;
    assert!((l[3] - expect).abs() < 1e-6);
}

#[test]
fn resize_keeps_existing_strip_state() {
    let mut g = graph_with(&[StripParams::default()], 4);
    g.set_strip(
        0,
        StripParams {
            gain: 0.3,
            ..StripParams::default()
        },
    );
    g.resize(2, 4, &[]);
    assert_eq!(g.channel_count(), 2);
    // 0 号通道增益状态保留。
    assert_eq!(g.channels[0].strip.params.gain, 0.3);
}

#[test]
fn resize_reallocates_buffers_on_frame_change() {
    let mut g = graph_with(&[StripParams::default()], 4);
    g.resize(1, 8, &[]);
    assert_eq!(g.frames(), 8);
    assert_eq!(g.channels[0].buffers.left.len(), 8);
}

/// 报告延迟 N 且实际延迟 N 个样本的 insert（PDC 测试用）。
struct DelayInsert {
    line: DelayLine,
    latency: u32,
}

impl DelayInsert {
    fn new(latency: u32) -> Self {
        let mut line = DelayLine::new();
        line.set_delay(latency as usize);
        Self { line, latency }
    }
}

impl InsertProcessor for DelayInsert {
    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.line.process(left, right);
    }

    fn latency_samples(&self) -> u32 {
        self.latency
    }

    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

#[test]
fn delay_line_delays_by_n_samples() {
    let mut d = DelayLine::new();
    d.set_delay(2);
    assert_eq!(d.delay(), 2);
    let mut l = [1.0f32, 2.0, 3.0, 4.0];
    let mut r = l;
    d.process(&mut l, &mut r);
    // 前两块是延迟线初始静音，随后是延迟 2 的输入。
    assert_eq!(l, [0.0, 0.0, 1.0, 2.0]);
    assert_eq!(r, [0.0, 0.0, 1.0, 2.0]);
    // 下一块继续吐出剩余的 3、4。
    let mut l2 = [5.0f32, 6.0];
    let mut r2 = l2;
    d.process(&mut l2, &mut r2);
    assert_eq!(l2, [3.0, 4.0]);
    // clear 后重新从静音开始。
    d.clear();
    let mut l3 = [7.0f32, 8.0];
    d.process(&mut l3, &mut [0.0, 0.0]);
    assert!(l3.iter().all(|&v| v == 0.0));
}

#[test]
fn delay_line_zero_delay_is_passthrough() {
    let mut d = DelayLine::new();
    let mut l = [1.0f32, 2.0];
    d.process(&mut l, &mut [0.0, 0.0]);
    assert_eq!(l, [1.0, 2.0]);
}

#[test]
fn pdc_aligns_channels_with_latency_insert() {
    // 通道 1 有 2 样本延迟的 insert；通道 0 无。
    // 两通道同时输入脉冲：PDC 应给通道 0 补 2 样本，使求和点对齐。
    let mut g = graph_with(&[StripParams::default(), StripParams::default()], 4);
    g.set_inserts(1, vec![Box::new(DelayInsert::new(2))]);
    fill(&mut g, 0, 0.0);
    fill(&mut g, 1, 0.0);
    {
        let b = g.channel_buffers_mut(0).unwrap();
        b.left[0] = 1.0;
        b.right[0] = 1.0;
    }
    {
        let b = g.channel_buffers_mut(1).unwrap();
        b.left[0] = 1.0;
        b.right[0] = 1.0;
    }
    // 第一块：两个脉冲都应出现在 sample 2（通道 1 由 insert 延迟，
    // 通道 0 由 PDC 延迟线延迟）。
    let (l, _r) = g.process();
    let single = core::f32::consts::FRAC_1_SQRT_2;
    assert!(l[0].abs() < 1e-6, "sample 0 应无输出（对齐后）");
    assert!(l[1].abs() < 1e-6, "sample 1 应无输出（对齐后）");
    assert!(
        (l[2] - 2.0 * single).abs() < 1e-6,
        "sample 2 应为两个通道脉冲之和: {}",
        l[2]
    );
    assert!(l[3].abs() < 1e-6);
}

#[test]
fn pdc_keeps_single_channel_alignment_when_no_latency() {
    // 没有延迟插入时延迟线全为 0（直通），输出与无 PDC 时一致。
    let mut g = graph_with(&[StripParams::default(), StripParams::default()], 4);
    assert!(g.channels.iter().all(|c| c.delay.delay() == 0));
    fill(&mut g, 0, 0.5);
    let (l, _r) = g.process();
    let expect = 0.5 * core::f32::consts::FRAC_1_SQRT_2;
    assert!((l[0] - expect).abs() < 1e-6);
}

#[test]
fn pdc_aligns_channel_send_with_target_own_source() {
    let mut g = graph_with(&[StripParams::default(), StripParams::default()], 4);
    g.set_inserts(0, vec![Box::new(DelayInsert::new(2))]);
    g.set_channel_sends(
        0,
        vec![ResolvedChannelSend {
            target: ChannelSendTarget::Channel(1),
            amount: 1.0,
            pre_fader: false,
        }],
    );
    assert_eq!(g.channels[1].input_delay.delay(), 2);
    assert_eq!(g.channels[0].channel_send_delays[0].delay(), 0);
    for ch in [0, 1] {
        let b = g.channel_buffers_mut(ch).unwrap();
        b.left[0] = 1.0;
        b.right[0] = 1.0;
    }
    let (l, _r) = g.process();
    let single = core::f32::consts::FRAC_1_SQRT_2;
    assert!(l[0].abs() < 1e-6, "目标自身源应与汇入 send 对齐: {}", l[0]);
    assert!(
        (l[2] - (2.0 * single + single * single)).abs() < 1e-6,
        "sample 2 应为目标输出 + 源直达之和: {}",
        l[2]
    );
}

#[test]
fn pdc_aligns_channel_main_output_with_target_own_source() {
    let mut g = graph_with(&[StripParams::default(), StripParams::default()], 4);
    g.set_inserts(0, vec![Box::new(DelayInsert::new(2))]);
    g.set_channel_output(0, Some(1));
    assert_eq!(g.channels[1].input_delay.delay(), 2);
    assert_eq!(g.channels[0].output_delay.delay(), 0);
    for ch in [0, 1] {
        let b = g.channel_buffers_mut(ch).unwrap();
        b.left[0] = 1.0;
        b.right[0] = 1.0;
    }
    let (l, _r) = g.process();
    let single = core::f32::consts::FRAC_1_SQRT_2;
    assert!(l[0].abs() < 1e-6);
    assert!(
        (l[2] - ((1.0 + single) * single)).abs() < 1e-6,
        "got {}",
        l[2]
    );
}

#[test]
fn pdc_recomputes_on_insert_removal() {
    // 移除延迟 insert 后对齐回退为 0（延迟线长度归零）。
    let mut g = graph_with(&[StripParams::default(), StripParams::default()], 4);
    g.set_inserts(1, vec![Box::new(DelayInsert::new(3))]);
    assert_eq!(g.channels[0].master_delay.delay(), 3);
    g.set_inserts(1, Vec::new());
    assert!(g.channels.iter().all(|c| c.master_delay.delay() == 0));
}

#[test]
fn reset_clears_pdc_delay_lines() {
    let mut g = graph_with(&[StripParams::default(), StripParams::default()], 2);
    g.set_inserts(1, vec![Box::new(DelayInsert::new(2))]);
    {
        let b = g.channel_buffers_mut(0).unwrap();
        b.left[0] = 1.0;
    }
    let _ = g.process();
    g.reset_inserts();
    fill(&mut g, 0, 0.0);
    let (l, _r) = g.process();
    assert!(l.iter().all(|&v| v == 0.0), "reset 后延迟线应清空");
}

#[test]
fn channel_main_output_to_channel_replaces_master() {
    let mut g = graph_with(&[StripParams::default(), StripParams::default()], 4);
    g.set_channel_output(0, Some(1));
    fill(&mut g, 0, 0.5);
    let (l, _r) = g.process();
    let single = core::f32::consts::FRAC_1_SQRT_2;
    let expect = 0.5 * single * single;
    assert!((l[3] - expect).abs() < 1e-6, "got {}", l[3]);
}

/// 逆序通道主输出用上一块快照，延迟一块。
#[test]
fn channel_main_output_reverse_is_one_block_delayed() {
    let mut g = graph_with(&[StripParams::default(), StripParams::default()], 4);
    g.set_channel_output(1, Some(0));
    fill(&mut g, 1, 0.5);
    let (l, _r) = g.process();
    assert!(l[3].abs() < 1e-6, "第一块不应有逆序主输出: {}", l[3]);
    fill(&mut g, 1, 0.5);
    let (l, _r) = g.process();
    let single = core::f32::consts::FRAC_1_SQRT_2;
    let expect = 0.5 * single * single;
    assert!((l[3] - expect).abs() < 1e-6, "got {}", l[3]);
}

/// 自动化直接写通道增益：只改 gain，不动 mute/solo/pan。
#[test]
fn set_inserts_returns_old_chain() {
    let mut g = graph_with(&[StripParams::default()], 4);
    g.set_inserts(0, vec![Box::new(Doubler)]);
    let old = g.set_inserts(0, Vec::new());
    assert_eq!(old.len(), 1);
    fill(&mut g, 0, 0.25);
    let (l, _r) = g.process();
    // 新链为空：无倍增。
    let expect = 0.25 * core::f32::consts::FRAC_1_SQRT_2;
    assert!((l[3] - expect).abs() < 1e-6);
}

/// 回归：通道电平表必须取推子/声像之后，推子变化要立刻反映到表上。
#[test]
fn channel_meter_is_post_fader() {
    let mut g = graph_with(
        &[StripParams {
            gain: 0.5,
            ..StripParams::default()
        }],
        4,
    );
    fill(&mut g, 0, 1.0);
    let _ = g.process();
    let single = core::f32::consts::FRAC_1_SQRT_2;
    let (l, r) = g.channel_meter_reading(0).expect("meter").read();
    assert!((l - 0.5 * single).abs() < 1e-6, "推子后 L={l}");
    assert!((r - 0.5 * single).abs() < 1e-6, "推子后 R={r}");

    g.set_strip(
        0,
        StripParams {
            gain: 0.25,
            ..StripParams::default()
        },
    );
    fill(&mut g, 0, 1.0);
    let _ = g.process(); // 推子斜坡过渡块
    fill(&mut g, 0, 1.0);
    let _ = g.process();
    let (l, _) = g.channel_meter_reading(0).expect("meter").read();
    assert!((l - 0.25 * single).abs() < 1e-6, "推子变化后 L={l}");
}
