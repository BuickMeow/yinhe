use super::*;

pub(crate) struct MixerMeters {
    /// 各 dense 通道的读数端。
    channels: Vec<MeterReading>,
    /// 主输出读数端。
    master: MeterReading,
    /// 主输出立体声样本（供电平/频谱分析）。
    master_samples: SampleReading,
    /// 每轨电平读数端（线性峰值，渲染线程写、UI 读）。
    tracks: Arc<Mutex<Vec<(f32, f32)>>>,
}

impl MixerMeters {
    /// 从引擎收集全部读数端。引擎随后 move 进渲染线程；layout 冻结，
    /// 通道数不变（resize 只重建缓冲不换 meter），读数端整个引擎生命周期有效。
    pub(crate) fn collect(engine: &crate::engine::AudioEngine) -> Self {
        Self {
            channels: (0..engine.mix.mixer.channel_count())
                .filter_map(|i| engine.mix.mixer.channel_meter_reading(i))
                .collect(),
            master: engine.mix.mixer.master_meter_reading(),
            master_samples: engine.mix.mixer.master_sample_reading(),
            tracks: engine.track_levels(),
        }
    }

    /// dense 通道数（= 引擎 compacted 通道数）。
    pub(crate) fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// 读某 dense 通道的电平（L, R 峰值，0 起）。
    pub(crate) fn channel_read(&self, dense: usize) -> Option<(f32, f32)> {
        self.channels.get(dense).map(|r| r.read())
    }

    /// 读主输出电平。
    pub(crate) fn master_read(&self) -> (f32, f32) {
        self.master.read()
    }

    /// 取走主输出样本（交错立体声）。
    pub(crate) fn master_samples_drain(&self, out: &mut Vec<f32>) -> usize {
        self.master_samples.drain_into(out)
    }

    /// 读某轨电平（线性峰值；不存在返回 0）。
    pub(crate) fn track_read(&self, track: usize) -> (f32, f32) {
        self.tracks
            .lock()
            .ok()
            .and_then(|g| g.get(track).copied())
            .unwrap_or((0.0, 0.0))
    }
}

pub struct AudioHandle {
    pub(crate) cmd_tx: Sender<AudioCommand>,
    /// 传输命令专用无界通道：Play/Resume/Pause/Stop/Seek 保序、永不丢。
    /// 普通命令通道（容量 16）在渲染线程忙（音色库加载/事件重建）时打满，
    /// 若 Play 被丢弃，UI 会永久 pending、指示线不动。
    pub(crate) transport_tx: Sender<AudioCommand>,
    pub(crate) sample_position: Arc<AtomicU64>,
    /// 渲染线程已产出（推入 ring）的采样位置。UI 用它限制播放指示线
    /// 不能超过实际已渲染的音频（防止"准备播放"期间指示线空跑）。
    pub(crate) producer_sample_position: Arc<AtomicU64>,
    /// 循环回绕累计绝对偏移（UI 据此把回绕初期的位置钳到循环起点）。
    pub(crate) loop_wrap_offset: Arc<AtomicU64>,
    /// seek/reload 等清空 ring 的代次（主输出表重置历史用）。
    pub(crate) reset_generation: Arc<AtomicU64>,
    pub(crate) playing: Arc<AtomicBool>,
    /// 活跃 voice 数（含 release 尾音）：播放到末尾后用它判断何时暂停。
    pub(crate) voice_count: Arc<AtomicU64>,
    pub(crate) duration_samples: Arc<AtomicU64>,
    /// 由 cpal 流错误回调置位。UI 每帧查询，若为 true 应弹窗提示用户重启。
    pub(crate) stream_error: Arc<AtomicBool>,
    /// 预览 Stop 快速路径标志：渲染线程每轮消费。命令通道满（渲染忙时
    /// `PreviewStop` 命令可能被丢弃）也保证松手即停。
    pub(crate) preview_stop_flag: Arc<AtomicBool>,
    /// 已完成加载的音色库 port 数（worker 每完成一 port +1）。
    pub(crate) sf_loaded: Arc<AtomicUsize>,
    /// 音色库切换进度（当前代次）：loading / done / total。
    pub(crate) sf_loading: Arc<AtomicBool>,
    pub(crate) sf_total: Arc<AtomicUsize>,
    pub(crate) sf_done: Arc<AtomicUsize>,
    /// 音色库加载细粒度进度（0..1000）。
    pub(crate) sf_progress: Arc<AtomicU32>,
    /// 音频完全就绪（模型 + 音色库 + 采样上传 + 管线预热）。
    pub(crate) audio_ready: Arc<AtomicBool>,
    /// latest-wins：轨道 mute/solo 掩码必达（命令通道满合并时不丢最新值）。
    pub(crate) pending_skip: Arc<Mutex<Option<Vec<bool>>>>,
    /// latest-wins：AM lane M/S 试听旁通集必达。
    pub(crate) pending_am_ms: Arc<Mutex<Option<Arc<AmMsMap>>>>,
    /// 混音台电平表读数端。
    pub(crate) meters: MixerMeters,
    /// 渲染线程退回的 insert 处理器（插件 deactivate 必须在 UI/管理线程做）。
    pub(crate) insert_return_rx: crossbeam_channel::Receiver<Vec<Box<dyn InsertProcessor>>>,
    /// 渲染线程退回的乐器处理器（deactivate 同样必须在 UI/管理线程做）。
    pub(crate) instrument_return_rx:
        crossbeam_channel::Receiver<(u64, Box<dyn InstrumentProcessor>)>,
}

impl AudioHandle {
    /// 发命令给 renderer 线程。
    ///
    /// 可靠命令（传输控制 + 结构性变更，见 [`AudioCommand::is_reliable`]）走独立
    /// 无界通道，保序永不丢。其余走容量 `AUDIO_CMD_CHANNEL_CAPACITY`（16）的通道，
    /// 满时 `try_send` 失败 → 丢弃新命令 + `warn!` 日志，绝不阻塞 UI 线程。
    /// - `Full`：renderer 处理不过来。renderer 已对 `ReloadNotes`/`UpdateNotes`
    ///   做同类型合并，worker 对 `PrepareModel`/`PrepareNotes`/`PrepareChase`
    ///   也做合并，因此偶发丢弃只造成短暂 UI/音频错位，下一次操作即重新同步。
    /// - `Disconnected`：renderer 线程已退出。仅记日志，不 panic ——
    ///   渲染线程死亡不应该让 UI 也跟着崩。
    pub fn send(&self, cmd: AudioCommand) {
        if cmd.is_reliable() {
            // 无界通道：除渲染线程退出外不会失败。
            let _ = self.transport_tx.send(cmd);
            return;
        }
        match self.cmd_tx.try_send(cmd) {
            Ok(()) => {}
            Err(crossbeam_channel::TrySendError::Full(cmd)) => {
                let kind = cmd.kind();
                tracing::warn!("AudioHandle::send: channel full, dropping {kind}");
                crate::audio_renderer::play_log(&format!("[play] 命令通道满，丢弃 {kind}"));
            }
            Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                tracing::warn!("AudioHandle::send: channel disconnected, dropping command");
            }
        }
    }

    /// 请求立即停止全部预览音。不依赖命令通道（渲染忙时通道满会丢命令），
    /// 由渲染线程每轮消费标志，保证松手即停。
    pub fn request_preview_stop(&self) {
        self.preview_stop_flag.store(true, Ordering::Release);
    }

    /// latest-wins：轨道 mute/solo 掩码必达。命令通道满合并时 `SkipTracks`
    /// 命令可能被丢弃，本槽保证 renderer 每轮总能拿到**最新**掩码。
    pub fn set_skip_tracks(&self, skip: Vec<bool>) {
        *self.pending_skip.lock().unwrap_or_else(|e| e.into_inner()) = Some(skip);
    }

    /// latest-wins：AM lane M/S 旁通集必达（见 [`Self::set_skip_tracks`]）。
    pub fn set_am_ms(&self, am_ms: Arc<AmMsMap>) {
        *self.pending_am_ms.lock().unwrap_or_else(|e| e.into_inner()) = Some(am_ms);
    }

    /// 开始新预览组时清除待消费的停止请求，避免新组被渲染器当作
    /// "松手后的堆积旧组"跳过。
    pub fn clear_preview_stop(&self) {
        self.preview_stop_flag.store(false, Ordering::Release);
    }

    pub fn sample_position(&self) -> u64 {
        self.sample_position.load(Ordering::Relaxed)
    }

    /// 渲染线程已推入 ring 的采样位置（producer），总是 `>= sample_position()`。
    ///
    /// 播放指示线以此为上限：Play/Seek 清空 ring 后、首个 chunk 渲染完成前
    /// 没有可听的音频，指示线必须停在当前位置等待，不能按墙钟空跑。
    pub fn producer_sample_position(&self) -> u64 {
        self.producer_sample_position.load(Ordering::Relaxed)
    }

    /// 循环回绕累计偏移。>0 表示已经发生过回绕；回绕初期消费位置换算后
    /// 会短暂小于循环起点，UI 应把播放头钳在起点等待声音追上。
    pub fn loop_wrap_offset(&self) -> u64 {
        self.loop_wrap_offset.load(Ordering::Relaxed)
    }

    pub fn sample_position_arc(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.sample_position)
    }

    pub fn playing_arc(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.playing)
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    /// 当前活跃 voice 数（含 release 尾音；0 = 声音已自然结束）。
    pub fn voice_count(&self) -> u64 {
        self.voice_count.load(Ordering::Relaxed)
    }

    pub fn duration_samples(&self) -> u64 {
        self.duration_samples.load(Ordering::Relaxed)
    }

    /// 查询 cpal 流是否已报错（设备热拔、驱动崩溃等）。
    /// 一旦置位就不会清零，UI 应弹出"需要重启"对话框。
    pub fn stream_error(&self) -> bool {
        self.stream_error.load(Ordering::Relaxed)
    }

    /// 已完成加载的音色库 port 数（`LoadSoundFont` 结果回传时 +1）。
    pub fn sf_loaded_count(&self) -> usize {
        self.sf_loaded.load(Ordering::Relaxed)
    }

    /// 音色库切换进度：`(loading, done, total, progress 0..1)`。
    pub fn sf_progress(&self) -> (bool, usize, usize, f32) {
        (
            self.sf_loading.load(Ordering::Relaxed),
            self.sf_done.load(Ordering::Relaxed),
            self.sf_total.load(Ordering::Relaxed),
            f32::from(self.sf_progress.load(Ordering::Relaxed) as u16) / 1000.0,
        )
    }

    /// 音频是否完全就绪（模型 + 音色库 + 采样上传 + 管线预热）。
    pub fn audio_ready(&self) -> bool {
        self.audio_ready.load(Ordering::Relaxed)
    }

    /// 混音台 dense 通道数（= 引擎 compacted 通道数）。
    pub fn mixer_channel_count(&self) -> usize {
        self.meters.channel_count()
    }

    /// 读某 dense 通道的电平（L, R 峰值，0 起）。
    pub fn channel_meter_read(&self, dense: usize) -> Option<(f32, f32)> {
        self.meters.channel_read(dense)
    }

    /// 读主输出电平。
    pub fn master_meter_read(&self) -> (f32, f32) {
        self.meters.master_read()
    }

    /// 取走主输出样本（交错立体声）。UI 每帧 drain 一次。
    pub fn master_samples_drain(&self, out: &mut Vec<f32>) -> usize {
        self.meters.master_samples_drain(out)
    }

    /// 读某轨电平 L/R（线性峰值，仅播放时非零；不存在返回 0）。
    pub fn track_meter_read(&self, track: usize) -> (f32, f32) {
        self.meters.track_read(track)
    }

    /// seek/reload 等清空 ring 的代次；变化表示表历史应重置。
    pub fn reset_generation(&self) -> u64 {
        self.reset_generation.load(Ordering::Relaxed)
    }

    /// 取回渲染线程退回的 insert 处理器（每帧轮询；插件 deactivate 在 UI 线程做）。
    pub fn drain_insert_returns(&self) -> Vec<Box<dyn InsertProcessor>> {
        let mut out = Vec::new();
        while let Ok(mut batch) = self.insert_return_rx.try_recv() {
            out.append(&mut batch);
        }
        out
    }

    /// 取回渲染线程退回的乐器处理器（每帧轮询；deactivate 在 UI 线程做）。
    pub fn drain_instrument_returns(&self) -> Vec<(u64, Box<dyn InstrumentProcessor>)> {
        let mut out = Vec::new();
        while let Ok(p) = self.instrument_return_rx.try_recv() {
            out.push(p);
        }
        out
    }

    /// 克隆退回通道的接收端：teardown 时先克隆、再 drop 句柄（join 渲染线程），
    /// 之后仍能从克隆的接收端取回渲染线程关机时退回的全部处理器。
    pub fn clone_insert_return_rx(
        &self,
    ) -> crossbeam_channel::Receiver<Vec<Box<dyn InsertProcessor>>> {
        self.insert_return_rx.clone()
    }

    /// 克隆乐器退回通道接收端（同 `clone_insert_return_rx` 的用途）。
    pub fn clone_instrument_return_rx(
        &self,
    ) -> crossbeam_channel::Receiver<(u64, Box<dyn InstrumentProcessor>)> {
        self.instrument_return_rx.clone()
    }
}

pub struct CpalAudioHandle {
    pub handle: AudioHandle,
    pub sample_rate: u32,
    /// 当前引擎使用的合成器后端（UI 据此限制 per-track 输出）。
    pub synth_engine: SynthEngine,
    /// 录音监听缓冲（交错立体声）：输入回调 push、输出回调混入。
    /// 上限见 `MONITOR_MAX_SAMPLES`，超限丢最旧样本（延迟不累积）。
    pub monitor: Arc<Mutex<std::collections::VecDeque<f32>>>,
    /// 共享给 cpal 错误回调（采样率变化时恢复流）。用 Arc<Mutex<Option>> 而非裸
    /// Stream：错误回调在流创建之前就注册，只能通过共享句柄访问。回调只持 Weak，
    /// handle drop 时 Arc 归零 → Stream 正常释放，不形成循环引用。
    pub(crate) _stream: Arc<Mutex<Option<Stream>>>,
    /// 设置为 true 时通知 renderer 线程退出。
    pub(crate) shutdown: Arc<AtomicBool>,
    /// renderer 线程的 JoinHandle。
    ///
    /// Drop 时同步 join，确保 AudioEngine（含 rayon 线程池）完全释放。
    /// 不 join 会导致反复 teardown+rebuild（如蓝牙耳机抖动触发设备切换）时
    /// rayon 工作线程累积，最终触发 EAGAIN (code 35) panic。
    ///
    /// 注意：渲染线程退出需释放 GPU 资源并 `purge_free_pages`（jemalloc
    /// 归还大块采样缓冲给 OS），join 可能耗时数百 ms——调用方（UI）应把
    /// drop 放到后台线程，避免卡住主线程。
    pub(crate) renderer_handle: Option<std::thread::JoinHandle<()>>,
}

impl CpalAudioHandle {
    /// 同步暂停输出流（立即静音，不销毁流）。teardown 在把句柄丢后台线程
    /// drop 之前调用：drop 里的 join 可能数百 ms，期间旧流仍会播完 ring 里
    /// 的残余音频，而新引擎可能已在另一线程 build+play（双流重叠的"滋"）。
    /// pause 在调用线程同步完成，瞬时可返回。
    pub fn pause_stream(&self) {
        if let Ok(guard) = self._stream.lock()
            && let Some(stream) = guard.as_ref()
            && let Err(e) = stream.pause()
        {
            tracing::debug!("Failed to pause audio stream: {e}");
        }
    }

    /// Notify the audio thread that the MIDI model has changed (full rebuild:
    /// cc_events + chase). Use for automation edits / undo / redo.
    /// AM lane M/S 旁通独立于模型存在，由 [`Self::set_am_ms`] 维护。
    pub fn reload_notes(&self, model: Arc<YinModel>) {
        self.handle.send(AudioCommand::ReloadNotes { model });
    }

    /// 更新 AR 自动化 lane 的 M/S 试听旁通集：只换动态掩码 + 异步 chase，
    /// 不重建模型、不 seek——与 SkipTracks 对轨道 mute 的处理同机制。
    /// 走 latest-wins 槽（通道满也不丢最新状态）。
    pub fn set_am_ms(&self, am_ms: Arc<AmMsMap>) {
        self.handle.set_am_ms(am_ms);
    }

    /// 轨道 mute/solo 掩码（latest-wins 槽，见 [`Self::set_am_ms`]）。
    pub fn set_skip_tracks(&self, skip: Vec<bool>) {
        self.handle.set_skip_tracks(skip);
    }

    /// Notify the audio thread that only notes have changed (no automation, no
    /// chase). Use for pure note edits — keeps current channel state intact.
    pub fn update_notes(&self, model: Arc<YinModel>) {
        self.handle.send(AudioCommand::UpdateNotes { model });
    }
}

impl Drop for CpalAudioHandle {
    fn drop(&mut self) {
        // 先暂停输出流：join 期间旧流仍在放音（ring 里最多 ~85ms），而调用方
        // 通常已在后台线程 rebuild 新引擎——新流 build+play 会与旧流短暂重叠
        //（设备被二次配置/双流混音），听感是一声"滋"。pause 立即静音
        //（不销毁流），消除新旧流的重叠窗口。
        if let Ok(guard) = self._stream.lock()
            && let Some(stream) = guard.as_ref()
            && let Err(e) = stream.pause()
        {
            tracing::debug!("Failed to pause audio stream during teardown: {e}");
        }
        self.shutdown.store(true, Ordering::Release);
        // 同步 join renderer 线程。WAKE_SLEEP=1ms，renderer 最多 1ms 后退出，
        // join 阻塞时间可忽略。确保 AudioEngine → ChannelGroup → 2 个 rayon::ThreadPool
        // 在返回前被 drop，避免线程泄漏。
        if let Some(handle) = self.renderer_handle.take()
            && let Err(payload) = handle.join()
        {
            tracing::error!("Audio renderer thread panicked during shutdown: {payload:?}");
        }
    }
}
