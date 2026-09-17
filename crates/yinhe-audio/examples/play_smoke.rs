//! 播放路径端到端冒烟（yinhe-synth CPU 后端）：spawn 完整音频系统 →
//! LoadModel + SetSoundFonts → Play → 轮询 sample_position 是否推进。
//!
//! 复现"切到 Yinhe CPU 后 Play 无响应"场景：若渲染线程 panic 或卡死，
//! `sample_position` 不会推进、`is_playing` 保持 false。
//!
//! 用法：
//! ```sh
//! cargo run --release -p yinhe-audio --example play_smoke --features gpu
//! YINHE_TEST_SFZ=/path/to.sfz cargo run ...   # 指定音色库（否则内置静音）
//! ```

use std::sync::Arc;

use yinhe_audio::{AudioCommand, SynthEngine, spawn_cpal_audio};
use yinhe_core::{ConductorData, NoteEvent, ProjectMeta, TrackData, YinModel};
use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, SegmentShape};

/// 4 拍、一个 C 大调和弦的模型（120 BPM / PPQ 480 → 2 秒）。
fn demo_model() -> Arc<YinModel> {
    let conductor = ConductorData {
        tempo: AutomationLane {
            target: AutomationTarget::Tempo,
            track: 0,
            events: vec![AutomationEvent {
                tick: 0,
                value: 120.0,
                shape: SegmentShape::Step,
            }],
        },
        time_sig: Vec::new(),
        key_sig: Vec::new(),
        markers: Vec::new(),
        lyrics: Vec::new(),
        chord: Vec::new(),
    };
    let mut model = YinModel {
        conductor: Arc::new(conductor),
        tracks: vec![Arc::new(TrackData::new(0, 0))],
        meta: ProjectMeta {
            ppq: 480,
            ..ProjectMeta::default()
        },
        ..Default::default()
    };
    model.load_track_notes(vec![vec![
        NoteEvent {
            start_tick: 0,
            end_tick: 1920,
            key: 60,
            velocity: 100,
            id: 0,
        },
        NoteEvent {
            start_tick: 0,
            end_tick: 1920,
            key: 64,
            velocity: 100,
            id: 1,
        },
        NoteEvent {
            start_tick: 0,
            end_tick: 1920,
            key: 67,
            velocity: 100,
            id: 2,
        },
    ]]);
    model.rebuild();
    Arc::new(model)
}

fn main() {
    let engine = match std::env::args().nth(1).as_deref() {
        Some("gpu") => SynthEngine::YinheGpu,
        Some("xsynth") => SynthEngine::XSynthCpu,
        _ => SynthEngine::YinheCpu,
    };
    println!("== play_smoke: 后端 {engine:?} ==");

    let model = demo_model();
    let layout = yinhe_audio::channels_for_model(&model);
    let handle = spawn_cpal_audio(48_000, layout, cpal::BufferSize::Default, None, engine)
        .expect("spawn 音频系统失败");

    handle.handle.send(AudioCommand::LoadModel {
        model: Arc::clone(&model),
    });
    if let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") {
        handle.handle.send(AudioCommand::SetSoundFonts {
            configs: Box::new(vec![(0u8, vec![sfz.to_string_lossy().into_owned()])]),
        });
    }
    // 等模型/音色库加载（首次音色库解析在 worker 里可达 4s；音频线程只查缓存）
    std::thread::sleep(std::time::Duration::from_secs(5));

    println!("-- Play --");
    handle.handle.send(AudioCommand::Play { from_sample: 0 });

    let t0 = std::time::Instant::now();
    let mut last = 0u64;
    let mut stalled_reported = false;
    while t0.elapsed() < std::time::Duration::from_secs(4) {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let pos = handle.handle.sample_position();
        let playing = handle.handle.is_playing();
        if pos != last {
            println!(
                "  t={:>4.1}s pos={pos:>7} playing={playing}",
                t0.elapsed().as_secs_f32()
            );
            last = pos;
        }
        if !stalled_reported && t0.elapsed() > std::time::Duration::from_secs(2) && pos == 0 {
            println!("  !! 2 秒内 sample_position 未推进（Play 无响应）");
            stalled_reported = true;
        }
    }

    println!("== 结束：最终 pos={} ==", handle.handle.sample_position());
    assert!(last > 0, "Play 后 sample_position 应推进");
}
