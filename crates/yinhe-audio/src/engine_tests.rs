use super::*;
use crate::audio_model::{ChannelAudioEvent, ControlEvent};
use std::collections::BTreeMap;
use yinhe_core::{ConductorData, NoteEvent, PcEvent, ProjectMeta, TrackData, YinModel};
use yinhe_editor_core::document::Document;
use yinhe_types::automation::{ParamDevice, builtin_synth_param};
use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, KEY_COUNT, SegmentShape};

use crate::audio_model::AudioEvent;
use crate::channel_layout::ChannelLayout;

fn make_model_with_notes(notes: Vec<(u8, u32, u32, u8, u8)>) -> YinModel {
    let conductor = ConductorData {
        tempo: AutomationLane {
            target: AutomationTarget::Tempo,
            track: 0,
            events: vec![AutomationEvent {
                id: 0,
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
    let first_ch = notes.first().map(|n| n.4).unwrap_or(0);
    let mut t = TrackData::new(0, first_ch);
    t.name = "Track 1".into();
    let per_track_notes: Vec<Vec<NoteEvent>> = vec![
        notes
            .into_iter()
            .map(|(key, start, end, vel, _ch)| NoteEvent {
                start_tick: start,
                end_tick: end,
                key,
                velocity: vel,
                id: 0,
            })
            .collect(),
    ];
    let meta = ProjectMeta {
        ppq: 480,
        ..ProjectMeta::default()
    };
    let mut model = YinModel {
        conductor: Arc::new(conductor),
        tracks: vec![Arc::new(t)],
        meta,
        ..Default::default()
    };
    model.load_track_notes(per_track_notes);
    model.rebuild();
    model
}

/// track 0（MIDI 通道 0）的内置 XSynth 参数 target。
fn xsynth_target(id: u32) -> AutomationTarget {
    AutomationTarget::Param {
        device: ParamDevice::ChannelInstrument { channel: 0 },
        id,
        name: String::new(),
    }
}

/// CC → target：一律低层 CC（CC 广播方案）。
fn cc_target(controller: u8) -> AutomationTarget {
    AutomationTarget::CC { controller }
}

fn make_model_with_controls(
    cc: Vec<(u8, u32, u8)>,
    pb: Vec<(u32, i16)>,
    pc: Vec<(u32, u8)>,
    rpn: Vec<(u16, u32, f32)>,
) -> YinModel {
    let conductor = ConductorData {
        tempo: AutomationLane {
            target: AutomationTarget::Tempo,
            track: 0,
            events: vec![AutomationEvent {
                id: 0,
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
    let mut t = TrackData::new(0, 0);

    // Build automation lanes from CC events（入参为原始整数 CC 值，内部归一化）。
    let mut lanes: Vec<AutomationLane> = Vec::new();
    if !cc.is_empty() {
        let mut cc_by_controller: BTreeMap<u8, Vec<AutomationEvent>> = BTreeMap::new();
        for (controller, tick, value) in cc {
            cc_by_controller
                .entry(controller)
                .or_default()
                .push(AutomationEvent {
                    id: 0,
                    tick,
                    value: value as f32 / 127.0,
                    shape: SegmentShape::Step,
                });
        }
        for (controller, events) in cc_by_controller {
            lanes.push(AutomationLane {
                target: cc_target(controller),
                track: 0,
                events,
            });
        }
    }

    // Pitch bend lane（入参为相对中心的原始偏移 -8192..8191，内部归一化）。
    if !pb.is_empty() {
        let events: Vec<AutomationEvent> = pb
            .into_iter()
            .map(|(tick, value)| AutomationEvent {
                id: 0,
                tick,
                value: (value + 8192) as f32 / 16383.0,
                shape: SegmentShape::Step,
            })
            .collect();
        lanes.push(AutomationLane {
            target: xsynth_target(builtin_synth_param::PITCH_BEND),
            track: 0,
            events,
        });
    }

    // RPN lanes（入参为原始整数，按绑定上限归一化）。
    for (key, tick, value) in rpn {
        let (target, max) = match key {
            0 => (xsynth_target(builtin_synth_param::PB_SENSITIVITY), 127.0),
            1 => (xsynth_target(builtin_synth_param::FINE_TUNE), 16383.0),
            2 => (xsynth_target(builtin_synth_param::COARSE_TUNE), 127.0),
            _ => (AutomationTarget::Rpn { parameter: key }, 16383.0),
        };
        lanes.push(AutomationLane {
            target,
            track: 0,
            events: vec![AutomationEvent {
                id: 0,
                tick,
                value: value / max,
                shape: SegmentShape::Step,
            }],
        });
    }

    t.automation_lanes = lanes;
    t.program_change = pc
        .into_iter()
        .map(|(tick, program)| PcEvent {
            tick,
            program,
            bank_msb: 0,
            bank_lsb: 0,
        })
        .collect();
    let mut model = YinModel {
        conductor: Arc::new(conductor),
        tracks: vec![Arc::new(t)],
        meta: ProjectMeta {
            ppq: 480,
            ..ProjectMeta::default()
        },
        ..Default::default()
    };
    model.rebuild();
    model
}

// ---------------------------------------------------------------------------
// 集成测试：用 Document 模拟真实编辑流程
// ---------------------------------------------------------------------------

/// 用当前 model 的 ChannelLayout spawn 引擎，模拟 App 的 rebuild_audio_if_needed。
fn spawn_engine_for_doc(doc: &Document, sample_rate: u32) -> AudioEngine {
    let layout = crate::spawn::channels_for_model(&doc.data.model);
    let mut engine = AudioEngine::new(sample_rate, layout);
    engine.handle_command(AudioCommand::LoadModel {
        model: Arc::clone(&doc.data.model),
    });
    engine
}

/// 构造 cyber-night 风格的模型：RPN(0) PBS 2→48（tick 768）、PitchBend 滑音、CC7。
/// 第 2 小节 = tick 768（PPQ 480，4/4）。
fn make_chase_model() -> YinModel {
    let conductor = ConductorData {
        tempo: AutomationLane {
            target: AutomationTarget::Tempo,
            track: 0,
            events: vec![AutomationEvent {
                id: 0,
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
    let mut t = TrackData::new(0, 0);
    t.name = "Chase".into();
    t.automation_lanes = vec![
        AutomationLane {
            target: xsynth_target(builtin_synth_param::PB_SENSITIVITY),
            track: 0,
            events: vec![
                AutomationEvent {
                    id: 0,
                    tick: 0,
                    value: 2.0 / 127.0,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    id: 0,
                    tick: 768,
                    value: 48.0 / 127.0,
                    shape: SegmentShape::Step,
                },
            ],
        },
        AutomationLane {
            target: xsynth_target(builtin_synth_param::PITCH_BEND),
            track: 0,
            events: vec![
                AutomationEvent {
                    id: 0,
                    tick: 336,
                    value: 8192.0 / 16383.0,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    id: 0,
                    tick: 1536,
                    value: 10892.0 / 16383.0,
                    shape: SegmentShape::Step,
                },
            ],
        },
        AutomationLane {
            target: cc_target(7),
            track: 0,
            events: vec![
                AutomationEvent {
                    id: 0,
                    tick: 192,
                    value: 100.0 / 127.0,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    id: 0,
                    tick: 768,
                    value: 80.0 / 127.0,
                    shape: SegmentShape::Step,
                },
            ],
        },
    ];
    // 至少一个音符：track_audible_count > 0，否则 skip_track 会把该轨道当 mute。
    let per_track_notes: Vec<Vec<NoteEvent>> = vec![vec![NoteEvent {
        start_tick: 0,
        end_tick: 100,
        key: 60,
        velocity: 100,
        id: 0,
    }]];
    let meta = ProjectMeta {
        ppq: 480,
        ..ProjectMeta::default()
    };
    let mut model = YinModel {
        conductor: Arc::new(conductor),
        tracks: vec![Arc::new(t)],
        meta,
        ..Default::default()
    };
    model.load_track_notes(per_track_notes);
    model.rebuild();
    model
}

// ---------------------------------------------------------------------------
// tick 域化回归测试：sample↔tick 转换、全曲渲染完整性、零长段、seek 去重
// ---------------------------------------------------------------------------

/// 构造带 tempo 变速的模型：`tempo_events` = [(tick, BPM), ...]，`notes` = [(key, start, end)]。
fn make_model_with_tempo(
    tempo_events: Vec<(u32, f32)>,
    notes: Vec<(u8, u32, u32)>,
) -> Arc<YinModel> {
    let conductor = ConductorData {
        tempo: AutomationLane {
            target: AutomationTarget::Tempo,
            track: 0,
            events: tempo_events
                .into_iter()
                .map(|(tick, value)| AutomationEvent {
                    id: 0,
                    tick,
                    value,
                    shape: SegmentShape::Step,
                })
                .collect(),
        },
        time_sig: Vec::new(),
        key_sig: Vec::new(),
        markers: Vec::new(),
        lyrics: Vec::new(),
        chord: Vec::new(),
    };
    let per_track_notes: Vec<Vec<NoteEvent>> = vec![
        notes
            .into_iter()
            .map(|(key, start, end)| NoteEvent {
                start_tick: start,
                end_tick: end,
                key,
                velocity: 100,
                id: 0,
            })
            .collect(),
    ];
    let mut model = YinModel {
        conductor: Arc::new(conductor),
        tracks: vec![Arc::new(TrackData::new(0, 0))],
        meta: ProjectMeta {
            ppq: 480,
            ..ProjectMeta::default()
        },
        ..Default::default()
    };
    model.load_track_notes(per_track_notes);
    model.rebuild();
    Arc::new(model)
}

// ---------------------------------------------------------------------------
// 查询式 chase 回归测试：模型 lane 二分 + 曲线实时插值
// ---------------------------------------------------------------------------

/// 构造单 track 带一条 lane 的模型。
fn model_with_lane(lane: AutomationLane) -> YinModel {
    let mut t = TrackData::new(0, 0);
    t.automation_lanes = vec![lane];
    let mut model = YinModel {
        tracks: vec![Arc::new(t)],
        meta: ProjectMeta {
            ppq: 480,
            ..ProjectMeta::default()
        },
        ..Default::default()
    };
    model.rebuild();
    model
}

// ---------------------------------------------------------------------------
// 通道处理回归音源：CC7/10/11/71/74 → 内置音源通道处理段
// ---------------------------------------------------------------------------

mod compare_tests {
    use super::*;

    /// EnchantedLove 对比渲染（CpuSynth vs xsynth）：输出 raw f32（交错立体声）
    /// 供波形/频谱分析。用户报告连续同 key 高频音符处有 click，xsynth 无。
    #[test]
    #[ignore]
    fn render_compare_enchanted_love() {
        let sfz = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";
        let midi = "/Users/jieneng/Music/MIDIs/EnchantedLove.mid";
        let sr = 48_000u32;
        let seconds = 300u64;
        let model = Arc::new(yinhe_midi::parse_path(midi).expect("parse"));
        let layout = if let Some(n) = std::env::var("YINHE_PROF_CHANNELS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
        {
            crate::channel_layout::ChannelLayout::from_mask(vec![true; n.min(256)])
        } else {
            crate::spawn::channels_for_model(&model)
        };
        let active: Vec<u8> = (0..16u8)
            .filter(|c| layout.is_active(*c as usize))
            .collect();
        eprintln!("active channels: {active:?}");
        // 定位"同 key 连续音符"密集段（用户报告的问题场景）
        let ppq = model.meta.ppq.max(1);
        for key in 0..128usize {
            let notes = &model.notes[key];
            if notes.len() < 10 {
                continue;
            }
            let mut best = (0usize, 0u32);
            let mut cur = 1usize;
            let mut cur_start = notes[0].start_tick;
            for w in 1..notes.len() {
                if notes[w].start_tick.saturating_sub(notes[w - 1].start_tick) < ppq / 8 {
                    cur += 1;
                    if cur > best.0 {
                        best = (cur, cur_start);
                    }
                } else {
                    cur = 1;
                    cur_start = notes[w].start_tick;
                }
            }
            if best.0 >= 20 {
                eprintln!(
                    "  key={key} 最长同键密集段={} 个音符 起于 tick={}（约 {:.2}s @120bpm）",
                    best.0,
                    best.1,
                    best.1 as f64 / ppq as f64 * 0.5
                );
            }
        }
        for (name, backend) in [("cpu", 1u8), ("xsynth", 0), ("cpu_l4", 2u8)] {
            let mut e = AudioEngine::new(sr, layout.clone());
            e.handle_command(AudioCommand::LoadModel {
                model: Arc::clone(&model),
            });
            match backend {
                0 => {
                    let configs: Vec<(u8, Vec<String>)> =
                        active.iter().map(|c| (*c, vec![sfz.to_string()])).collect();
                    e.handle_command(AudioCommand::SetSoundFonts {
                        configs: Box::new(configs),
                    });
                }
                _ => {
                    let mut cs = yinhe_synth::CpuSynth::new(sr);
                    cs.set_layer_count(if backend == 2 { Some(4) } else { None });
                    for c in &active {
                        cs.load_dense_soundfonts(*c as u32, &[std::path::PathBuf::from(sfz)])
                            .expect("sf");
                    }
                    e.synth.cpu_synth = cs;
                }
            }
            e.handle_command(AudioCommand::Play { from_sample: 0 });
            let frames = 512usize;
            let mut buf = vec![0.0f32; frames * 2];
            let target = seconds * sr as u64;
            let mut out: Vec<f32> = Vec::new();
            let t0 = std::time::Instant::now();
            while (out.len() as u64 / 2) < target {
                e.render(&mut buf);
                out.extend_from_slice(&buf);
            }
            let mut bytes = Vec::with_capacity(out.len() * 4);
            for f in &out {
                bytes.extend_from_slice(&f.to_le_bytes());
            }
            let path = format!("/tmp/compare_{name}.raw");
            std::fs::write(&path, &bytes).expect("write");
            eprintln!(
                "{name}: {} frames 耗时 {:?} -> {path}",
                out.len() / 2,
                t0.elapsed()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 同 MIDI 通道、同 tick、同一语义目标的自动化取 max（播放聚合）
// ---------------------------------------------------------------------------

/// 两轨同 MIDI 通道（ch0）各一条 CC 自动化 lane，同 tick 各一个 Step 事件。
fn make_two_track_cc_model(controller: u8, tick: u32, v0: u8, v1: u8) -> YinModel {
    let lane = |track: u16, value: u8| AutomationLane {
        target: cc_target(controller),
        track,
        events: vec![AutomationEvent {
            id: 0,
            tick,
            value: value as f32 / 127.0,
            shape: SegmentShape::Step,
        }],
    };
    let mut t0 = TrackData::new(0, 0);
    t0.automation_lanes = vec![lane(0, v0)];
    let mut t1 = TrackData::new(0, 0);
    t1.automation_lanes = vec![lane(1, v1)];
    let mut model = YinModel {
        tracks: vec![Arc::new(t0), Arc::new(t1)],
        meta: ProjectMeta {
            ppq: 480,
            ..ProjectMeta::default()
        },
        ..Default::default()
    };
    model.rebuild();
    model
}

/// 本块 dispatch 实际会发送的 CC 代表事件（与 dispatch 同一套过滤 + 取 max）。
/// 返回 `(controller, value)`，按事件表顺序。
fn cc_reps(engine: &AudioEngine) -> Vec<(u8, f32)> {
    let events = &engine.playback.cc_events;
    let reps = crate::audio_model::aggregate_max_reps(events, |cc| {
        let track_skipped = engine
            .playback
            .skip_track
            .get(cc.track as usize)
            .copied()
            .unwrap_or(false);
        let lane_skipped = engine
            .playback
            .am_lane_skip
            .get(cc.track as usize)
            .and_then(|v| v.get(cc.lane as usize))
            .copied()
            .unwrap_or(false);
        !track_skipped && !lane_skipped
    });
    reps.iter()
        .filter_map(|&i| match &events[i].event {
            AudioEvent::Cc { controller, value } => Some((*controller, *value)),
            _ => None,
        })
        .collect()
}

/// 测试用 insert：持续输出一个按样本衰减的"尾音"，忽略输入。
struct TailInsert {
    value: f32,
}

impl yinhe_mixer::InsertProcessor for TailInsert {
    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for i in 0..left.len().min(right.len()) {
            left[i] += self.value;
            right[i] += self.value;
            self.value *= 0.999;
        }
    }

    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

fn tail_peak(engine: &mut AudioEngine) -> f32 {
    let mut buf = vec![0.0f32; 512 * 2];
    engine.render_tail(&mut buf);
    buf.iter().fold(0.0_f32, |m, s| m.max(s.abs()))
}

#[path = "engine_tests/dispatch.rs"]
mod dispatch;
#[path = "engine_tests/lifecycle.rs"]
mod lifecycle;
#[path = "engine_tests/render.rs"]
mod render;
