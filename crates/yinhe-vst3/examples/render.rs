//! 手动验证：VST3 音频处理（效果器/乐器）。
//!
//! 用法：
//!   `cargo run -p yinhe-vst3 --example render -- <bundle> <class_id> [--instrument]`
//!
//! 效果器模式：输入 440Hz 正弦（幅度 0.25），观察输出 RMS/峰值。
//! 乐器模式：第 5 块发 NoteOn、第 25 块 NoteOff，观察声音起落。

use std::path::Path;

use yinhe_mixer::PluginEvent;
use yinhe_vst3::Vst3PluginInstance;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(bundle) = args.get(1) else {
        eprintln!("用法: render <bundle.vst3> <class_id> [--instrument]");
        std::process::exit(2);
    };
    let Some(class_id) = args.get(2) else {
        eprintln!("用法: render <bundle.vst3> <class_id> [--instrument]");
        std::process::exit(2);
    };
    let instrument = args.iter().any(|a| a == "--instrument");

    let instance = match Vst3PluginInstance::load(Path::new(bundle), class_id) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("加载失败: {e}");
            std::process::exit(1);
        }
    };
    let mut processor = match instance.activate_audio(48_000.0, 512) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("激活失败: {e}");
            std::process::exit(1);
        }
    };
    println!(
        "已激活：{} 个参数，模式 = {}",
        instance.params().len(),
        if instrument { "乐器" } else { "效果器" }
    );

    let frames = 512usize;
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    let mut events: Vec<PluginEvent> = Vec::new();
    let mut phase = 0.0f32;

    for block in 0..40u64 {
        if instrument {
            left.fill(0.0);
            right.fill(0.0);
        } else {
            for i in 0..frames {
                let s = (phase * std::f32::consts::TAU).sin() * 0.25;
                phase = (phase + 440.0 / 48_000.0).fract();
                left[i] = s;
                right[i] = s;
            }
        }

        events.clear();
        if instrument && block == 5 {
            events.push(PluginEvent::NoteOn {
                time: 0,
                channel: 0,
                key: 60,
                velocity: 100.0 / 127.0,
            });
        }
        if instrument && block == 25 {
            events.push(PluginEvent::NoteOff {
                time: 0,
                channel: 0,
                key: 60,
                velocity: 0.0,
            });
        }

        let input = if instrument {
            None
        } else {
            Some((left.as_slice(), right.as_slice()))
        };
        if let Err(e) = processor.process_block(&events, block * frames as u64, input) {
            eprintln!("block {block} 处理失败: {e}");
            std::process::exit(1);
        }
        let (ol, or) = processor.output();
        let rms = (ol.iter().chain(or).map(|v| v * v).sum::<f32>() / (frames * 2) as f32).sqrt();
        let peak = ol.iter().chain(or).map(|v| v.abs()).fold(0.0f32, f32::max);
        println!("block {block:02}: rms={rms:.5} peak={peak:.5}");
    }

    processor.stop();
    println!("完成");
}
