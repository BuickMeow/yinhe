// 临时诊断：bend/CC 场景的 GPU vs CPU 波形对比
use std::path::PathBuf;
use std::sync::Arc;

use xsynth_core::channel::{ChannelAudioEvent, ChannelConfigEvent, ChannelEvent, ControlEvent};
use xsynth_core::channel_group::{
    ChannelGroup, ChannelGroupConfig, ParallelismOptions, SynthEvent, SynthFormat, ThreadCount,
};
use xsynth_core::soundfont::{SampleSoundfont, SoundfontBase, SoundfontInitOptions};
use xsynth_core::{AudioPipe, AudioStreamParams, ChannelCount};

const SR: u32 = 44100;

fn test_sfz() -> Option<PathBuf> {
    std::env::var("YINHE_TEST_SFZ").ok().map(PathBuf::from)
}

fn cpu_render(
    sf: Arc<dyn SoundfontBase>,
    events: &[(u64, SynthEvent)],
    total_frames: u64,
) -> Vec<f32> {
    let stream_params = AudioStreamParams {
        channels: ChannelCount::Stereo,
        sample_rate: SR,
    };
    let config = ChannelGroupConfig {
        channel_init_options: Default::default(),
        format: SynthFormat::Custom { channels: 1 },
        audio_params: stream_params,
        parallelism: ParallelismOptions {
            channel: ThreadCount::None,
            key: ThreadCount::None,
        },
    };
    let mut cg = ChannelGroup::new(config);
    cg.send_event(SynthEvent::Channel(
        0,
        ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(vec![sf])),
    ));
    let mut out = Vec::new();
    let mut chunk = vec![0.0f32; 512 * 2];
    let mut cursor = 0usize;
    let mut rendered = 0u64;
    while rendered < total_frames {
        while cursor < events.len() && events[cursor].0 <= rendered {
            cg.send_event(events[cursor].1.clone());
            cursor += 1;
        }
        let seg_end = events
            .get(cursor)
            .map(|(s, _)| (*s).min(total_frames))
            .unwrap_or(total_frames);
        let frames = (seg_end - rendered) as usize;
        let mut done = 0usize;
        while done < frames {
            let n = (frames - done).min(512);
            let buf = &mut chunk[..n * 2];
            cg.read_samples(buf);
            out.extend_from_slice(buf);
            done += n;
        }
        rendered = seg_end;
    }
    out
}

fn main() {
    let Some(sfz) = test_sfz() else { return };
    let stream_params = AudioStreamParams {
        channels: ChannelCount::Stereo,
        sample_rate: SR,
    };
    let sf = Arc::new(
        SampleSoundfont::new_sfz(sfz.clone(), stream_params, SoundfontInitOptions::default())
            .unwrap(),
    ) as Arc<dyn SoundfontBase>;
    let mut synth = yinhe_synth::GpuSynth::new_default(&sfz, SR).unwrap();
    synth.set_limiter_enabled(false);

    // 场景：parity 完整音符计划 + 完整 CC 计划
    let s = |ms: u64| ms * SR as u64 / 1000;
    let plan: Vec<(u64, u8, u8, u64)> = vec![
        (50, 60, 90, 800),
        (80, 64, 100, 800),
        (110, 67, 110, 800),
        (1000, 62, 60, 300),
        (1400, 65, 110, 250),
        (1800, 69, 45, 350),
        (2200, 67, 80, 200),
        (2600, 72, 120, 400),
        (2400, 48, 100, 600),
        (2400, 84, 70, 600),
        (3200, 55, 55, 500),
    ];
    let mut events: Vec<(u64, SynthEvent)> = Vec::new();
    for (start, key, vel, dur) in &plan {
        events.push((
            s(*start),
            SynthEvent::Channel(
                0,
                ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                    key: *key,
                    vel: *vel,
                }),
            ),
        ));
        events.push((
            s(start + dur),
            SynthEvent::Channel(
                0,
                ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: *key }),
            ),
        ));
    }
    for (ms, controller, value) in [
        (1000u64, 7u8, 100u8),
        (1500, 7, 60),
        (2600, 64, 127),
        (2800, 64, 0),
    ] {
        events.push((
            s(ms),
            SynthEvent::Channel(
                0,
                ChannelEvent::Audio(ChannelAudioEvent::Control(ControlEvent::Raw(
                    controller, value,
                ))),
            ),
        ));
    }
    events.push((
        s(3000),
        SynthEvent::Channel(
            0,
            ChannelEvent::Audio(ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
                0.5,
            ))),
        ),
    ));
    events.sort_by_key(|(t, _)| *t);
    let total = s(3700) + SR as u64 * 400 / 1000;

    let cpu = cpu_render(sf.clone(), &events, total);

    let mut gpu_events: Vec<yinhe_synth::SynthEvent> = Vec::new();
    for (start, key, vel, dur) in &plan {
        gpu_events.push(yinhe_synth::SynthEvent::NoteOn {
            sample: s(*start),
            channel: 0,
            key: *key,
            velocity: *vel,
        });
        gpu_events.push(yinhe_synth::SynthEvent::NoteOff {
            sample: s(start + dur),
            channel: 0,
            key: *key,
        });
    }
    for (ms, controller, value) in [
        (1000u64, 7u8, 100u8),
        (1500, 7, 60),
        (2600, 64, 127),
        (2800, 64, 0),
    ] {
        gpu_events.push(yinhe_synth::SynthEvent::Control {
            sample: s(ms),
            channel: 0,
            event: yinhe_synth::ControlEvent::Raw(controller, value),
        });
    }
    gpu_events.push(yinhe_synth::SynthEvent::Control {
        sample: s(3000),
        channel: 0,
        event: yinhe_synth::ControlEvent::PitchBend(0.5),
    });
    gpu_events.sort_by_key(|e| e.sample());
    synth.load_events(gpu_events);
    let mut gpu = Vec::new();
    let mut chunk = vec![0.0f32; 512 * 2];
    while synth.sample_position() < total {
        let n = ((total - synth.sample_position()) as usize).min(512);
        let buf = &mut chunk[..n * 2];
        synth.render(buf);
        gpu.extend_from_slice(buf);
    }

    // 对比渐变区域（CC7 1000ms/1500ms、damper 2600/2800ms、bend 3000ms）
    for ms in 990..1030 {
        let i = (ms as usize - 100) * SR as usize / 1000 * 2;
        eprintln!("t={ms}ms: cpu={:.6} gpu={:.6}", cpu[i], gpu[i]);
    }
    for ms in 1490..1530 {
        let i = (ms as usize - 100) * SR as usize / 1000 * 2;
        eprintln!("t={ms}ms: cpu={:.6} gpu={:.6}", cpu[i], gpu[i]);
    }
    for ms in 2790..3020 {
        let i = (ms as usize - 100) * SR as usize / 1000 * 2;
        eprintln!("t={ms}ms: cpu={:.6} gpu={:.6}", cpu[i], gpu[i]);
    }
    // RMS 分段
    for seg in 0..5 {
        let st = (seg * 600) as usize * SR as usize / 1000 * 2;
        let en = ((seg + 1) * 600) as usize * SR as usize / 1000 * 2;
        let rms_c = (cpu[st..en].iter().map(|x| (*x as f64).powi(2)).sum::<f64>()
            / (en - st) as f64)
            .sqrt();
        let rms_g = (gpu[st..en].iter().map(|x| (*x as f64).powi(2)).sum::<f64>()
            / (en - st) as f64)
            .sqrt();
        eprintln!(
            "rms[{}-{}ms]: cpu={rms_c:.5} gpu={rms_g:.5}",
            seg * 600,
            (seg + 1) * 600
        );
    }
}
