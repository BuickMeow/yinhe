//! SFZ 语义集成测试：用合成 wav 验证门控/释放触发/round-robin/one_shot。
//!
//! 这些语义此前被 `xsynth-soundfonts` 静默丢弃（`xfin_*`、`trigger`、`seq_*`
//! 不在它的 opcode 表里），导致真实音色库的锤子层/踏板层恒定叠响、释放采样
//! 全丢。测试不依赖外部音色库，CI 可直接跑。

use std::path::{Path, PathBuf};

use yinhe_mixer::ChannelBuffers;
use yinhe_synth::ControlEvent;
use yinhe_synth::{CpuSynth, SynthEvent};

const SR: u32 = 48_000;

/// 写一个 1 秒 440Hz 的 wav（16-bit 单声道）。
fn write_tone(dir: &Path, name: &str, freq: f32) -> PathBuf {
    let path = dir.join(name);
    let len = SR as usize;
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SR,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&path, spec).unwrap();
    for i in 0..len {
        let s = (2.0 * std::f32::consts::PI * freq * i as f32 / SR as f32).sin() * 0.5;
        w.write_sample((s * i16::MAX as f32) as i16).unwrap();
    }
    w.finalize().unwrap();
    path
}

fn buffers(frames: usize) -> Vec<ChannelBuffers> {
    (0..1)
        .map(|_| ChannelBuffers {
            left: vec![0.0; frames],
            right: vec![0.0; frames],
        })
        .collect()
}

/// 渲染 n 块并返回 (voice 峰值, 总能量)。
fn render(synth: &mut CpuSynth, blocks: usize) -> (usize, f64) {
    let mut peak = 0usize;
    let mut energy = 0.0f64;
    let mut bufs = buffers(512);
    for _ in 0..blocks {
        synth.render_to_mixer(&mut bufs);
        peak = peak.max(synth.voice_count());
        for &v in &bufs[0].left {
            energy += (v as f64).abs();
        }
    }
    (peak, energy)
}

/// `xfin_loccN=0 xfin_hiccN=127`：CC 值决定该层增益。
#[test]
fn xfin_gate_scales_layer_with_cc() {
    let dir = tempfile::tempdir().unwrap();
    write_tone(dir.path(), "a.wav", 440.0);
    let sfz = dir.path().join("t.sfz");
    std::fs::write(
        &sfz,
        "<control>\nset_cc20=0\n<region>\nsample=a.wav key=60\nxfin_locc20=0 xfin_hicc20=127\n",
    )
    .unwrap();

    // CC20 = 0 → 该层增益 0（整层被门控掉，但仍应是"选中"的）
    let mut quiet = CpuSynth::new(SR);
    quiet
        .load_dense_soundfonts(0, std::slice::from_ref(&sfz))
        .unwrap();
    quiet.load_events(vec![SynthEvent::NoteOn {
        sample: 0,
        channel: 0,
        key: 60,
        velocity: 100,
        end_sample: SR as u64 / 2,
    }]);
    let (_, quiet_energy) = render(&mut quiet, 8);

    // CC20 = 127 → 满增益
    let mut loud = CpuSynth::new(SR);
    loud.load_dense_soundfonts(0, &[sfz]).unwrap();
    loud.load_events(vec![
        SynthEvent::Control {
            sample: 0,
            channel: 0,
            event: ControlEvent::Raw(20, 127.0),
        },
        SynthEvent::NoteOn {
            sample: 0,
            channel: 0,
            key: 60,
            velocity: 100,
            end_sample: SR as u64 / 2,
        },
    ]);
    let (_, loud_energy) = render(&mut loud, 8);

    assert!(
        quiet_energy < loud_energy * 0.05,
        "CC=0 时该层应几乎静音（quiet={quiet_energy:.3}, loud={loud_energy:.3}）"
    );
    assert!(loud_energy > 0.0, "CC=127 时应正常发声");
}

/// `<control> set_ccN=v` 必须在加载时写入通道，否则 xfin 门控会让整库静音。
#[test]
fn set_cc_defaults_enable_gated_layers() {
    let dir = tempfile::tempdir().unwrap();
    write_tone(dir.path(), "a.wav", 440.0);
    let sfz = dir.path().join("t.sfz");
    // 初始 CC 由库提供 = 127；若未应用默认值，CC 为 0 → 完全静音。
    std::fs::write(
        &sfz,
        "<control>\nset_cc20=127\n<region>\nsample=a.wav key=60\nxfin_locc20=0 xfin_hicc20=127\n",
    )
    .unwrap();

    let mut synth = CpuSynth::new(SR);
    synth.load_dense_soundfonts(0, &[sfz]).unwrap();
    synth.load_events(vec![SynthEvent::NoteOn {
        sample: 0,
        channel: 0,
        key: 60,
        velocity: 100,
        end_sample: SR as u64 / 2,
    }]);
    let (_, energy) = render(&mut synth, 8);
    assert!(
        energy > 1.0,
        "set_cc20=127 应让门控层发声（energy={energy:.3}）"
    );
}

/// `trigger=release`：主音结束后才有声（且 inaudible 于主音期间）。
#[test]
fn release_trigger_sounds_after_note_end() {
    let dir = tempfile::tempdir().unwrap();
    write_tone(dir.path(), "a.wav", 440.0);
    let sfz = dir.path().join("t.sfz");
    std::fs::write(
        &sfz,
        "<region>\nsample=a.wav key=60\n\
         <region>\nsample=a.wav key=60 volume=-3 trigger=release\n",
    )
    .unwrap();

    // 主音只持续 0.2s，之后渲染 0.5s：释放层应在后半段出现。
    let mut synth = CpuSynth::new(SR);
    synth.load_dense_soundfonts(0, &[sfz]).unwrap();
    synth.load_events(vec![SynthEvent::NoteOn {
        sample: 0,
        channel: 0,
        key: 60,
        velocity: 100,
        end_sample: SR as u64 / 5,
    }]);

    let mut before = 0.0f64;
    let mut after = 0.0f64;
    let mut bufs = buffers(512);
    for block in 0..48 {
        synth.render_to_mixer(&mut bufs);
        let e: f64 = bufs[0].left.iter().map(|v| (f64::from(*v)).abs()).sum();
        if block < 12 {
            before += e;
        } else {
            after += e;
        }
    }
    assert!(
        after > before * 0.02,
        "释放层应在主音结束后发声（before={before:.3}, after={after:.3}）"
    );
}

/// `loop_mode=one_shot`：忽略 note_off，播到采样末尾。
#[test]
fn one_shot_ignores_note_off() {
    let dir = tempfile::tempdir().unwrap();
    write_tone(dir.path(), "a.wav", 440.0);
    let sfz = dir.path().join("t.sfz");
    std::fs::write(
        &sfz,
        "<region>\nsample=a.wav key=60 loop_mode=one_shot ampeg_release=0.001\n",
    )
    .unwrap();

    let mut synth = CpuSynth::new(SR);
    synth.load_dense_soundfonts(0, &[sfz]).unwrap();
    synth.load_events(vec![
        SynthEvent::NoteOn {
            sample: 0,
            channel: 0,
            key: 60,
            velocity: 100,
            end_sample: SR as u64,
        },
        // 提前 NoteOff：one_shot 应忽略它
        SynthEvent::NoteOff {
            sample: 480,
            channel: 0,
            key: 60,
        },
    ]);
    let mut bufs = buffers(512);
    let mut late_energy = 0.0f64;
    for block in 0..16 {
        synth.render_to_mixer(&mut bufs);
        if block >= 8 {
            // 0.08s 之后：已过 NoteOff
            late_energy += bufs[0]
                .left
                .iter()
                .map(|v| (f64::from(*v)).abs())
                .sum::<f64>();
        }
    }
    assert!(late_energy > 1.0, "one_shot 应在 NoteOff 之后继续发声");
}

/// `off_by`：新音触发时掐掉同组旧音。
#[test]
fn off_by_cuts_previous_voice_in_same_group() {
    let dir = tempfile::tempdir().unwrap();
    write_tone(dir.path(), "a.wav", 440.0);
    let sfz = dir.path().join("t.sfz");
    std::fs::write(&sfz, "<region>\nsample=a.wav key=60 group=1 off_by=1\n").unwrap();

    let mut synth = CpuSynth::new(SR);
    synth.load_dense_soundfonts(0, &[sfz]).unwrap();
    synth.load_events(vec![
        SynthEvent::NoteOn {
            sample: 0,
            channel: 0,
            key: 60,
            velocity: 100,
            end_sample: SR as u64,
        },
        SynthEvent::NoteOn {
            sample: 4800,
            channel: 0,
            key: 60,
            velocity: 100,
            end_sample: SR as u64,
        },
    ]);
    let mut bufs = buffers(512);
    // 第一段：1 个 voice；到第二个 note_on 后旧音被杀，仍在淡出中（1ms）
    for _ in 0..10 {
        synth.render_to_mixer(&mut bufs);
    }
    let alive = synth.voice_count();
    assert!(
        alive <= 2,
        "off_by 应掐掉旧组（alive={alive}；若未实现会累积成 2 个长音）"
    );
}

/// `seq_length/seq_position`：第 N 个 note_on 才命中该层（round-robin）。
#[test]
fn seq_position_rotates_layers() {
    let dir = tempfile::tempdir().unwrap();
    write_tone(dir.path(), "a.wav", 440.0);
    let sfz = dir.path().join("t.sfz");
    std::fs::write(
        &sfz,
        "<region>\nsample=a.wav key=60 volume=0 seq_length=2 seq_position=1 ampeg_release=0.001\n\
         <region>\nsample=a.wav key=60 volume=-12 seq_length=2 seq_position=2 ampeg_release=0.001\n",
    )
    .unwrap();

    let mut synth = CpuSynth::new(SR);
    synth.load_dense_soundfonts(0, &[sfz]).unwrap();
    // 两次 note_on（间隔很长，前一音早已结束）
    synth.load_events(vec![
        SynthEvent::NoteOn {
            sample: 0,
            channel: 0,
            key: 60,
            velocity: 100,
            end_sample: 4800,
        },
        SynthEvent::NoteOn {
            sample: 48_000,
            channel: 0,
            key: 60,
            velocity: 100,
            end_sample: 52_800,
        },
    ]);
    let mut bufs = buffers(512);
    let mut energies = Vec::new();
    for _ in 0..110 {
        synth.render_to_mixer(&mut bufs);
        energies.push(
            bufs[0]
                .left
                .iter()
                .map(|v| (f64::from(*v)).abs())
                .sum::<f64>(),
        );
    }
    let first: f64 = energies[1..6].iter().sum();
    let second: f64 = energies[96..101].iter().sum();
    assert!(first > 0.0 && second > 0.0, "两次都应有声");
    assert!(
        first > second * 1.5,
        "第一轮应命中 0dB 层、第二轮 -12dB 层（first={first:.3}, second={second:.3}）"
    );
}

/// `pitch_random`：每次 note_on 的播放倍率不同（确定性抖动）。
#[test]
fn pitch_random_varies_speed_deterministically() {
    let dir = tempfile::tempdir().unwrap();
    write_tone(dir.path(), "a.wav", 440.0);
    let sfz = dir.path().join("t.sfz");
    std::fs::write(
        &sfz,
        "<region>\nsample=a.wav key=60 pitch_random=200 ampeg_release=0.001\n",
    )
    .unwrap();

    let mut a = CpuSynth::new(SR);
    a.load_dense_soundfonts(0, std::slice::from_ref(&sfz))
        .unwrap();
    let mut b = CpuSynth::new(SR);
    b.load_dense_soundfonts(0, &[sfz]).unwrap();
    for s in [&mut a, &mut b] {
        s.load_events(vec![SynthEvent::NoteOn {
            sample: 0,
            channel: 0,
            key: 60,
            velocity: 100,
            end_sample: 4800,
        }]);
    }
    let mut ba = buffers(512);
    let mut bb = buffers(512);
    a.render_to_mixer(&mut ba);
    b.render_to_mixer(&mut bb);
    // 同输入 → 完全一致的确定性抖动（离线导出 = 实时回放）
    assert_eq!(ba[0].left, bb[0].left, "pitch_random 必须确定性");

    // 且确实偏离未抖动的波形
    let plain = dir.path().join("plain.sfz");
    std::fs::write(
        &plain,
        "<region>\nsample=a.wav key=60 ampeg_release=0.001\n",
    )
    .unwrap();
    let mut c = CpuSynth::new(SR);
    c.load_dense_soundfonts(0, &[plain]).unwrap();
    c.load_events(vec![SynthEvent::NoteOn {
        sample: 0,
        channel: 0,
        key: 60,
        velocity: 100,
        end_sample: 4800,
    }]);
    let mut bc = buffers(512);
    c.render_to_mixer(&mut bc);
    assert_ne!(ba[0].left, bc[0].left, "pitch_random 应改变波形");
}

/// Cubic 插值：与 Linear 输出不同，且全部有限（无 NaN/爆音）。
#[test]
fn cubic_interpolation_is_finite_and_differs() {
    let dir = tempfile::tempdir().unwrap();
    write_tone(dir.path(), "a.wav", 440.0);
    let sfz = dir.path().join("t.sfz");
    // key=63 触发 +3 半音变调 → 非整数播放倍率（1.1892），插值才真正生效
    //（整八度是 2.0 倍，frac 恒为 0，两种插值结果相同）。
    std::fs::write(&sfz, "<region>\nsample=a.wav key=63 pitch_keycenter=60\n").unwrap();

    let render_with = |interp: u32| {
        let mut s = CpuSynth::new(SR);
        s.set_interpolation(interp);
        s.load_dense_soundfonts(0, std::slice::from_ref(&sfz))
            .unwrap();
        s.load_events(vec![SynthEvent::NoteOn {
            sample: 0,
            channel: 0,
            key: 63,
            velocity: 100,
            end_sample: SR as u64 / 2,
        }]);
        let mut bufs = buffers(512);
        for _ in 0..4 {
            s.render_to_mixer(&mut bufs);
        }
        bufs[0].left.clone()
    };

    let linear = render_with(1);
    let cubic = render_with(2);
    assert!(cubic.iter().all(|v| v.is_finite()), "Cubic 输出必须有限");
    assert!(cubic.iter().any(|v| *v != 0.0), "Cubic 输出不应静音");
    assert_ne!(linear, cubic, "Cubic 与 Linear 的波形应不同");
}

/// keyswitch：`sw_lokey..sw_hikey` 内的键只切换状态、不发声。
#[test]
fn keyswitch_key_is_swallowed() {
    let dir = tempfile::tempdir().unwrap();
    write_tone(dir.path(), "a.wav", 440.0);
    let sfz = dir.path().join("t.sfz");
    std::fs::write(
        &sfz,
        "<group>\nsw_lokey=24 sw_hikey=30 sw_last=26\n\
         <region>\nsample=a.wav key=60\n\
         <region>\nsample=a.wav key=26\n",
    )
    .unwrap();

    let mut synth = CpuSynth::new(SR);
    synth.load_dense_soundfonts(0, &[sfz]).unwrap();
    synth.load_events(vec![SynthEvent::NoteOn {
        sample: 0,
        channel: 0,
        key: 26,
        velocity: 100,
        end_sample: 4800,
    }]);
    let mut bufs = buffers(512);
    synth.render_to_mixer(&mut bufs);
    assert_eq!(synth.voice_count(), 0, "开关键不应发声");
    assert!(bufs[0].left.iter().all(|&v| v == 0.0));
}
