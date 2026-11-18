use super::*;

pub(crate) struct EngineInit {
    engine: crate::engine::AudioEngine,
    meters: MixerMeters,
    /// 渲染线程 → UI 的 insert 处理器退回通道（替换/移除/拆除时回收 deactivate）。
    insert_return_tx: Sender<Vec<Box<dyn InsertProcessor>>>,
    insert_return_rx: crossbeam_channel::Receiver<Vec<Box<dyn InsertProcessor>>>,
    /// 渲染线程 → UI 的乐器处理器退回通道（替换/移除/拆除时回收 deactivate）。
    instrument_return_tx: Sender<(u64, Box<dyn InstrumentProcessor>)>,
    instrument_return_rx: crossbeam_channel::Receiver<(u64, Box<dyn InstrumentProcessor>)>,
}

pub(crate) fn init_engine(
    sample_rate: u32,
    layout: ChannelLayout,
    interpolation: Interpolation,
    synth_engine: SynthEngine,
) -> Result<EngineInit, String> {
    let engine = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut engine = crate::engine::AudioEngine::new(sample_rate, layout);
        engine.set_interpolation(interpolation);
        // 预览后端跟随主引擎：XSynthCpu → xsynth；YinheCpu → yinhe CPU。
        let preview = crate::preview_engine::PreviewEngine::new(
            synth_engine,
            &engine.channel_layout,
            engine.sample_rate,
            interpolation.code(),
        );
        engine.set_preview_engine(preview);
        engine
    })) {
        Ok(engine) => engine,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<String>()
                .map(|s| s.as_str())
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("unknown panic");
            return Err(format!("Audio engine initialization failed: {msg}"));
        }
    };
    let meters = MixerMeters::collect(&engine);
    let (insert_return_tx, insert_return_rx) = unbounded::<Vec<Box<dyn InsertProcessor>>>();
    let (instrument_return_tx, instrument_return_rx) =
        unbounded::<(u64, Box<dyn InstrumentProcessor>)>();
    Ok(EngineInit {
        engine,
        meters,
        insert_return_tx,
        insert_return_rx,
        instrument_return_tx,
        instrument_return_rx,
    })
}

pub(crate) struct StreamCallback {
    ring: crate::audio_ring::AudioRingConsumer,
    reset_generation: Arc<AtomicU64>,
    clear_ring_write: Arc<AtomicUsize>,
    clear_base_sample: Arc<AtomicU64>,
    underrun_samples: Arc<AtomicU64>,
    /// 回调把 renderer 侧播放状态/位置/时长镜像到 UI 侧原子量。
    playing: Arc<AtomicBool>,
    renderer_playing: Arc<AtomicBool>,
    sample_position: Arc<AtomicU64>,
    /// 循环回绕累计绝对偏移：回调上报 `consumer_abs - offset`，回绕后播放
    /// 位置才不会永久领先/落后一个 ring。
    loop_wrap_offset: Arc<AtomicU64>,
    duration_samples: Arc<AtomicU64>,
    renderer_duration: Arc<AtomicU64>,
    /// 录音监听缓冲（交错立体声）：输入回调 push、输出回调混入。
    monitor: Arc<Mutex<std::collections::VecDeque<f32>>>,
}

pub(crate) struct RendererStarted {
    handle: std::thread::JoinHandle<()>,
    /// UI 播放指示线的上限：渲染器已推入 ring 的采样位置（producer）。
    producer_sample_position: Arc<AtomicU64>,
    /// 循环回绕累计绝对偏移（见 `RendererSharedState::loop_wrap_offset`）。
    loop_wrap_offset: Arc<AtomicU64>,
    /// seek/reload 等清空 ring 的代次（UI 用它重置表历史）。
    reset_generation: Arc<AtomicU64>,
    /// 活跃 voice 数（含 release 尾音）。
    voice_count: Arc<AtomicU64>,
    /// 已完成加载的音色库 port 数。
    sf_loaded: Arc<AtomicUsize>,
    /// 音色库切换进度（latest-wins 代次的 loading/done/total）。
    sf_loading: Arc<AtomicBool>,
    sf_total: Arc<AtomicUsize>,
    sf_done: Arc<AtomicUsize>,
    /// 音色库加载细粒度进度（0..1000）。
    sf_progress: Arc<AtomicU32>,
    /// 音频完全就绪（模型 + 音色库 + 采样上传 + 管线预热）。
    audio_ready: Arc<AtomicBool>,
    stream_callback: StreamCallback,
    monitor: Arc<Mutex<std::collections::VecDeque<f32>>>,
}

#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub(crate) fn init_renderer(
    engine: crate::engine::AudioEngine,
    cmd_rx: crossbeam_channel::Receiver<AudioCommand>,
    transport_rx: crossbeam_channel::Receiver<AudioCommand>,
    worker_tx: Sender<WorkerCmd>,
    prepared_rx: crossbeam_channel::Receiver<WorkerResult>,
    shutdown: Arc<AtomicBool>,
    preview_stop_flag: Arc<AtomicBool>,
    sample_position: Arc<AtomicU64>,
    playing: Arc<AtomicBool>,
    duration_samples: Arc<AtomicU64>,
    pending_skip: Arc<Mutex<Option<Vec<bool>>>>,
    pending_am_ms: Arc<Mutex<Option<Arc<AmMsMap>>>>,
    sf_gen: Arc<AtomicU64>,
    sf_progress: Arc<AtomicU32>,
    callback_frames: usize,
    insert_return_tx: Sender<Vec<Box<dyn InsertProcessor>>>,
    instrument_return_tx: Sender<(u64, Box<dyn InstrumentProcessor>)>,
    synth_engine: SynthEngine,
    interpolation: Interpolation,
) -> Result<RendererStarted, String> {
    let (ring_producer, ring_consumer) = AudioRing::new(RING_CAPACITY).split();
    let mut renderer_state = RendererSharedState::new();
    renderer_state.sf.generation = sf_gen;
    renderer_state.sf.progress = sf_progress;

    let producer_sample_position = Arc::clone(&renderer_state.playback.producer_sample_position);
    let loop_wrap_offset = Arc::clone(&renderer_state.playback.loop_wrap_offset);
    let reset_generation = Arc::clone(&renderer_state.playback.reset_generation);
    let sf_loaded = Arc::clone(&renderer_state.sf.loaded);
    let sf_loading = Arc::clone(&renderer_state.sf.loading);
    let sf_total = Arc::clone(&renderer_state.sf.total);
    let sf_done = Arc::clone(&renderer_state.sf.done);
    let sf_progress = Arc::clone(&renderer_state.sf.progress);
    let audio_ready = Arc::clone(&renderer_state.sf.audio_ready);
    let voice_count = Arc::clone(&renderer_state.playback.voice_count);
    // 监听缓冲：容量上限由输入侧（yinhe-egui 的录音回调）控制，
    // 保证监听延迟不随录音时长增长。
    let monitor: Arc<Mutex<std::collections::VecDeque<f32>>> =
        Arc::new(Mutex::new(std::collections::VecDeque::new()));
    let stream_callback = StreamCallback {
        ring: ring_consumer,
        reset_generation: Arc::clone(&renderer_state.playback.reset_generation),
        // 清空边界：ack 时丢弃边界前的旧音频、保留新音频（竞态安全清空）。
        clear_ring_write: Arc::clone(&renderer_state.playback.clear_ring_write),
        clear_base_sample: Arc::clone(&renderer_state.playback.clear_base_sample),
        // 欠载计数（cpal 回调写；renderer_state 即将 move，先 clone）。
        underrun_samples: Arc::clone(&renderer_state.underrun_samples),
        playing,
        renderer_playing: Arc::clone(&renderer_state.playback.playing),
        sample_position: Arc::clone(&sample_position),
        loop_wrap_offset: Arc::clone(&renderer_state.playback.loop_wrap_offset),
        duration_samples,
        renderer_duration: Arc::clone(&renderer_state.playback.duration_samples),
        monitor: Arc::clone(&monitor),
    };

    let handle = spawn_renderer(
        engine,
        ring_producer,
        renderer_state,
        cmd_rx,
        transport_rx,
        worker_tx,
        prepared_rx,
        shutdown,
        preview_stop_flag,
        sample_position,
        pending_skip,
        pending_am_ms,
        callback_frames,
        insert_return_tx,
        instrument_return_tx,
        synth_engine,
        interpolation,
    )
    .map_err(|e| format!("Failed to spawn audio renderer thread: {e}"))?;

    Ok(RendererStarted {
        handle,
        producer_sample_position,
        loop_wrap_offset,
        reset_generation,
        voice_count,
        sf_loaded,
        sf_loading,
        sf_total,
        sf_done,
        sf_progress,
        audio_ready,
        stream_callback,
        monitor,
    })
}

pub(crate) fn build_output_stream(
    device: &Device,
    format: OutputFormat,
    buffer_size: BufferSize,
    callback: StreamCallback,
    stream_holder_weak: std::sync::Weak<Mutex<Option<Stream>>>,
    stream_error: Arc<AtomicBool>,
) -> Result<Stream, String> {
    let StreamCallback {
        mut ring,
        reset_generation,
        clear_ring_write,
        clear_base_sample,
        underrun_samples,
        playing,
        renderer_playing,
        sample_position,
        loop_wrap_offset,
        duration_samples,
        renderer_duration,
        monitor,
    } = callback;
    let mut consumer_sample_position = 0u64;
    let mut acknowledged_generation = 0u64;

    device.build_output(
        format,
        buffer_size,
        move |data: &mut [f32]| {
            yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Audio, || {
                let generation = reset_generation.load(Ordering::Acquire);
                if generation != acknowledged_generation {
                    // 丢弃边界前的旧音频，保留边界后渲染器已推入的新音频
                    // （整体 clear 会把新播放位置的开头一起丢掉）。
                    ring.discard_before(clear_ring_write.load(Ordering::Acquire));
                    consumer_sample_position = clear_base_sample.load(Ordering::Acquire);
                    acknowledged_generation = generation;
                }

                // ring 有音频就读，没有就填静音（含未加载模型时的纯预览场景）。
                let popped = ring.pop_into(data);
                if popped < data.len() {
                    data[popped..].fill(0.0);
                    // 欠载计数：仅播放中（未播放时 ring 空是正常静音）。
                    if playing.load(Ordering::Relaxed) {
                        underrun_samples.fetch_add((data.len() - popped) as u64, Ordering::Relaxed);
                    }
                }
                consumer_sample_position =
                    consumer_sample_position.saturating_add((popped / STEREO_CHANNELS) as u64);

                // 监听混入（try_lock：失败跳过，绝不阻塞实时回调）。
                if let Ok(mut mon) = monitor.try_lock()
                    && !mon.is_empty()
                {
                    let n = mon.len().min(data.len());
                    for (dst, src) in data.iter_mut().zip(mon.drain(..n)) {
                        *dst += src;
                    }
                }

                // 把“已消费绝对样本”换算成引擎逻辑位置（循环回绕后回退）。
                let offset = loop_wrap_offset.load(Ordering::Acquire);
                sample_position.store(
                    consumer_sample_position.saturating_sub(offset),
                    Ordering::Relaxed,
                );
                playing.store(renderer_playing.load(Ordering::Relaxed), Ordering::Relaxed);
                duration_samples
                    .store(renderer_duration.load(Ordering::Relaxed), Ordering::Relaxed);
            })
        },
        move |err| {
            let is_rate_change = err.kind == StreamErrorKind::StreamInvalidated
                && err
                    .message
                    .as_deref()
                    .is_some_and(|m| m.contains("sample rate changed"));
            if is_rate_change {
                // cpal 已把流暂停；设备还活着，CoreAudio 会做 SRC，恢复即可。
                tracing::warn!("Audio stream paused by device sample rate change, resuming: {err}");
                if let Some(holder) = stream_holder_weak.upgrade() {
                    let guard = holder.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(stream) = guard.as_ref()
                        && let Err(e) = stream.play()
                    {
                        tracing::error!("Failed to resume audio stream: {e}");
                        stream_error.store(true, Ordering::Release);
                    }
                }
            } else {
                tracing::error!(
                    "Audio stream error: {}",
                    err.message.as_deref().unwrap_or("(no message)")
                );
                stream_error.store(true, Ordering::Release);
            }
        },
    )
}

pub(crate) fn start_output_stream(
    stream_holder: &Arc<Mutex<Option<Stream>>>,
    stream: Stream,
) -> Result<(), String> {
    *stream_holder.lock().unwrap_or_else(|e| e.into_inner()) = Some(stream);
    match stream_holder
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|s| s.play())
    {
        Some(Ok(())) => Ok(()),
        Some(Err(e)) => Err(format!("Failed to start stream: {e}")),
        None => Err("Audio stream unexpectedly missing after build".to_string()),
    }
}

pub fn spawn_cpal_audio(
    sample_rate: u32,
    layout: ChannelLayout,
    buffer_size: BufferSize,
    device_name: Option<&str>,
    synth_engine: SynthEngine,
    interpolation: Interpolation,
) -> Result<CpalAudioHandle, String> {
    let synth_engine = resolve_synth_engine(synth_engine);

    let (cmd_tx, cmd_rx) = bounded::<AudioCommand>(AUDIO_CMD_CHANNEL_CAPACITY);
    let (transport_tx, transport_rx) = unbounded::<AudioCommand>();
    let sample_position = Arc::new(AtomicU64::new(0));
    let playing = Arc::new(AtomicBool::new(false));
    let duration_samples = Arc::new(AtomicU64::new(0));
    let stream_error = Arc::new(AtomicBool::new(false));
    let preview_stop_flag = Arc::new(AtomicBool::new(false));

    let device = select_output_device(device_name)?;
    let supported = device.default_format()?;
    // 强制立体声：xsynth 是立体声合成器，ring buffer 和位置计算也硬编码 2 声道。
    // 不取设备默认声道数，避免多声道设备（HDMI/聚合设备 6/8 声道）导致声道映射错乱。
    let channels = STEREO_CHANNELS;

    // 设备能力协商：蓝牙设备（尤其 HFP 模式）的缓冲区/采样率范围往往比
    // 内置扬声器窄很多（实测小米开放式耳机缓冲区仅 14..=1024 帧）。
    // 直接把用户设置塞给 build_output_stream 会失败，而失败只进日志、
    // UI 无感知，表现为"播放键失灵、时间线冻结"。这里先钳制/回退。
    let sample_rate = negotiate_sample_rate(&device, sample_rate, supported.sample_rate);
    let buffer_size = negotiate_buffer_size(buffer_size, device.buffer_range());

    let format = OutputFormat {
        channels: channels as u16,
        sample_rate,
    };
    // cpal 回调每次请求的帧数（约等于流缓冲）：预览时 ring 目标必须 ≥ 它，
    // 否则回调只能 pop 出部分帧、其余填静音 → 每个回调周期后半段静音 → 声音
    // "一闪一闪"地卡顿。Default（设备默认缓冲）时帧数未知，保守按 1024 帧估算
    // （常见设备与蓝牙 HFP 上限均 ≤ 1024）。
    let callback_frames = match buffer_size {
        BufferSize::Fixed(n) => n as usize,
        BufferSize::Default => 1024,
    };

    let EngineInit {
        engine,
        meters,
        insert_return_tx,
        insert_return_rx,
        instrument_return_tx,
        instrument_return_rx,
    } = init_engine(sample_rate, layout, interpolation, synth_engine)?;

    let sf_gen: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
    let sf_progress: Arc<AtomicU32> = Arc::new(AtomicU32::new(0));
    let (worker_tx, prepared_rx) = spawn_worker(
        sample_rate,
        interpolation,
        Arc::clone(&sf_gen),
        Arc::clone(&sf_progress),
    )
    .map_err(|e| format!("Failed to spawn audio worker thread: {e}"))?;

    let shutdown = Arc::new(AtomicBool::new(false));
    // latest-wins 槽：M/S 掩码必达（UI 写、renderer 每轮消费最新值）。
    let pending_skip: Arc<Mutex<Option<Vec<bool>>>> = Arc::new(Mutex::new(None));
    let pending_am_ms: Arc<Mutex<Option<Arc<AmMsMap>>>> = Arc::new(Mutex::new(None));
    let RendererStarted {
        handle: renderer_handle,
        producer_sample_position,
        loop_wrap_offset,
        reset_generation,
        voice_count,
        sf_loaded,
        sf_loading,
        sf_total,
        sf_done,
        sf_progress,
        audio_ready,
        stream_callback,
        monitor,
    } = init_renderer(
        engine,
        cmd_rx,
        transport_rx,
        worker_tx,
        prepared_rx,
        Arc::clone(&shutdown),
        Arc::clone(&preview_stop_flag),
        Arc::clone(&sample_position),
        Arc::clone(&playing),
        Arc::clone(&duration_samples),
        Arc::clone(&pending_skip),
        Arc::clone(&pending_am_ms),
        Arc::clone(&sf_gen),
        Arc::clone(&sf_progress),
        callback_frames,
        insert_return_tx,
        instrument_return_tx,
        synth_engine,
        interpolation,
    )?;

    // 流在回调注册之后才创建，用 Arc<Mutex<Option>> 共享给错误回调；
    // 回调只持 Weak，handle 释放时不会形成循环引用。
    let stream_holder: Arc<Mutex<Option<Stream>>> = Arc::new(Mutex::new(None));
    let stream = match build_output_stream(
        &device,
        format,
        buffer_size,
        stream_callback,
        Arc::downgrade(&stream_holder),
        Arc::clone(&stream_error),
    ) {
        Ok(stream) => stream,
        Err(e) => {
            // build stream 失败 —— 清理已 spawn 的 renderer 线程，避免泄漏
            shutdown.store(true, Ordering::Release);
            let _ = renderer_handle.join();
            return Err(e);
        }
    };
    if let Err(e) = start_output_stream(&stream_holder, stream) {
        shutdown.store(true, Ordering::Release);
        let _ = renderer_handle.join();
        return Err(e);
    }

    Ok(CpalAudioHandle {
        synth_engine,
        handle: AudioHandle {
            cmd_tx,
            transport_tx,
            sample_position,
            producer_sample_position,
            loop_wrap_offset,
            reset_generation,
            playing,
            voice_count,
            duration_samples,
            stream_error,
            preview_stop_flag,
            sf_loaded,
            sf_loading,
            sf_total,
            sf_done,
            sf_progress,
            audio_ready,
            pending_skip,
            pending_am_ms,
            meters,
            insert_return_rx,
            instrument_return_rx,
        },
        sample_rate,
        monitor,
        _stream: stream_holder,
        shutdown,
        renderer_handle: Some(renderer_handle),
    })
}
