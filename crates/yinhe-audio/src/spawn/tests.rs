use super::*;

/// 测试用最小 insert 处理器。
struct TestInsert;

impl InsertProcessor for TestInsert {
    fn process(&mut self, _left: &mut [f32], _right: &mut [f32]) {}

    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

/// 回归：结构性变更命令必须走可靠通道。渲染线程曾因音色库/采样上传
/// 阻塞 4-5 秒导致命令通道（容量 16）满，InsertAdd 被静默丢弃 ——
/// UI 标记已发送、无重发路径，效果器永远不生效。
///
/// 设置类命令（层数/复音数/力度忽略/自动化密度）同属"单次、无重发"：
/// 走普通通道被丢后 UI 的 diff 已归零不再重发，设置永不生效。
#[test]
fn structural_commands_use_reliable_channel() {
    assert!(AudioCommand::Play { from_sample: 0 }.is_reliable());
    assert!(
        AudioCommand::LoadModel {
            model: std::sync::Arc::new(yinhe_core::YinModel::default()),
        }
        .is_reliable()
    );
    assert!(
        AudioCommand::InsertRemove {
            target: InsertTarget::Midi(0),
            slot: 0,
        }
        .is_reliable()
    );
    assert!(
        AudioCommand::SetInstrumentSlot {
            target: InsertTarget::Midi(0),
            slot_id: 0,
            processor: None,
        }
        .is_reliable()
    );
    assert!(AudioCommand::SetLayerCount { count: Some(4) }.is_reliable());
    assert!(AudioCommand::SetMaxVoices { max: None }.is_reliable());
    assert!(AudioCommand::SetIgnoreVelocity { threshold: 1 }.is_reliable());
    assert!(AudioCommand::SetAutomationDensity { density: 1 }.is_reliable());
    // 可重发/可合并的普通命令仍走有界通道（满了丢弃、下一次操作自愈）。
    assert!(!AudioCommand::RefreshLatency.is_reliable());
    assert!(!AudioCommand::PreviewStop.is_reliable());
}

/// 命令分类单点定义回归：每个变体都必须有具体诊断名（不再有 "Other"
/// 兜底）且可靠通道分类符合预期。新增变体时编译器先强制补 `meta()`
/// 分支，本表再锁住分类结果，防止新命令被漏进有界通道。
#[test]
fn command_meta_classifies_every_variant() {
    let model = || Arc::new(yinhe_core::YinModel::default());
    let insert = || -> Box<dyn InsertProcessor> { Box::new(TestInsert) };
    let cases: Vec<(AudioCommand, &str, bool)> = vec![
        (AudioCommand::Play { from_sample: 0 }, "Play", true),
        (AudioCommand::Resume, "Resume", true),
        (AudioCommand::Pause, "Pause", true),
        (AudioCommand::Stop, "Stop", true),
        (AudioCommand::StopTransport, "StopTransport", true),
        (AudioCommand::Seek { sample: 0 }, "Seek", true),
        (
            AudioCommand::LoadModel { model: model() },
            "LoadModel",
            true,
        ),
        (
            AudioCommand::ReloadNotes { model: model() },
            "ReloadNotes",
            false,
        ),
        (
            AudioCommand::UpdateNotes { model: model() },
            "UpdateNotes",
            false,
        ),
        (
            AudioCommand::SetSoundFonts {
                configs: Box::new(Vec::new()),
            },
            "SetSoundFonts",
            true,
        ),
        (
            AudioCommand::SkipTracks { skip: Vec::new() },
            "SkipTracks",
            false,
        ),
        (
            AudioCommand::SetAmMs {
                am_ms: Arc::new(AmMsMap::new()),
            },
            "SetAmMs",
            false,
        ),
        (
            AudioCommand::SetLayerCount { count: None },
            "SetLayerCount",
            true,
        ),
        (
            AudioCommand::SetIgnoreVelocity { threshold: 0 },
            "SetIgnoreVelocity",
            true,
        ),
        (
            AudioCommand::SetMaxVoices { max: None },
            "SetMaxVoices",
            true,
        ),
        (
            AudioCommand::SetAutomationDensity { density: 1 },
            "SetAutomationDensity",
            true,
        ),
        (
            AudioCommand::PreviewNotes {
                notes: Vec::new(),
                exclusive: false,
            },
            "PreviewNotes",
            false,
        ),
        (AudioCommand::PreviewStop, "PreviewStop", false),
        (
            AudioCommand::PreviewStopKey { key: 60 },
            "PreviewStopKey",
            false,
        ),
        (
            AudioCommand::PreviewInstrumentNotes {
                channel: 0,
                notes: Vec::new(),
                exclusive: false,
            },
            "PreviewInstrumentNotes",
            false,
        ),
        (
            AudioCommand::PreviewInstrumentStop {
                channel: None,
                key: None,
            },
            "PreviewInstrumentStop",
            false,
        ),
        (
            AudioCommand::SetMixerParams {
                params: Box::new(MixerParams::default()),
            },
            "SetMixerParams",
            true,
        ),
        (
            AudioCommand::SetNodeStrip {
                node: MixerChannelId::Midi(0),
                params: StripParams::default(),
            },
            "SetNodeStrip",
            false,
        ),
        (
            AudioCommand::SetMasterParams {
                params: MasterParams::default(),
            },
            "SetMasterParams",
            false,
        ),
        (
            AudioCommand::InsertAdd {
                target: InsertTarget::Master,
                slot: 0,
                processor: insert(),
            },
            "InsertAdd",
            true,
        ),
        (
            AudioCommand::InsertRemove {
                target: InsertTarget::Master,
                slot: 0,
            },
            "InsertRemove",
            true,
        ),
        (
            AudioCommand::InsertReplace {
                target: InsertTarget::Master,
                slot: 0,
                processor: insert(),
            },
            "InsertReplace",
            true,
        ),
        (
            AudioCommand::SetInstrumentSlot {
                target: InsertTarget::Midi(0),
                slot_id: 0,
                processor: None,
            },
            "SetInstrumentSlot",
            true,
        ),
        (
            AudioCommand::SetAudioSource {
                uuid: String::new(),
                decoded: Arc::new(crate::audio_source::DecodedAudio {
                    sample_rate: 0,
                    frames: 0,
                    left: Arc::from(Vec::<f32>::new()),
                    right: Arc::from(Vec::<f32>::new()),
                    peaks: crate::audio_source::WavePeaks {
                        base_bucket: 0,
                        levels: Vec::new(),
                    },
                }),
            },
            "SetAudioSource",
            false,
        ),
        (AudioCommand::RefreshLatency, "RefreshLatency", false),
        (
            AudioCommand::ExportStart {
                path: std::path::PathBuf::new(),
                bit_depth: crate::export::WavBitDepth::Bit16,
                layer_count: None,
                restore_layer_count: None,
                progress: crate::export::ExportProgress::new(),
                cancel: Arc::new(AtomicBool::new(false)),
                pause: Arc::new(AtomicBool::new(false)),
            },
            "ExportStart",
            false,
        ),
        (
            AudioCommand::FreezeStart {
                node: MixerChannelId::Midi(0),
                progress: crate::freeze::FreezeProgress::new(),
                cancel: Arc::new(AtomicBool::new(false)),
            },
            "FreezeStart",
            true,
        ),
    ];
    for (cmd, name, reliable) in &cases {
        let meta = cmd.meta();
        assert_eq!(meta.name, *name, "命令诊断名不符");
        assert_eq!(meta.reliable, *reliable, "{name} 的可靠通道分类不符");
    }
}

#[test]
fn negotiate_buffer_size_clamps_fixed_into_device_range() {
    let range = Some((64, 1024));
    assert_eq!(
        negotiate_buffer_size(BufferSize::Fixed(4096), range),
        BufferSize::Fixed(1024)
    );
    assert_eq!(
        negotiate_buffer_size(BufferSize::Fixed(16), range),
        BufferSize::Fixed(64)
    );
    assert_eq!(
        negotiate_buffer_size(BufferSize::Fixed(512), range),
        BufferSize::Fixed(512)
    );
}

#[test]
fn negotiate_buffer_size_keeps_default_and_unknown_support() {
    let range = Some((64, 1024));
    assert_eq!(
        negotiate_buffer_size(BufferSize::Default, range),
        BufferSize::Default
    );
    assert_eq!(
        negotiate_buffer_size(BufferSize::Fixed(4096), None),
        BufferSize::Fixed(4096)
    );
}

#[test]
fn negotiate_sample_rate_falls_back_outside_supported_ranges() {
    let ranges = [(44100u32, 48000u32), (88200, 96000)];
    assert_eq!(negotiate_sample_rate_in(48000, 44100, Some(&ranges)), 48000);
    assert_eq!(negotiate_sample_rate_in(96000, 44100, Some(&ranges)), 96000);
    assert_eq!(
        negotiate_sample_rate_in(192000, 44100, Some(&ranges)),
        44100
    );
    assert_eq!(negotiate_sample_rate_in(22050, 44100, Some(&[])), 44100);
}

#[test]
fn negotiate_sample_rate_keeps_request_when_ranges_unknown() {
    // 枚举失败（None）不做判断，交给 cpal 建流时报错。
    assert_eq!(negotiate_sample_rate_in(22050, 44100, None), 22050);
}

/// 命令合并语义：连续同类型只保留最新（latest 覆盖先前命令），
/// 异类命令回填 pending 防饿死，且不丢弃。
#[test]
fn merge_pending_commands_keeps_latest_and_defers_others() {
    let (tx, rx) = unbounded::<WorkerCmd>();
    let m_old = Arc::new(yinhe_core::YinModel::default());
    let m_new = Arc::new(yinhe_core::YinModel::default());
    tx.send(WorkerCmd::PrepareModel(Arc::clone(&m_old), 1))
        .unwrap();
    tx.send(WorkerCmd::PrepareModel(Arc::clone(&m_new), 2))
        .unwrap();
    tx.send(WorkerCmd::PrepareNotes(Arc::clone(&m_old)))
        .unwrap();

    let mut pending = std::collections::VecDeque::new();
    let (latest, density) = merge_pending_commands(
        &rx,
        &mut pending,
        (Arc::clone(&m_old), 0u32),
        |cmd| match cmd {
            WorkerCmd::PrepareModel(m, d) => Ok((m, d)),
            other => Err(other),
        },
    );

    assert!(
        Arc::ptr_eq(&latest, &m_new),
        "latest 必须被最新的 PrepareModel 覆盖"
    );
    assert_eq!(density, 2);
    assert_eq!(pending.len(), 1, "异类命令应回填 pending");
    assert!(matches!(pending[0], WorkerCmd::PrepareNotes(..)));
    assert!(rx.is_empty(), "合并应取尽通道内命令");
}
