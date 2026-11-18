use super::*;

pub type InsertTarget = yinhe_types::MixerChannelId;

pub type AmMsMap =
    std::collections::HashMap<(u16, yinhe_types::AutomationTarget), yinhe_types::AmMsState>;

pub enum AudioCommand {
    Play {
        from_sample: u64,
    },
    Resume,
    Pause,
    /// 硬停止：停走带 + 复位到 0，并重置混音/插件状态（加载、导出、teardown 用）。
    Stop,
    /// 用户停止按钮：停走带 + 复位到 0，但保留 insert/DSP 状态，让尾音自然衰减。
    StopTransport,
    Seek {
        sample: u64,
    },
    /// 设置走带循环区间（tick 域；`end_tick <= start_tick` 表示关闭）。
    /// 引擎在块内跨过 end 时回绕到 start；状态的恢复见 `AudioEngine::set_loop`。
    SetLoop {
        enabled: bool,
        start_tick: u32,
        end_tick: u32,
    },
    LoadModel {
        model: Arc<YinModel>,
    },
    /// Like LoadModel but does NOT stop playback.
    /// Replaces the model reference and resets note cursors.
    /// Full rebuild: cc_events (+ chase).
    /// Used for automation edits / undo / redo / arrange drag (notes+automation).
    ReloadNotes {
        model: Arc<YinModel>,
    },
    /// Only swap the model reference — no CC rebuild, no chase.
    /// Used for pure note edits (move/drag/add/delete/paste/duplicate/transpose)
    /// where automation lanes are untouched. Keeps current playback position and
    /// channel state intact, only repositions note cursors for future dispatch.
    UpdateNotes {
        model: Arc<YinModel>,
    },
    /// 设置某全局通道的内置音源参数（工程级覆盖，实时生效）。
    SetSynthParams {
        channel: u8,
        params: yinhe_types::SynthParams,
    },
    /// 全量设置各源通道（0..256）的音色库（批量；只含需要加载的非空配置）。
    /// 每个通道独立；未列出的通道保持当前音色库（通常是全局默认）。
    SetSoundFonts {
        configs: Box<Vec<(u8, Vec<String>)>>,
    },
    /// `skip[i] == true` means track i is hidden (not audible).
    SkipTracks {
        skip: Vec<bool>,
    },
    /// AR 自动化 lane 的 M/S 试听旁通集：只更新 dispatch 动态掩码 + 异步 chase，
    /// 不重建模型、不 seek（与 SkipTracks 同机制）。
    SetAmMs {
        am_ms: Arc<AmMsMap>,
    },
    /// Set per-key layer count (None = unlimited).
    SetLayerCount {
        count: Option<usize>,
    },
    /// 忽略力度 ≤ threshold 的音符（0 = 不忽略）。dispatch 时实时判断，立即生效。
    SetIgnoreVelocity {
        threshold: u8,
    },
    /// Set global max voices (None = automatic / system recommended).
    SetMaxVoices {
        max: Option<usize>,
    },
    /// Set automation Linear/Curve intermediate event density (tick interval).
    /// Triggers a cc_events rebuild if a model is loaded.
    SetAutomationDensity {
        density: u32,
    },
    /// 音符听觉预览：替换旧组的待触发音符；已在响的旧组音符继续响满自己的 gate
    /// （快速拖拽时若立即 NoteOff，音符会在被渲染前死掉，永远听不到）。
    /// 每组音符应用目标位置自动化状态并 NoteOn。停止由 PreviewStop（释放）负责。
    /// 预览走独立合成器（PreviewEngine）：不占主引擎 voice、不改播放状态。
    PreviewNotes {
        notes: Vec<PreviewNoteParams>,
        /// 替换模式（PR 拖动/铅笔）：先停掉旧组全部预览音再触发新组，
        /// 避免拖动时多个音叠加；false（MIDI 直通）= 叠加式，和弦保持。
        exclusive: bool,
    },
    /// 停止全部预览音（余音自然衰减完才停）。
    PreviewStop,
    /// 停止单个键的预览音（MIDI 直通单键 NoteOff：只停松开的键，和弦保持）。
    PreviewStopKey {
        key: u8,
    },
    /// 乐器插件预览：把音符直接送进 MIDI 通道上挂载的插件实例（停止状态下空闲渲染出声）。
    /// 与 `PreviewNotes`（xsynth 预览）并列：挂了插件的通道预览走这条，音色与播放一致。
    /// 组替换语义与 `PreviewNotes` 一致（替换旧组待触发音符；已响的保留）。
    PreviewInstrumentNotes {
        /// MIDI 全局通道（0..256，与 `TrackData::global_channel()` 对齐）。
        channel: u8,
        notes: Vec<InstrumentPreviewNote>,
        /// 替换模式：先停掉该通道旧组全部预览音再触发新组（PR 拖动不叠加）。
        exclusive: bool,
    },
    /// 停止乐器插件预览。`channel` 限定 MIDI 通道（None = 全部）；
    /// `key` 限定单个键（None = 全部，MIDI 直通单键停止用）。
    PreviewInstrumentStop {
        channel: Option<u8>,
        key: Option<u8>,
    },
    /// 全量同步混音台参数（引擎 spawn / 工程加载后由 UI 推一次；Box 避免命令枚举过大）。
    SetMixerParams {
        params: Box<MixerParams>,
    },
    /// 更新任意混音节点的 strip 参数（推子拖动高频路径，幂等）。
    /// MIDI / 音频 / 每轨 LC / Master 走同一个入口，引擎内部再映射到 dense 通道。
    SetNodeStrip {
        node: MixerChannelId,
        params: StripParams,
    },
    /// 更新主输出参数。
    SetMasterParams {
        params: MasterParams,
    },
    /// 在 target 指定位置 insert 链的 slot 处插入处理器。
    InsertAdd {
        target: InsertTarget,
        slot: usize,
        processor: Box<dyn InsertProcessor>,
    },
    /// 移除 target 位置 insert 链 slot 处的处理器（经 return 通道送回 UI 回收）。
    InsertRemove {
        target: InsertTarget,
        slot: usize,
    },
    /// 替换 target 位置 insert 链 slot 处的处理器（插件 restart 用）。
    InsertReplace {
        target: InsertTarget,
        slot: usize,
        processor: Box<dyn InsertProcessor>,
    },
    /// 安装/替换/移除某乐器宿主的插件实例（CLAP/VST3 等，抽象为 trait）。
    ///
    /// `target`：
    /// - `Midi(ch)` = AP 共享链（同一 MIDI 通道的所有 AP 轨共享一条插件链）；
    /// - `TrackLc(id)` = OwnLc 每轨实例（插件链落在该轨 home LC 的 dense）。
    ///
    /// `Some(processor)` = 安装/替换；`None` = 移除（空链后回到内置音源）。
    /// 被替换/移除的旧处理器经乐器 return 通道送回 UI 线程 deactivate。
    SetInstrumentSlot {
        target: InsertTarget,
        /// UI 侧分配的单槽位 id（同一宿主内唯一）。
        slot_id: u64,
        /// `None` = 移除该槽位。处理器较大（含渲染缓冲），用 Box 避免枚举体积膨胀。
        processor: Option<Box<dyn InstrumentProcessor>>,
    },
    /// 安装/替换已解码音频素材（uuid → PCM）。UI 后台线程解码后推送；
    /// 音频轨片段按 uuid 引用。素材池只增不减。
    SetAudioSource {
        uuid: String,
        decoded: Arc<crate::audio_source::DecodedAudio>,
    },
    /// 插件延迟变化（CLAP latency changed / VST3 kLatencyChanged）：
    /// 重新查询各处理器延迟并重算 PDC 对齐。
    RefreshLatency,
    /// 开始导出：渲染线程进入导出模式——停止播放、复位到 0，用**当前引擎**
    /// （含全部 insert/乐器插件与 PDC）从头离线渲染到 WAV。
    /// 进度/暂停/取消经共享 Arc 通信；完成后 `progress.finished` 置位。
    ExportStart {
        path: std::path::PathBuf,
        bit_depth: crate::export::WavBitDepth,
        /// xsynth 层数限制（None = 跟随当前设置）。
        layer_count: Option<usize>,
        /// 导出结束后恢复的层数（= UI 当前设置换算值；导出不改用户设置）。
        restore_layer_count: Option<usize>,
        progress: Arc<Mutex<crate::export::ExportProgress>>,
        cancel: Arc<AtomicBool>,
        pause: Arc<AtomicBool>,
    },
    /// 开始冻结一条 LC：离线渲染整条时间轴，只捕获该 LC 的 pre-LC 干声。
    /// 结果经 `progress.result` 交回 UI（UI 负责写入工程并设置节点 freeze）。
    FreezeStart {
        node: MixerChannelId,
        progress: Arc<Mutex<crate::freeze::FreezeProgress>>,
        cancel: Arc<AtomicBool>,
    },
}

pub struct PreviewNoteParams {
    /// 全局通道（port<<4 | channel）。
    pub channel: u8,
    pub key: u8,
    pub velocity: u8,
    /// 目标位置 tick：该处的自动化状态（volume/pan/PBS/Program 等）用于预览。
    pub target_tick: u32,
    /// 预览时长（tick）；0 = 持续音（等 `PreviewStop`）。
    pub duration_ticks: u32,
}

pub struct InstrumentPreviewNote {
    /// 通道内 MIDI 通道（低 4 位，与播放路径一致）。
    pub midi_channel: u8,
    pub key: u8,
    /// 1 ~ 127（与编辑层一致）。
    pub velocity: u8,
    /// 目标位置 tick：组内音符按此差错开触发（与 xsynth 预览一致）。
    pub target_tick: u32,
    /// 预览时长（tick）；0 = 持续音（等 `PreviewInstrumentStop`）。
    pub duration_ticks: u32,
}

pub(crate) struct CmdMeta {
    /// 可靠命令（走独立无界通道，保序、永不丢）。
    ///
    /// - 传输控制：UI 时钟对齐依赖，丢失会造成播放/暂停状态错位。
    /// - 结构性变更（insert 挂载/移除、乐器挂载、通道结构）：单次命令、无
    ///   重发路径（UI 发完即标记已发送），丢失即永久不一致（曾因此效果器
    ///   在渲染线程阻塞 4-5 秒时被丢，卡片在 UI 上但引擎里根本没挂上）。
    /// - 设置类（层数/复音数/力度忽略/自动化密度）：同属单次命令、无重发路径，
    ///   丢弃后 UI 不再发（diff 已归零）→ 设置永不生效、用户以为"必须重启"。
    /// - `LoadModel`：同样单次、无重发（spawn/过户/切文档时各发一次）；
    ///   丢失后模型永不加载 → Play 永远挂起等待（engine.model_loaded() 为
    ///   false）而 UI 只看到"播放无响应"。命令通道仅由渲染线程短暂阻塞填满。
    pub(crate) reliable: bool,
    /// 诊断日志用类型名。
    pub(crate) name: &'static str,
}

impl AudioCommand {
    /// 命令分类的**唯一**入口：一次 match 同时推导可靠性与诊断名。
    ///
    /// 刻意不写 `_` 通配分支：新增 `AudioCommand` 变体时编译器强制在此
    /// 声明分类，杜绝此前 `is_reliable` / `cmd_kind` 两份 match 靠人工同步、
    /// 新命令漏进可靠通道（或类型名落 "Other"）的隐患。
    pub(crate) fn meta(&self) -> CmdMeta {
        let (name, reliable) = match self {
            AudioCommand::Play { .. } => ("Play", true),
            AudioCommand::Resume => ("Resume", true),
            AudioCommand::Pause => ("Pause", true),
            AudioCommand::Stop => ("Stop", true),
            AudioCommand::StopTransport => ("StopTransport", true),
            AudioCommand::Seek { .. } => ("Seek", true),
            AudioCommand::SetLoop { .. } => ("SetLoop", true),
            AudioCommand::LoadModel { .. } => ("LoadModel", true),
            AudioCommand::ReloadNotes { .. } => ("ReloadNotes", false),
            AudioCommand::UpdateNotes { .. } => ("UpdateNotes", false),
            AudioCommand::SetSoundFonts { .. } => ("SetSoundFonts", true),
            AudioCommand::SetSynthParams { .. } => ("SetSynthParams", false),
            AudioCommand::SkipTracks { .. } => ("SkipTracks", false),
            AudioCommand::SetAmMs { .. } => ("SetAmMs", false),
            AudioCommand::SetLayerCount { .. } => ("SetLayerCount", true),
            AudioCommand::SetIgnoreVelocity { .. } => ("SetIgnoreVelocity", true),
            AudioCommand::SetMaxVoices { .. } => ("SetMaxVoices", true),
            AudioCommand::SetAutomationDensity { .. } => ("SetAutomationDensity", true),
            AudioCommand::PreviewNotes { .. } => ("PreviewNotes", false),
            AudioCommand::PreviewStop => ("PreviewStop", false),
            AudioCommand::PreviewStopKey { .. } => ("PreviewStopKey", false),
            AudioCommand::PreviewInstrumentNotes { .. } => ("PreviewInstrumentNotes", false),
            AudioCommand::PreviewInstrumentStop { .. } => ("PreviewInstrumentStop", false),
            AudioCommand::SetMixerParams { .. } => ("SetMixerParams", true),
            AudioCommand::SetNodeStrip { .. } => ("SetNodeStrip", false),
            AudioCommand::SetMasterParams { .. } => ("SetMasterParams", false),
            AudioCommand::InsertAdd { .. } => ("InsertAdd", true),
            AudioCommand::InsertRemove { .. } => ("InsertRemove", true),
            AudioCommand::InsertReplace { .. } => ("InsertReplace", true),
            AudioCommand::SetInstrumentSlot { .. } => ("SetInstrumentSlot", true),
            AudioCommand::SetAudioSource { .. } => ("SetAudioSource", false),
            AudioCommand::RefreshLatency => ("RefreshLatency", false),
            AudioCommand::ExportStart { .. } => ("ExportStart", false),
            AudioCommand::FreezeStart { .. } => ("FreezeStart", true),
        };
        CmdMeta { reliable, name }
    }

    /// 是否走可靠（无界、保序、永不丢）通道。
    pub(crate) fn is_reliable(&self) -> bool {
        self.meta().reliable
    }

    /// 诊断日志用类型名。
    pub(crate) fn kind(&self) -> &'static str {
        self.meta().name
    }
}

pub(crate) enum ChasePurpose {
    Seek,
    Loop,
}

pub(crate) enum WorkerCmd {
    /// Full prepare: cc_events + duration (LoadModel / ReloadNotes).
    PrepareModel(Arc<YinModel>, u32),
    /// Notes-only prepare: 换模型引用 + duration（UpdateNotes），不重建 cc_events。
    PrepareNotes(Arc<YinModel>),
    /// Compute channel-state snapshot at `target_tick` by **querying the model
    /// automation lanes**（每 lane 二分 + 曲线实时插值，不再从曲首逐条累计）。
    /// `generation` matches `AudioEngine::chase_generation` so the renderer can
    /// discard stale results after a PrepareModel replaces the model.
    PrepareChase {
        model: Arc<YinModel>,
        target_tick: u32,
        generation: u64,
        /// Seek = 立即应用；Loop = 存为 loop 起点快照，回绕时同步应用。
        purpose: ChasePurpose,
        /// 当前 skip_track 快照，chase 时跳过 mute 轨道的 CC。
        skip_mask: Vec<bool>,
        /// AR 自动化 lane 的 M/S 试听旁通，chase 时同步过滤。
        am_ms: Arc<AmMsMap>,
    },
    LoadSoundFont {
        /// 共享同一音色库集合的源通道（按 paths 分组后一条命令多个通道）。
        channels: Vec<u8>,
        paths: Vec<String>,
        /// 在 worker 里预热 yinhe-synth 的进程级解析缓存（GPU 与 CPU 后端共用）：
        /// 音频线程随后命中缓存，不在音频线程解析（400MB 级音色库 3-6s）。
        prefetch_keymaps: bool,
        /// 音色库切换代次；worker 开始/回传时对比，旧代次直接跳过。
        generation: u64,
    },
}

pub(crate) enum WorkerResult {
    PreparedModel(PreparedModel),
    /// Result of `PrepareNotes` — 模型引用 + 派生快照 + duration。
    PreparedNotes {
        model: crate::audio_model::AudioModel,
        yin_model: Arc<YinModel>,
        duration_samples: u64,
    },
    /// Result of `PrepareChase` — 256-channel state snapshot.
    /// `Some(state)` = 该通道在目标位置有生效事件（无事件通道不触碰）。
    /// `plugin_params`：插件参数 chase 值（MIDI 通道, 乐器 uid, param_id, 归一化值）。
    ChaseResult {
        states: Box<[Option<ChannelState>; 256]>,
        plugin_params: Vec<(u8, u64, u32, f32)>,
        generation: u64,
        purpose: ChasePurpose,
        target_tick: u32,
    },
    LoadedSoundFont {
        channels: Vec<u8>,
        /// 原始路径列表 — 供音源/诊断定位音色库
        paths: Vec<String>,
        /// 音色库切换代次（renderer 丢弃过期结果）。
        generation: u64,
    },
}
