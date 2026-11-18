//! 音频模型：事件词汇、聚合与按轨快照。
//!
//! 拆成三个子模块，避免单文件同时承载事件、聚合与模型：
//! - `event`：事件枚举、排序键、活动音符；
//! - `aggregate`：CC 聚合、自动化展开、事件投递；
//! - `model`：`PreparedModel` / `AudioModel` 与 tick/sample 换算。

mod aggregate;
mod event;
mod model;

pub(crate) use aggregate::*;
pub(crate) use event::*;
pub(crate) use model::*;

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use super::*;
    use yinhe_core::{ConductorData, ProjectMeta, TrackData, YinModel};
    use yinhe_types::automation::{ParamDevice, builtin_synth_param};
    use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, SegmentShape};

    /// 构建 1 轨道模型，给定 automation lanes。
    fn model_with_lanes(lanes: Vec<AutomationLane>) -> YinModel {
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
        t.automation_lanes = lanes;
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

    /// 在 `cc_events` 中找第一个匹配 `pred` 事件的索引。
    fn index_of<F>(events: &[SortedCC], pred: F) -> Option<usize>
    where
        F: Fn(&AudioEvent) -> bool,
    {
        events.iter().position(|e| pred(&e.event))
    }

    /// 回归测试：默认乐器表覆盖 A01..P16，B..P 的鼓声明也要生效。
    #[test]
    fn channel_instruments_beyond_port_a_affect_percussion() {
        let mut model = model_with_lanes(vec![]);
        model.channel_instruments[0xC5] = Some(yinhe_core::ChannelInstrument {
            bank_msb: 0,
            bank_lsb: 0,
            program: 0,
            is_drum: true,
        });

        let decl = AudioModel::from_model(&model).percussion_declarations();
        assert!(decl.contains(&(0xC5, true)));
    }

    /// 回归测试：非 MIDI 轨不再回退到 A01。音频/文件夹/Master 的
    /// `port/channel` 默认是 0，旧实现会把它们当成 MIDI 通道 0。
    #[test]
    fn non_midi_tracks_have_no_midi_channel() {
        let mut model = model_with_lanes(vec![]);
        let mut audio = TrackData::new(0, 0);
        audio.kind = yinhe_core::TrackKind::Audio;
        audio.audio_channel = None;
        let mut folder = TrackData::new(0, 0);
        folder.kind = yinhe_core::TrackKind::Folder;
        let mut master = TrackData::new(0, 0);
        master.kind = yinhe_core::TrackKind::Master;
        model.tracks = vec![
            Arc::new(TrackData::new(2, 5)),
            Arc::new(audio),
            Arc::new(folder),
            Arc::new(master),
        ];

        let audio_model = AudioModel::from_model(&model);
        assert_eq!(audio_model.track_channel(0), Some(0x25));
        assert_eq!(audio_model.track_channel(1), None);
        assert_eq!(audio_model.track_channel(2), None);
        assert_eq!(audio_model.track_channel(3), None);
        assert_eq!(audio_model.track_channel(99), None);
    }

    /// 回归测试：bank 声明（独立 CC0 自动化 + 被 PC 折叠的 CC0）必须全部进入
    /// track_banks，供鼓/乐器模式检测使用——否则 10 通道用 CC0<120 声明为乐器
    /// 时会被当作默认鼓通道（事件丢失 bug）。
    #[test]
    fn track_banks_merge_cc0_lane_and_folded_pc_bank() {
        use yinhe_types::PcEvent;

        let mut model = model_with_lanes(vec![AutomationLane {
            target: AutomationTarget::CC { controller: 0 },
            track: 0,
            events: vec![AutomationEvent {
                id: 0,
                tick: 100,
                value: 0.0, // 乐器 bank
                shape: SegmentShape::Step,
            }],
        }]);
        // 轨道 2：同 tick CC0+PC 被折叠进 PcEvent.bank_msb（独立 lane 不存在）。
        let mut t2 = TrackData::new(0, 9);
        t2.program_change = vec![
            PcEvent {
                tick: 200,
                program: 0,
                bank_msb: 0, // 乐器 bank（XG 风格声明）
                bank_lsb: 0,
            },
            PcEvent {
                tick: 400,
                program: 0,
                bank_msb: 121, // 鼓 bank
                bank_lsb: 0xFF,
            },
        ];
        model.tracks.push(Arc::new(t2));
        // 轨道 3：无任何 bank 声明。
        model.tracks.push(Arc::new(TrackData::new(0, 3)));

        let audio = AudioModel::from_model(&model);
        assert_eq!(audio.track_banks[0], vec![(100, 0)]);
        assert_eq!(audio.track_banks[1], vec![(200, 0), (400, 121)]);
        assert!(audio.track_banks[2].is_empty());
    }

    /// 鼓组声明表：GM 默认 → 通道默认乐器（界面「鼓通道」开关）→ 轨内 bank，
    /// 后者覆盖前者；且 `channel_instruments` 的鼓声明不再泄漏成 CC0。
    #[test]
    fn percussion_declarations_follow_precedence() {
        let mut model = model_with_lanes(vec![]);
        // 轨道 0 用通道 9（GM 默认鼓）；UI 关掉鼓 → 必须能覆盖。
        model.channel_instruments[9] = Some(yinhe_core::ChannelInstrument {
            bank_msb: 0,
            bank_lsb: 0,
            program: 0,
            is_drum: false,
        });
        let decl = AudioModel::from_model(&model).percussion_declarations();
        fn of(decl: &[(u16, bool)], ch: u16) -> Option<bool> {
            decl.iter().rev().find(|(c, _)| *c == ch).map(|(_, on)| *on)
        }
        assert_eq!(of(&decl, 9), Some(false), "UI 关鼓必须压过 GM 默认鼓");
        assert_eq!(of(&decl, 25), Some(true), "其它 port 的 9 通道仍是 GM 鼓");

        // 打开鼓 = is_drum。
        model.channel_instruments[9] = Some(yinhe_core::ChannelInstrument {
            bank_msb: 0,
            bank_lsb: 0,
            program: 0,
            is_drum: true,
        });
        let decl = AudioModel::from_model(&model).percussion_declarations();
        assert_eq!(of(&decl, 9), Some(true));

        // 轨内 bank 声明压过通道默认乐器。
        let mut t = TrackData::new(0, 9);
        t.program_change = vec![yinhe_types::PcEvent {
            tick: 0,
            program: 0,
            bank_msb: 0,
            bank_lsb: 0xFF,
        }];
        model.tracks.push(Arc::new(t));
        model.channel_instruments[9] = Some(yinhe_core::ChannelInstrument {
            bank_msb: 0,
            bank_lsb: 0,
            program: 0,
            is_drum: true,
        });
        let decl = AudioModel::from_model(&model).percussion_declarations();
        assert_eq!(
            of(&decl, 9),
            Some(false),
            "轨内 bank 声明优先于通道默认乐器"
        );

        // 鼓通道不得下发 bank：鼓组不占真实 bank 号，发出去会被当成普通 bank
        // （鼓通道的 PC 事件 track = u16::MAX，与轨内事件区分）。
        let events = flatten_automation_to_cc_events(&model, 1);
        let channel_level_bank = |cc: u8| {
            events.iter().any(|e| {
                e.track == u16::MAX
                    && matches!(
                        e.event,
                        AudioEvent::Cc {
                            controller,
                            ..
                        } if controller == cc
                    )
            })
        };
        assert!(!channel_level_bank(0), "鼓通道不能下发 Bank MSB");
        assert!(!channel_level_bank(32), "鼓通道不能下发 Bank LSB");

        // 旋律通道仍要下发 bank（MSB + LSB 都要）。
        model.channel_instruments[4] = Some(yinhe_core::ChannelInstrument {
            bank_msb: 17,
            bank_lsb: 3,
            program: 40,
            is_drum: false,
        });
        let events = flatten_automation_to_cc_events(&model, 1);
        let bank_of = |ch: u32, cc: u8| {
            events.iter().find_map(|e| match e.event {
                AudioEvent::Cc { controller, value } if controller == cc && e.channel == ch => {
                    Some(value)
                }
                _ => None,
            })
        };
        assert_eq!(bank_of(4, 0), Some(17.0), "Bank MSB");
        assert_eq!(bank_of(4, 32), Some(3.0), "Bank LSB");
    }

    /// 插件乐器参数 lane 展平成 plugin_param 事件：占位 event 不参与 xsynth 语义，
    /// 值保持归一化 0..1（越界钳制），param_id/乐器通道原样携带。
    #[test]
    fn emit_plugin_instrument_param_keeps_normalized() {
        let model = model_with_lanes(vec![AutomationLane {
            target: AutomationTarget::Param {
                device: ParamDevice::PluginInstrument { channel: 2, uid: 7 },
                id: 42,
                name: "Cutoff".into(),
            },
            track: 0,
            events: vec![
                AutomationEvent {
                    id: 0,
                    tick: 0,
                    value: 0.25,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    id: 0,
                    tick: 10,
                    value: 1.7, // 越界：展平钳制到 1.0
                    shape: SegmentShape::Step,
                },
            ],
        }]);
        let events = flatten_automation_to_cc_events(&model, 1);
        assert_eq!(events.len(), 2);
        for e in events.iter() {
            let pp = e.plugin_param.expect("插件参数事件必须带 plugin_param");
            assert_eq!(pp.channel, 2);
            assert_eq!(pp.uid, 7);
            assert_eq!(pp.param_id, 42);
            // 占位 event 恒为 Raw(0,0)：dispatch 按 plugin_param 分支优先处理。
            assert!(matches!(
                e.event,
                AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(0, 0)))
            ));
        }
        assert_eq!(events[0].plugin_param.map(|p| p.value), Some(0.25));
        assert_eq!(events[1].plugin_param.map(|p| p.value), Some(1.0));
    }

    /// 通道级默认乐器：tick 0 先下发默认 PC，MIDI 内 PC 同日生效时在其后覆盖。
    #[test]
    fn channel_instrument_default_pc_before_midi_pc() {
        let mut model = model_with_lanes(vec![]);
        Arc::make_mut(&mut model.tracks[0]).program_change = vec![yinhe_types::PcEvent {
            tick: 0,
            program: 9,
            bank_msb: 0,
            bank_lsb: 0xFF,
        }];
        model.channel_instruments[0] = Some(yinhe_core::ChannelInstrument {
            bank_msb: 0,
            bank_lsb: 0,
            program: 42,
            is_drum: false,
        });
        let events = flatten_automation_to_cc_events(&model, 1);
        let pcs: Vec<u8> = events
            .iter()
            .filter_map(|e| match &e.event {
                AudioEvent::ProgramChange(p) => Some(*p),
                _ => None,
            })
            .collect();
        assert_eq!(pcs, vec![42, 9], "默认 PC 先发，MIDI PC 覆盖它");

        // 没有 MIDI PC 时只剩默认。
        Arc::make_mut(&mut model.tracks[0]).program_change.clear();
        let events = flatten_automation_to_cc_events(&model, 1);
        let pcs: Vec<u8> = events
            .iter()
            .filter_map(|e| match &e.event {
                AudioEvent::ProgramChange(p) => Some(*p),
                _ => None,
            })
            .collect();
        assert_eq!(pcs, vec![42]);
    }

    /// 低层 CC：归一化值在 flatten 边界还原成原始值域（CC7 = 100，f32 不量化）。
    #[test]
    fn emit_low_level_cc_restores_integer() {
        let model = model_with_lanes(vec![AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            track: 0,
            events: vec![AutomationEvent {
                id: 0,
                tick: 0,
                value: 100.0 / 127.0,
                shape: SegmentShape::Step,
            }],
        }]);
        let events = flatten_automation_to_cc_events(&model, 1);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].channel, 0);
        assert!(matches!(
            events[0].event,
            AudioEvent::Cc {
                controller: 7,
                value
            } if (value - 100.0).abs() < 1e-4
        ));
    }

    /// XSynth 内置参数：PB 中心/满值、RPN0 半音数都按原始整数上限还原。
    #[test]
    fn emit_pitch_bend_and_rpn_restore() {
        let pb_lane = AutomationLane {
            target: AutomationTarget::Param {
                device: ParamDevice::ChannelInstrument { channel: 0 },
                id: builtin_synth_param::PITCH_BEND,
                name: String::new(),
            },
            track: 0,
            events: vec![
                AutomationEvent {
                    id: 0,
                    tick: 0,
                    value: 8192.0 / 16383.0,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    id: 0,
                    tick: 10,
                    value: 1.0,
                    shape: SegmentShape::Step,
                },
            ],
        };
        let pbs_lane = AutomationLane {
            target: AutomationTarget::Param {
                device: ParamDevice::ChannelInstrument { channel: 0 },
                id: builtin_synth_param::PB_SENSITIVITY,
                name: String::new(),
            },
            track: 0,
            events: vec![AutomationEvent {
                id: 0,
                tick: 0,
                value: 2.0 / 127.0,
                shape: SegmentShape::Step,
            }],
        };
        let model = model_with_lanes(vec![pb_lane, pbs_lane]);
        let events = flatten_automation_to_cc_events(&model, 1);

        let pb_values: Vec<f32> = events
            .iter()
            .filter_map(|e| match e.event {
                AudioEvent::PitchBend(v) => Some(v),
                _ => None,
            })
            .collect();
        assert_eq!(pb_values.len(), 2);
        assert_eq!(pb_values[0], 0.0, "中心 8192 → 无弯音");
        assert!(
            (pb_values[1] - (16383.0 - 8192.0) / 8192.0).abs() < 1e-6,
            "满值 16383 应还原到 +1 附近，实际 {}",
            pb_values[1]
        );

        // RPN0（PBS 内置参数）：原生 Rpn 事件，归一化 f32（2 半音 → 2/127）。
        let pbs = events.iter().find_map(|e| match e.event {
            AudioEvent::Rpn {
                parameter: 0,
                value,
            } => Some(value),
            _ => None,
        });
        assert!(
            pbs.is_some_and(|v| (v - 2.0 / 127.0).abs() < 1e-6),
            "RPN0 半音数 2 应归一化为 2/127，实际 {pbs:?}"
        );
    }

    /// 回归对照：`effective_fades_all`（预计算，进 AudioModel 快照）必须与
    /// 旧逐 index 调用的结果逐位一致——含空/单片段、重叠方向、包含、同起点叠加。
    #[test]
    fn fade_precompute_matches_per_index() {
        let clip = |id, start, dur, fade_in, fade_out| yinhe_core::AudioClip {
            id,
            source: "s".into(),
            start_seconds: start,
            offset_seconds: 0.0,
            duration_seconds: dur,
            gain: 1.0,
            fade_in_seconds: fade_in,
            fade_out_seconds: fade_out,
            reversed: false,
            start_beat: None,
            warp: None,
        };
        let cases: Vec<Vec<yinhe_core::AudioClip>> = vec![
            vec![],
            vec![clip(1, 0.0, 4.0, 2.0, 2.0)],
            vec![clip(1, 0.0, 4.0, 0.0, 0.0), clip(2, 3.0, 2.0, 0.0, 0.0)],
            vec![clip(1, 0.0, 10.0, 0.0, 0.0), clip(2, 3.0, 2.0, 0.0, 0.0)],
            vec![
                clip(1, 0.0, 4.0, 0.0, 0.0),
                clip(2, 3.0, 2.0, 0.0, 0.0),
                clip(3, 3.0, 3.0, 0.5, 0.0),
                clip(4, 5.0, 2.0, 0.0, 1.5),
            ],
            vec![clip(1, 0.0, 2.0, 0.0, 0.0), clip(2, 3.0, 2.0, 0.0, 0.0)],
        ];
        for clips in &cases {
            let all = effective_fades_all(clips);
            assert_eq!(all.len(), clips.len());
            for (i, fade) in all.iter().enumerate() {
                assert_eq!(
                    *fade,
                    yinhe_core::effective_fades(clips, i),
                    "片段 {i} 的预计算结果应与逐次计算一致"
                );
            }
        }
    }

    /// `sample_to_tick`：空 tempo map / sample 0 / 精确边界。
    #[test]
    fn sample_to_tick_zero_and_exact_boundaries() {
        assert_eq!(sample_to_tick(0, &[], 480, 48_000.0), 0);
        // 120 BPM + 60 BPM 两段：48000 sample 前 50 sample/tick，之后 100 sample/tick。
        let segments = vec![
            yinhe_core::TempoSegment {
                start_tick: 0,
                start_time: 0.0,
                micros_per_quarter: 500_000,
            },
            yinhe_core::TempoSegment {
                start_tick: 960,
                start_time: 1.0,
                micros_per_quarter: 1_000_000,
            },
        ];
        assert_eq!(sample_to_tick(0, &segments, 480, 48_000.0), 0);
        assert_eq!(sample_to_tick(49, &segments, 480, 48_000.0), 0);
        assert_eq!(sample_to_tick(50, &segments, 480, 48_000.0), 1);
        // tempo 段边界：tick 960 == sample 48000，两侧各差 1 sample。
        assert_eq!(sample_to_tick(47_999, &segments, 480, 48_000.0), 959);
        assert_eq!(sample_to_tick(48_000, &segments, 480, 48_000.0), 960);
        assert_eq!(sample_to_tick(48_001, &segments, 480, 48_000.0), 960);
    }

    /// `sample_to_tick` 单调不降：跨变速段密集采样。
    #[test]
    fn sample_to_tick_monotonic_across_tempo_change() {
        let segments = vec![
            yinhe_core::TempoSegment {
                start_tick: 0,
                start_time: 0.0,
                micros_per_quarter: 500_000,
            },
            yinhe_core::TempoSegment {
                start_tick: 960,
                start_time: 1.0,
                micros_per_quarter: 250_000,
            },
        ];
        let mut prev = 0u32;
        for sample in (0..240_000u64).step_by(37) {
            let t = sample_to_tick(sample, &segments, 480, 48_000.0);
            assert!(
                t >= prev,
                "sample {sample}: t={t} < prev={prev}（必须单调不降）"
            );
            prev = t;
        }
    }

    /// 极值：
    /// - ppq/mpq 退化（0）时立即返回段起点（旧实现会 ~2^32 次空转）；
    /// - sample 超出 u32 tick 域时饱和，不经 i64→u32 截断返回任意值。
    #[test]
    fn sample_to_tick_extremes_saturate() {
        let degenerate = vec![yinhe_core::TempoSegment {
            start_tick: 123,
            start_time: 0.0,
            micros_per_quarter: 0,
        }];
        assert_eq!(sample_to_tick(0, &degenerate, 480, 48_000.0), 123);
        assert_eq!(sample_to_tick(u64::MAX, &degenerate, 480, 48_000.0), 123);
        assert_eq!(sample_to_tick(1_000_000, &degenerate, 0, 48_000.0), 123);

        let segments = vec![yinhe_core::TempoSegment {
            start_tick: 0,
            start_time: 0.0,
            micros_per_quarter: 500_000,
        }];
        let tick_ceiling_sample = tick_to_sample(u32::MAX, &segments, 480, 48_000.0);
        // 超出 tick 域的 sample 与 tick 上限处的查询同值（饱和到 u32::MAX 附近），
        // 而不是旧实现的 i64→u32 截断垃圾值。
        assert_eq!(
            sample_to_tick(u64::MAX, &segments, 480, 48_000.0),
            sample_to_tick(tick_ceiling_sample, &segments, 480, 48_000.0),
        );
    }

    /// 回归测试：同 tick 上 RPN 0 (PBS) 必须排在 PitchBend 之前。
    /// 见 commit 3490e02：若 PB 先于 PBS，PB 会用旧 PBS 计算弯音，导致音高异常。
    #[test]
    fn rpn_pbs_must_precede_pitch_bend_at_same_tick() {
        let lanes = vec![
            AutomationLane {
                target: AutomationTarget::Param {
                    device: ParamDevice::ChannelInstrument { channel: 0 },
                    id: builtin_synth_param::PITCH_BEND,
                    name: String::new(),
                },
                track: 0,
                events: vec![AutomationEvent {
                    id: 0,
                    tick: 0,
                    value: 1.0,
                    shape: SegmentShape::Step,
                }],
            },
            AutomationLane {
                target: AutomationTarget::Param {
                    device: ParamDevice::ChannelInstrument { channel: 0 },
                    id: builtin_synth_param::PB_SENSITIVITY,
                    name: String::new(),
                },
                track: 0,
                events: vec![AutomationEvent {
                    id: 0,
                    tick: 0,
                    value: 24.0 / 127.0,
                    shape: SegmentShape::Step,
                }],
            },
        ];
        let model = model_with_lanes(lanes);
        let events = flatten_automation_to_cc_events(&model, 1);

        let pbs_idx = index_of(&events, |e| {
            matches!(e, AudioEvent::Rpn { parameter: 0, .. })
        });
        let pb_idx = index_of(&events, |e| matches!(e, AudioEvent::PitchBend(_)));

        let pbs_idx = pbs_idx.expect("PBS event should exist");
        let pb_idx = pb_idx.expect("PitchBend event should exist");
        assert!(
            pbs_idx < pb_idx,
            "PBS (index {}) must precede PitchBend (index {}) at the same tick, \
             otherwise PB uses stale PBS and pitch is wrong (regression of 3490e02)",
            pbs_idx,
            pb_idx
        );
    }

    /// AR 自动化 lane 的 M/S 试听旁通已是**运行期动态掩码**（dispatch 时查询，
    /// 见 `build_am_lane_skip`），flatten 全量展平所有 lane。
    /// 本测试验证掩码规则：mute 的 lane 被跳过；有 solo 时音轨内只有被 solo
    /// 的 lane 不被跳过（作用域 = 音轨内，主音轨音符与其他音轨不受影响）。
    #[test]
    fn am_ms_builds_lane_skip_mask() {
        let lanes = vec![
            AutomationLane {
                target: AutomationTarget::CC { controller: 7 },
                track: 0,
                events: vec![AutomationEvent {
                    id: 0,
                    tick: 0,
                    value: 100.0 / 127.0,
                    shape: SegmentShape::Step,
                }],
            },
            AutomationLane {
                target: AutomationTarget::CC { controller: 10 },
                track: 0,
                events: vec![AutomationEvent {
                    id: 0,
                    tick: 0,
                    value: 64.0 / 127.0,
                    shape: SegmentShape::Step,
                }],
            },
        ];
        let model = model_with_lanes(lanes);
        let cc7 = AutomationTarget::CC { controller: 7 };
        let cc10 = AutomationTarget::CC { controller: 10 };
        let mask_for = |am_ms: &HashMap<
            (u16, yinhe_types::AutomationTarget),
            yinhe_types::AmMsState,
        >| build_am_lane_skip(&model, am_ms);

        // 无旁通：两条 lane 都不跳过（lane 顺序 = 声明顺序）。
        assert_eq!(mask_for(&HashMap::new()), vec![vec![false, false]]);

        // mute CC7：只有 CC7 被跳过。
        let mut mutes = HashMap::new();
        mutes.insert(
            (0u16, cc7.clone()),
            yinhe_types::AmMsState {
                mute: true,
                solo: false,
            },
        );
        assert_eq!(mask_for(&mutes), vec![vec![true, false]]);

        // solo CC10：同轨未 solo 的 CC7 被跳过，CC10 保留。
        let mut solos = HashMap::new();
        solos.insert(
            (0u16, cc10.clone()),
            yinhe_types::AmMsState {
                mute: false,
                solo: true,
            },
        );
        assert_eq!(mask_for(&solos), vec![vec![true, false]]);
    }

    /// 非标准 RPN（原生 Rpn 事件，不再拆 CC 序列）：同 tick 上必须排在 PB 之前。
    #[test]
    fn nonstandard_rpn_native_event_must_precede_pitch_bend() {
        let lanes = vec![
            AutomationLane {
                target: AutomationTarget::Param {
                    device: ParamDevice::ChannelInstrument { channel: 0 },
                    id: builtin_synth_param::PITCH_BEND,
                    name: String::new(),
                },
                track: 0,
                events: vec![AutomationEvent {
                    id: 0,
                    tick: 0,
                    value: 1.0,
                    shape: SegmentShape::Step,
                }],
            },
            // RPN 5（非标准）→ 原生 Rpn 事件
            AutomationLane {
                target: AutomationTarget::Rpn { parameter: 5 },
                track: 0,
                events: vec![AutomationEvent {
                    id: 0,
                    tick: 0,
                    value: 100.0 / 16383.0,
                    shape: SegmentShape::Step,
                }],
            },
        ];
        let model = model_with_lanes(lanes);
        let events = flatten_automation_to_cc_events(&model, 1);

        let rpn_idx = index_of(&events, |e| {
            matches!(e, AudioEvent::Rpn { parameter: 5, .. })
        });
        let pb_idx = index_of(&events, |e| matches!(e, AudioEvent::PitchBend(_)));

        let rpn_idx = rpn_idx.expect("RPN 5 event should exist");
        let pb_idx = pb_idx.expect("PitchBend event should exist");
        assert!(
            rpn_idx < pb_idx,
            "RPN 5 (index {}) must precede PitchBend (index {}) at the same tick",
            rpn_idx,
            pb_idx
        );
    }

    /// 覆盖 NRPN（原生 Nrpn 事件）：同 tick 上必须排在 PB 之前。
    #[test]
    fn nrpn_native_event_must_precede_pitch_bend() {
        let lanes = vec![
            AutomationLane {
                target: AutomationTarget::Param {
                    device: ParamDevice::ChannelInstrument { channel: 0 },
                    id: builtin_synth_param::PITCH_BEND,
                    name: String::new(),
                },
                track: 0,
                events: vec![AutomationEvent {
                    id: 0,
                    tick: 0,
                    value: 1.0,
                    shape: SegmentShape::Step,
                }],
            },
            AutomationLane {
                target: AutomationTarget::Nrpn { parameter: 10 },
                track: 0,
                events: vec![AutomationEvent {
                    id: 0,
                    tick: 0,
                    value: 100.0 / 16383.0,
                    shape: SegmentShape::Step,
                }],
            },
        ];
        let model = model_with_lanes(lanes);
        let events = flatten_automation_to_cc_events(&model, 1);

        let nrpn_idx = index_of(&events, |e| {
            matches!(e, AudioEvent::Nrpn { parameter: 10, .. })
        });
        let pb_idx = index_of(&events, |e| matches!(e, AudioEvent::PitchBend(_)));

        let nrpn_idx = nrpn_idx.expect("NRPN event should exist");
        let pb_idx = pb_idx.expect("PitchBend event should exist");
        assert!(
            nrpn_idx < pb_idx,
            "NRPN (index {}) must precede PitchBend (index {}) at the same tick",
            nrpn_idx,
            pb_idx
        );
    }

    /// 同 tick 上 FineTune (RPN 1) / CoarseTune (RPN 2) 也应排在 PB 前。
    #[test]
    fn rpn_fine_and_coarse_tune_precede_pitch_bend() {
        let lanes = vec![
            AutomationLane {
                target: AutomationTarget::Param {
                    device: ParamDevice::ChannelInstrument { channel: 0 },
                    id: builtin_synth_param::PITCH_BEND,
                    name: String::new(),
                },
                track: 0,
                events: vec![AutomationEvent {
                    id: 0,
                    tick: 0,
                    value: 1.0,
                    shape: SegmentShape::Step,
                }],
            },
            AutomationLane {
                target: AutomationTarget::Param {
                    device: ParamDevice::ChannelInstrument { channel: 0 },
                    id: builtin_synth_param::FINE_TUNE,
                    name: String::new(),
                },
                track: 0,
                events: vec![AutomationEvent {
                    id: 0,
                    tick: 0,
                    value: 9000.0 / 16383.0,
                    shape: SegmentShape::Step,
                }],
            },
            AutomationLane {
                target: AutomationTarget::Param {
                    device: ParamDevice::ChannelInstrument { channel: 0 },
                    id: builtin_synth_param::COARSE_TUNE,
                    name: String::new(),
                },
                track: 0,
                events: vec![AutomationEvent {
                    id: 0,
                    tick: 0,
                    value: 70.0 / 127.0,
                    shape: SegmentShape::Step,
                }],
            },
        ];
        let model = model_with_lanes(lanes);
        let events = flatten_automation_to_cc_events(&model, 1);

        let fine_idx = index_of(&events, |e| {
            matches!(e, AudioEvent::Rpn { parameter: 1, .. })
        });
        let coarse_idx = index_of(&events, |e| {
            matches!(e, AudioEvent::Rpn { parameter: 2, .. })
        });
        let pb_idx = index_of(&events, |e| matches!(e, AudioEvent::PitchBend(_)));

        let fine_idx = fine_idx.expect("FineTune event should exist");
        let coarse_idx = coarse_idx.expect("CoarseTune event should exist");
        let pb_idx = pb_idx.expect("PitchBend event should exist");
        assert!(
            fine_idx < pb_idx && coarse_idx < pb_idx,
            "FineTune (idx {}) and CoarseTune (idx {}) must precede PitchBend (idx {}) at the same tick",
            fine_idx,
            coarse_idx,
            pb_idx
        );
    }

    // -----------------------------------------------------------------------
    // 同 tick 同通道同目标取 max（聚合辅助）
    // -----------------------------------------------------------------------

    /// 构造测试用 SortedCC。
    fn scc(tick: u32, channel: u32, event: AudioEvent) -> SortedCC {
        SortedCC {
            tick,
            channel,
            track: 0,
            lane: 0,
            plugin_param: None,
            event,
        }
    }

    /// 同 tick 同通道同 CC：聚合为一个代表，值取 max（两个顺序都生效）。
    #[test]
    fn aggregate_max_reps_cc_takes_max_both_orders() {
        let asc = vec![
            scc(
                10,
                0,
                AudioEvent::Cc {
                    controller: 7,
                    value: 40.0,
                },
            ),
            scc(
                10,
                0,
                AudioEvent::Cc {
                    controller: 7,
                    value: 100.0,
                },
            ),
        ];
        let reps = aggregate_max_reps(&asc, |_| true);
        assert_eq!(reps.len(), 1, "同 tick 同通道同 CC 只保留一条");
        assert_eq!(agg_value(&asc[reps[0]]), 100.0, "取 max");

        let desc = vec![
            scc(
                10,
                0,
                AudioEvent::Cc {
                    controller: 7,
                    value: 100.0,
                },
            ),
            scc(
                10,
                0,
                AudioEvent::Cc {
                    controller: 7,
                    value: 40.0,
                },
            ),
        ];
        let reps = aggregate_max_reps(&desc, |_| true);
        assert_eq!(reps.len(), 1);
        assert_eq!(agg_value(&desc[reps[0]]), 100.0, "顺序颠倒仍取 max");
    }

    /// 不同语义键互不干扰；不聚合事件（ProgramChange）原样保留；输出顺序 =
    /// 组内首见顺序（同 tick 内 RPN 先于 PitchBend 的优先级不被破坏）。
    #[test]
    fn aggregate_max_reps_keeps_distinct_keys_and_order() {
        let events = vec![
            // 组 1：CC7（首见）
            scc(
                10,
                0,
                AudioEvent::Cc {
                    controller: 7,
                    value: 40.0,
                },
            ),
            // 不聚合：PC 原地保留
            scc(10, 0, AudioEvent::ProgramChange(5)),
            // 组 2：CC10（首见）
            scc(
                10,
                0,
                AudioEvent::Cc {
                    controller: 10,
                    value: 80.0,
                },
            ),
            // 组 1 的代表更新（值更大）
            scc(
                10,
                0,
                AudioEvent::Cc {
                    controller: 7,
                    value: 100.0,
                },
            ),
        ];
        let reps = aggregate_max_reps(&events, |_| true);
        let out: Vec<&AudioEvent> = reps.iter().map(|&i| &events[i].event).collect();
        assert_eq!(
            out,
            vec![
                &AudioEvent::Cc {
                    controller: 7,
                    value: 100.0
                },
                &AudioEvent::ProgramChange(5),
                &AudioEvent::Cc {
                    controller: 10,
                    value: 80.0
                },
            ],
            "代表事件按首见顺序发送，CC7/CC10 互不干扰"
        );
    }

    /// 过滤先于聚合：被过滤（mute/旁通）的事件不参与 max；整组被过滤则不发。
    #[test]
    fn aggregate_max_reps_filters_before_max() {
        let events = vec![
            scc(
                10,
                0,
                AudioEvent::Cc {
                    controller: 7,
                    value: 40.0,
                },
            ),
            scc(
                10,
                0,
                AudioEvent::Cc {
                    controller: 7,
                    value: 100.0,
                },
            ),
        ];
        // 过滤掉第二条（值 100 的"被旁通"事件）→ 代表降级为 40
        let reps = aggregate_max_reps(
            &events,
            |cc| !matches!(cc.event, AudioEvent::Cc { value, .. } if value == 100.0),
        );
        assert_eq!(reps.len(), 1);
        assert_eq!(agg_value(&events[reps[0]]), 40.0);

        // 全部过滤 → 整组不发
        let reps = aggregate_max_reps(&events, |_| false);
        assert!(reps.is_empty());
    }

    /// 不同 tick 的同键事件不聚合（后写覆盖由发送顺序决定）。
    #[test]
    fn aggregate_max_reps_does_not_merge_across_ticks() {
        let events = vec![
            scc(
                10,
                0,
                AudioEvent::Cc {
                    controller: 7,
                    value: 100.0,
                },
            ),
            scc(
                20,
                0,
                AudioEvent::Cc {
                    controller: 7,
                    value: 40.0,
                },
            ),
        ];
        let reps = aggregate_max_reps(&events, |_| true);
        assert_eq!(reps, vec![0, 1]);
    }

    /// xsynth 高层 Raw CC / PitchBendValue 与内部浮点 CC / PB 同目标聚合；
    /// RPN 按参数号独立（不与 PB 混）。
    #[test]
    fn aggregate_max_reps_unifies_high_level_and_float_events() {
        let events = vec![
            scc(
                10,
                0,
                AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(7, 40))),
            ),
            scc(
                10,
                0,
                AudioEvent::Cc {
                    controller: 7,
                    value: 100.0,
                },
            ),
            scc(
                10,
                0,
                AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
                    0.25,
                ))),
            ),
            scc(10, 0, AudioEvent::PitchBend(0.5)),
            scc(
                10,
                0,
                AudioEvent::Rpn {
                    parameter: 0,
                    value: 48.0 / 127.0,
                },
            ),
        ];
        let reps = aggregate_max_reps(&events, |_| true);
        let out: Vec<f32> = reps.iter().map(|&i| agg_value(&events[i])).collect();
        assert_eq!(out, vec![100.0, 0.5, 48.0 / 127.0]);
    }

    /// 插件参数事件按 (uid, param_id) 取 max；不同参数互不干扰。
    #[test]
    fn aggregate_max_reps_plugin_params_take_max() {
        let pp = |uid: u64, param_id: u32, value: f32| {
            Some(PluginParamEvent {
                channel: 0,
                uid,
                param_id,
                value,
            })
        };
        let placeholder =
            || AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(0, 0)));
        let events = vec![
            SortedCC {
                tick: 10,
                channel: 0,
                track: 0,
                lane: 0,
                plugin_param: pp(1, 2, 0.2),
                event: placeholder(),
            },
            SortedCC {
                tick: 10,
                channel: 0,
                track: 1,
                lane: 0,
                plugin_param: pp(1, 2, 0.9),
                event: placeholder(),
            },
            SortedCC {
                tick: 10,
                channel: 0,
                track: 2,
                lane: 0,
                plugin_param: pp(1, 3, 0.4),
                event: placeholder(),
            },
        ];
        let reps = aggregate_max_reps(&events, |_| true);
        assert_eq!(reps.len(), 2, "同参数取 max，不同参数各自保留");
        let vals: Vec<f32> = reps.iter().map(|&i| agg_value(&events[i])).collect();
        assert_eq!(vals, vec![0.9, 0.4]);
    }
}
