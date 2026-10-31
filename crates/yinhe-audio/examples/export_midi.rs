//! 开发用：MIDI → WAV（走完整播放/导出链路，验证多乐器与 PC 切换）。
//!
//! ```sh
//! cargo run --release -p yinhe-audio --example export_midi -- \
//!   song.mid GM.sf2 out.wav [yinhe|xsynth]
//! ```

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use yinhe_audio::channel_layout::ChannelLayout;
use yinhe_audio::export::{ExportProgress, WavBitDepth};
use yinhe_audio::{AudioCommand, SynthEngine, spawn_cpal_audio};
use yinhe_types::Interpolation;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: export_midi <midi> <soundfont> <out.wav> [yinhe|xsynth]");
        std::process::exit(2);
    }
    let midi_path = &args[1];
    let sf_path = &args[2];
    let out = &args[3];
    let engine = match args.get(4).map(|s| s.as_str()) {
        Some("xsynth") => SynthEngine::XSynthCpu,
        _ => SynthEngine::YinheCpu,
    };

    let mut model = yinhe_midi::parse_path(midi_path).expect("解析 MIDI 失败");
    model.rebuild();
    let model = Arc::new(model);
    let layout = ChannelLayout::from_model(&model);

    let handle = spawn_cpal_audio(
        48_000,
        layout.clone(),
        yinhe_audio::BufferSize::Default,
        None,
        engine,
        Interpolation::default(),
    )
    .expect("启动音频系统失败");

    handle.handle.send(AudioCommand::LoadModel {
        model: Arc::clone(&model),
    });

    // 所有激活通道共用同一份音色库；乐器由 MIDI 的 Program Change 决定。
    let configs: Vec<(u8, Vec<String>)> = (0..layout.num_channels())
        .filter(|&ch| layout.is_active(ch as usize))
        .map(|ch| (ch as u8, vec![sf_path.clone()]))
        .collect();
    handle.handle.send(AudioCommand::SetSoundFonts {
        configs: Box::new(configs),
    });

    // 等 worker 加载音色库（大 SF2 需要几秒）。
    std::thread::sleep(Duration::from_secs(2));

    let progress = ExportProgress::new();
    let cancel = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));
    handle.handle.send(AudioCommand::ExportStart {
        path: std::path::PathBuf::from(out),
        bit_depth: WavBitDepth::Bit16,
        layer_count: None,
        restore_layer_count: None,
        progress: Arc::clone(&progress),
        cancel,
        pause,
    });

    let deadline = Instant::now() + Duration::from_secs(900);
    loop {
        if progress.lock().map(|p| p.finished).unwrap_or(false) {
            break;
        }
        if Instant::now() > deadline {
            eprintln!("导出超时");
            std::process::exit(1);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    if let Some(e) = progress.lock().ok().and_then(|p| p.error.clone()) {
        eprintln!("导出失败: {e}");
        std::process::exit(1);
    }
    println!("完成: {out}");
}
