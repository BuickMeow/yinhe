//! 渲染线程导出端到端验证：spawn 完整音频系统 → 加载模型 → ExportStart →
//! 轮询完成 → 校验 WAV 写出。
//!
//! 与 GUI 导出走同一条链路（`AudioCommand::ExportStart` → 渲染线程导出模式），
//! 用于回归验证"导出复用实时引擎"的行为。
//!
//! 用法：
//! ```sh
//! cargo run --release -p yinhe-audio --example export_smoke
//! ```

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use yinhe_audio::channel_layout::ChannelLayout;
use yinhe_audio::export::{ExportProgress, WavBitDepth};
use yinhe_audio::{AudioCommand, spawn_cpal_audio};
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

/// spawn 音频系统（兼容 gpu/非 gpu 构建的签名差异；导出走 CPU 混音路径）。
fn spawn(sample_rate: u32, layout: ChannelLayout) -> Result<yinhe_audio::CpalAudioHandle, String> {
    #[cfg(feature = "gpu")]
    {
        spawn_cpal_audio(sample_rate, layout, cpal::BufferSize::Default, None, false)
    }
    #[cfg(not(feature = "gpu"))]
    {
        spawn_cpal_audio(sample_rate, layout, cpal::BufferSize::Default, None)
    }
}

fn main() {
    let sample_rate = 48000u32;
    let model = demo_model();
    let layout = ChannelLayout::from_model(&model);
    let expected = {
        // 4 拍 @120BPM = 2s。
        (sample_rate as u64) * 2
    };

    let handle = match spawn(sample_rate, layout) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("无法启动音频系统（本机可能无输出设备）: {e}");
            std::process::exit(1);
        }
    };
    println!("音频系统已启动（{} Hz）", handle.sample_rate);

    handle.handle.send(AudioCommand::LoadModel {
        model: model.clone(),
    });
    // worker 异步 PrepareModel：等待就绪（无音色库，仅验证导出链路）。
    std::thread::sleep(Duration::from_millis(500));

    let out = std::env::temp_dir().join("yinhe_export_smoke.wav");
    let _ = std::fs::remove_file(&out);
    let progress = ExportProgress::new();
    let cancel = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));
    println!("开始导出: {}", out.display());
    handle.handle.send(AudioCommand::ExportStart {
        path: out.clone(),
        bit_depth: WavBitDepth::Bit16,
        layer_count: None,
        restore_layer_count: None,
        progress: Arc::clone(&progress),
        cancel,
        pause,
    });

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let finished = progress.lock().map(|p| p.finished).unwrap_or(false);
        if finished {
            break;
        }
        if Instant::now() > deadline {
            eprintln!("导出超时（60s）");
            std::process::exit(1);
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let error = progress.lock().ok().and_then(|p| p.error.clone());
    if let Some(e) = error {
        eprintln!("导出失败: {e}");
        std::process::exit(1);
    }

    let reader = match hound::WavReader::open(&out) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("无法读取导出文件: {e}");
            std::process::exit(1);
        }
    };
    let spec = reader.spec();
    let frames = reader.len() as u64 / spec.channels as u64;
    println!(
        "导出完成: {} Hz / {} ch / {} bit / {} 帧 ({:.2}s)",
        spec.sample_rate,
        spec.channels,
        spec.bits_per_sample,
        frames,
        frames as f64 / spec.sample_rate as f64
    );
    // 主内容 2s；尾音上限 30s（无插件、无 voice 时应立即收尾）。
    assert_eq!(spec.sample_rate, sample_rate, "采样率应为设备采样率");
    assert_eq!(spec.channels, 2, "立体声");
    assert!(
        frames >= expected,
        "帧数应不少于主内容（{frames} < {expected}）"
    );
    assert!(
        frames <= expected + sample_rate as u64,
        "无插件时不应产生超过 1s 的多余尾音（{frames}）"
    );
    println!("导出链路验证通过。");
}
