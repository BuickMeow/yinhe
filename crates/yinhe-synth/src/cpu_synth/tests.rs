//! CpuSynth 单元测试（原 cpu_synth.rs 内测试模块；文件过长拆出）。
//!
//! 本文件即 `cpu_synth::tests` 模块体（`#[cfg(test)] mod tests;` 引入）。

use super::*;

fn buffers(frames: usize) -> Vec<ChannelBuffers> {
    (0..2)
        .map(|_| ChannelBuffers {
            left: vec![0.0; frames],
            right: vec![0.0; frames],
        })
        .collect()
}

/// 无音色库时 note_on 不 panic、输出静音（select 落空静默）。
#[test]
fn note_on_without_soundfont_is_silent() {
    let mut synth = CpuSynth::new(48_000);
    synth.load_events(vec![SynthEvent::NoteOn {
        sample: 0,
        channel: 0,
        key: 60,
        velocity: 100,
        end_sample: 4800,
    }]);
    let mut bufs = buffers(512);
    synth.render_to_mixer(&mut bufs);
    assert_eq!(synth.voice_count(), 0);
    assert!(bufs.iter().all(|b| b.left.iter().all(|&v| v == 0.0)));
}

/// 事件在正确帧生效：NoteOn 在块中间（sample 256）时前半块静音。
#[test]
fn note_on_starts_at_event_frame() {
    let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
        return; // 无测试音色库时跳过（CI）
    };
    let mut synth = CpuSynth::new(48_000);
    synth
        .load_dense_soundfonts(0, &[PathBuf::from(sfz)])
        .expect("load soundfont");
    synth.load_events(vec![SynthEvent::NoteOn {
        sample: 256,
        channel: 0,
        key: 60,
        velocity: 100,
        end_sample: 48_000,
    }]);
    let mut bufs = buffers(512);
    synth.render_to_mixer(&mut bufs);
    let head_energy: f32 = bufs[0].left[..256].iter().map(|v| v.abs()).sum();
    let tail_energy: f32 = bufs[0].left[256..].iter().map(|v| v.abs()).sum();
    assert_eq!(head_energy, 0.0, "起始帧前不得发声");
    assert!(tail_energy > 0.0, "起始帧后应有输出");
}
/// 重叠音符精确释放（回归：显式 NoteOff 的 FIFO 错位——短音符的结束会
/// 释放长音符——是「音符被截断」的根因；NoteOn 自带 end_sample 后消除）。
#[test]
fn overlapping_notes_end_precisely() {
    let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
        return;
    };
    let mut synth = CpuSynth::new(48_000);
    synth
        .load_dense_soundfonts(0, &[PathBuf::from(sfz)])
        .expect("load soundfont");
    synth.load_events(vec![
        SynthEvent::NoteOn {
            sample: 0,
            channel: 0,
            key: 60,
            velocity: 100,
            end_sample: 48_000,
        },
        SynthEvent::NoteOn {
            sample: 0,
            channel: 0,
            key: 60,
            velocity: 60,
            end_sample: 4_800,
        },
    ]);
    let mut bufs = buffers(4_800);
    synth.render_to_mixer(&mut bufs);
    let long = synth
        .voices
        .iter()
        .find(|v| v.velocity == 100)
        .expect("长音符应在响");
    assert!(!long.released, "短音符结束不得释放长音符（FIFO 错位回归）");
    let short = synth.voices.iter().find(|v| v.velocity == 60);
    assert!(
        short.is_none_or(|v| v.released || v.finished()),
        "短音符到期应已释放"
    );
}

/// 完全重复合批（黑乐谱重复 NoteOn）：同参数 NoteOn 合成一个 voice，
/// 输出 = 单个 × 2（线性等价）；不同参数不得误合并。
#[test]
fn duplicate_note_on_batches_into_one_voice() {
    let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
        return;
    };
    let load = || {
        let mut synth = CpuSynth::new(48_000);
        synth
            .load_dense_soundfonts(0, &[PathBuf::from(&sfz)])
            .expect("load soundfont");
        synth
    };
    let on = |velocity: u8, end_sample: u64| SynthEvent::NoteOn {
        sample: 0,
        channel: 0,
        key: 60,
        velocity,
        end_sample,
    };

    let mut single = load();
    single.load_events(vec![on(100, 48_000)]);
    let mut bufs1 = buffers(512);
    single.render_to_mixer(&mut bufs1);

    let mut dup = load();
    dup.load_events(vec![on(100, 48_000), on(100, 48_000)]);
    let mut bufs2 = buffers(512);
    dup.render_to_mixer(&mut bufs2);
    assert_eq!(dup.voices.len(), 1, "完全重复 NoteOn 应合批为一个 voice");
    assert_eq!(dup.voices[0].dup, 2);
    for i in 0..512 {
        assert!(
            (bufs2[0].left[i] - 2.0 * bufs1[0].left[i]).abs() < 1e-4,
            "合批输出应等于 2 倍单音（样本 {i}）"
        );
    }

    let mut diff = load();
    diff.load_events(vec![on(100, 48_000), on(100, 24_000), on(60, 48_000)]);
    let mut bufs3 = buffers(512);
    diff.render_to_mixer(&mut bufs3);
    assert_eq!(diff.voices.len(), 3, "参数不同不得合并");
}

/// 合批 voice 的 NoteOff 引用消耗：逐个递减，归 1 才释放。
#[test]
fn batched_voice_needs_all_note_offs_to_release() {
    let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
        return;
    };
    let load = || {
        let mut synth = CpuSynth::new(48_000);
        synth
            .load_dense_soundfonts(0, &[PathBuf::from(&sfz)])
            .expect("load soundfont");
        synth
    };
    let on = SynthEvent::NoteOn {
        sample: 0,
        channel: 0,
        key: 60,
        velocity: 100,
        end_sample: 192_000,
    };
    let off = |sample: u64| SynthEvent::NoteOff {
        sample,
        channel: 0,
        key: 60,
    };

    let mut one_off = load();
    one_off.load_events(vec![on, on, off(256)]);
    let mut bufs1 = buffers(512);
    one_off.render_to_mixer(&mut bufs1);
    assert_eq!(one_off.voices.len(), 1);
    assert_eq!(one_off.voices[0].dup, 1, "第一个 NoteOff 只消耗引用");
    assert!(!one_off.voices[0].released, "尚有一个引用，不得释放");

    let mut two_off = load();
    two_off.load_events(vec![on, off(256), off(512)]);
    let mut bufs2 = buffers(1024);
    two_off.render_to_mixer(&mut bufs2);
    assert_eq!(two_off.voices[0].dup, 1);
    assert!(two_off.voices[0].released, "最后一个 NoteOff 应释放");
}

/// layer 上限（对齐 xsynth）：同一 key 5 个递增力度音符 + layer=4 →
/// 活跃 voice 只 4 个（杀 velocity 最低的 20）。
#[test]
fn layer_limit_kills_quietest() {
    let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
        return;
    };
    let mut synth = CpuSynth::new(48_000);
    synth
        .load_dense_soundfonts(0, &[PathBuf::from(sfz)])
        .expect("load soundfont");
    synth.set_layer_count(Some(4));
    let mut events = Vec::new();
    for i in 0..5u8 {
        events.push(SynthEvent::NoteOn {
            sample: 0,
            channel: 0,
            key: 60,
            velocity: 20 + i * 20,
            end_sample: 48_000,
        });
    }
    synth.load_events(events);
    let mut bufs = buffers(512);
    synth.render_to_mixer(&mut bufs);
    assert_eq!(synth.voice_count(), 4, "layer=4 应限制同 key 活跃 voice 数");
    // 最弱的（20）被淘汰，剩下的都 >= 40
    assert!(synth.voice_count() == 4);
}
