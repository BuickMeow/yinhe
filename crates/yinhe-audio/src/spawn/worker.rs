use super::*;

pub(crate) fn merge_pending_commands<T>(
    cmd_rx: &crossbeam_channel::Receiver<WorkerCmd>,
    pending: &mut std::collections::VecDeque<WorkerCmd>,
    initial: T,
    mut try_merge: impl FnMut(WorkerCmd) -> Result<T, WorkerCmd>,
) -> T {
    let mut latest = initial;
    while let Ok(next) = cmd_rx.try_recv() {
        match try_merge(next) {
            Ok(merged) => latest = merged,
            Err(other) => pending.push_back(other),
        }
    }
    latest
}

pub(crate) fn spawn_worker(
    sample_rate: u32,
    interpolation: Interpolation,
    sf_gen: Arc<AtomicU64>,
    sf_progress: Arc<AtomicU32>,
) -> Result<(Sender<WorkerCmd>, crossbeam_channel::Receiver<WorkerResult>), std::io::Error> {
    let (cmd_tx, cmd_rx) = unbounded::<WorkerCmd>();
    let (result_tx, result_rx) = bounded::<WorkerResult>(1);

    thread::Builder::new()
        .name("audio-worker".into())
        .spawn(move || {
            yinhe_synth::denormals::enable_flush_denormals();
            // 内部 pending 缓冲：处理某个命令时，try_recv 到的非同类型命令存这里。
            // 下次循环优先从 pending 取，避免饿死后续命令。
            let mut pending: std::collections::VecDeque<WorkerCmd> =
                std::collections::VecDeque::new();
            loop {
                let cmd = match pending.pop_front() {
                    Some(c) => c,
                    None => match cmd_rx.recv() {
                        Ok(c) => c,
                        Err(_) => break,
                    },
                };
                match cmd {
                    WorkerCmd::PrepareModel(model, density) => {
                        // 合并连续 PrepareModel，只保留最新
                        let (latest, latest_density) = merge_pending_commands(
                            &cmd_rx,
                            &mut pending,
                            (model, density),
                            |cmd| match cmd {
                                WorkerCmd::PrepareModel(m, d) => Ok((m, d)),
                                other => Err(other),
                            },
                        );
                        let t_prepare = std::time::Instant::now();
                        let prepared = crate::prepare_model::prepare_model(
                            &latest,
                            sample_rate,
                            latest_density,
                        );
                        crate::audio_renderer::play_log(&format!(
                            "[play] worker prepare_model={:?}",
                            t_prepare.elapsed()
                        ));
                        let _ = result_tx.send(WorkerResult::PreparedModel(prepared));
                        // 构建 cc_events 的临时内存已释放：归还空闲页，
                        // 避免 RSS 跨阶段累积（大工程峰值内存只涨不跌的根因之一）。
                        yinhe_memtrace::purge_free_pages();
                    }
                    WorkerCmd::PrepareNotes(model) => {
                        // 合并连续 PrepareNotes，只保留最新
                        let (latest,) = merge_pending_commands(
                            &cmd_rx,
                            &mut pending,
                            (model,),
                            |cmd| match cmd {
                                WorkerCmd::PrepareNotes(m) => Ok((m,)),
                                other => Err(other),
                            },
                        );
                        let (audio_model, yin_model, duration_samples) =
                            crate::prepare_model::prepare_notes(&latest, sample_rate);
                        let _ = result_tx.send(WorkerResult::PreparedNotes {
                            model: audio_model,
                            yin_model,
                            duration_samples,
                        });
                        yinhe_memtrace::purge_free_pages();
                    }
                    WorkerCmd::PrepareChase {
                        model,
                        target_tick,
                        generation,
                        purpose,
                        skip_mask,
                        am_ms,
                    } => {
                        // 合并连续 PrepareChase，只保留最新（同 generation 或不同 generation 都只留最新）
                        let (
                            latest_model,
                            latest_target,
                            latest_gen,
                            latest_mask,
                            latest_am_ms,
                            latest_purpose,
                        ) = merge_pending_commands(
                            &cmd_rx,
                            &mut pending,
                            (model, target_tick, generation, skip_mask, am_ms, purpose),
                            |cmd| match cmd {
                                WorkerCmd::PrepareChase {
                                    model,
                                    target_tick,
                                    generation,
                                    skip_mask,
                                    am_ms,
                                    purpose,
                                } => {
                                    Ok((model, target_tick, generation, skip_mask, am_ms, purpose))
                                }
                                other => Err(other),
                            },
                        );
                        let (states, plugin_params) = compute_chase_states(
                            &latest_model,
                            latest_target,
                            &latest_mask,
                            &latest_am_ms,
                        );
                        let _ = result_tx.send(WorkerResult::ChaseResult {
                            states,
                            plugin_params,
                            generation: latest_gen,
                            purpose: latest_purpose,
                            target_tick: latest_target,
                        });
                    }
                    WorkerCmd::LoadSoundFont {
                        channels,
                        paths,
                        prefetch_keymaps,
                        generation,
                    } => {
                        // 不合并，但把 try_recv 到的命令存到 pending 避免饿死
                        while let Ok(next) = cmd_rx.try_recv() {
                            pending.push_back(next);
                        }
                        // latest-wins：切到更新的音色库后，队列里的旧代次直接跳过
                        // （仍回传空结果，让 renderer 的进度计数前进）。
                        if generation != sf_gen.load(Ordering::Acquire) {
                            crate::audio_renderer::play_log(&format!(
                                "[sf] worker 跳过旧代次 generation={generation} current={}",
                                sf_gen.load(Ordering::Relaxed)
                            ));
                            let _ = result_tx.send(WorkerResult::LoadedSoundFont {
                                channels,
                                paths,
                                generation,
                            });
                            continue;
                        }
                        // yinhe 路径的 key map 解析（重采样 SFZ，未命中缓存时 3-4s）
                        // 移到 worker：音频线程随后只做缓存命中 + 登记。
                        if prefetch_keymaps {
                            let t_prefetch = std::time::Instant::now();
                            sf_progress.store(50, Ordering::Relaxed);
                            let n = paths.len().max(1);
                            for (i, path) in paths.iter().enumerate() {
                                let base = 50 + (450 * i / n) as u32;
                                let span = (450 / n) as f32;
                                if let Err(e) = yinhe_synth::prefetch_key_maps_with_progress(
                                    std::path::Path::new(path),
                                    sample_rate,
                                    interpolation.code(),
                                    &mut |f| {
                                        // SFZ 逐 region / SF2 解析阶段：映射进本路径区间。
                                        let p = base as f32 + f.clamp(0.0, 1.0) * span;
                                        sf_progress.store(p as u32, Ordering::Relaxed);
                                    },
                                ) {
                                    eprintln!("[gpu] worker 预解析失败 {path}: {e}");
                                }
                                sf_progress
                                    .store(50 + (450 * (i + 1) / n) as u32, Ordering::Relaxed);
                            }
                            crate::audio_renderer::play_log(&format!(
                                "[play] worker 预解析 GPU key map={:?}",
                                t_prefetch.elapsed()
                            ));
                            sf_progress.store(500, Ordering::Relaxed);
                        }
                        // 预解析可能耗时几秒：完成后再检查一次代次，过期就不回传
                        // 实际音色（但仍回传空结果维持进度计数）。
                        if generation != sf_gen.load(Ordering::Acquire) {
                            crate::audio_renderer::play_log(&format!(
                                "[sf] worker 跳过旧代次 generation={generation} current={}",
                                sf_gen.load(Ordering::Relaxed)
                            ));
                            let _ = result_tx.send(WorkerResult::LoadedSoundFont {
                                channels,
                                paths,
                                generation,
                            });
                            continue;
                        }
                        // 方案 C：只有 YinheCpu，不加载 xsynth 版音色库；
                        // 仅回传 paths 供主引擎登记 key map。
                        sf_progress.store(900, Ordering::Relaxed);
                        let _ = result_tx.send(WorkerResult::LoadedSoundFont {
                            channels,
                            paths,
                            generation,
                        });
                    }
                }
            }
        })
        .map_err(|e| {
            tracing::error!("Failed to spawn audio worker thread: {e}");
            e
        })?;

    Ok((cmd_tx, result_rx))
}

pub(crate) type ChaseOutcome = (Box<[Option<ChannelState>; 256]>, Vec<(u8, u64, u32, f32)>);

pub(crate) fn compute_chase_states(
    model: &YinModel,
    target_tick: u32,
    skip_mask: &[bool],
    am_ms: &crate::spawn::AmMsMap,
) -> ChaseOutcome {
    use crate::audio_model::{emit_automation_event, push_program_change};

    // 每通道收集目标位置生效事件（tick 排序后顺序 apply，多 track 同 channel 自动合并）。
    let mut events: [Vec<SortedCC>; 256] = std::array::from_fn(|_| Vec::new());
    // 插件参数 chase：目标位置生效的 (乐器通道, 乐器 uid, param_id, 归一化值)。
    let mut plugin_params: Vec<(u8, u64, u32, f32)> = Vec::new();

    for (track_idx, track) in model.tracks.iter().enumerate() {
        if skip_mask.get(track_idx).copied().unwrap_or(false) {
            continue; // mute 轨道不参与 chase
        }
        let Some(ch) = track.midi_target_channel() else {
            continue;
        };
        let ch = ch as usize;
        if ch >= 256 {
            continue;
        }
        let out = &mut events[ch];
        // 与播放事件流一致：该音轨是否有任意 lane solo（S 作用域 = 音轨内）。
        let track_has_solo = track.automation_lanes.iter().any(|l| {
            am_ms
                .get(&(track_idx as u16, l.target.clone()))
                .is_some_and(|s| s.solo)
        });
        for (lane_idx, lane) in track.automation_lanes.iter().enumerate() {
            if crate::audio_model::automation_lane_skipped(
                am_ms,
                track_idx as u16,
                lane,
                track_has_solo,
            ) {
                continue;
            }
            // 第三方乐器插件参数：不进 MIDI 通道状态（占位事件会污染 CC0），单独收集。
            if let yinhe_types::AutomationTarget::Param {
                device: yinhe_types::automation::ParamDevice::PluginInstrument { channel, uid },
                id,
                ..
            } = &lane.target
            {
                if let Some((value, _)) = lane.value_at(target_tick) {
                    plugin_params.push((*channel, *uid, *id, value));
                }
                continue;
            }
            if let Some((value, tick)) = lane.value_at(target_tick) {
                emit_automation_event(
                    &lane.target,
                    value,
                    tick,
                    ch as u32,
                    track_idx as u16,
                    lane_idx as u16,
                    out,
                );
            }
        }
        for pc in &track.program_change {
            if pc.tick < target_tick {
                push_program_change(pc, ch as u32, track_idx as u16, out);
            }
        }
    }

    let mut states: Box<[Option<ChannelState>; 256]> = Box::new(std::array::from_fn(|_| None));
    for (ch, mut evs) in events.into_iter().enumerate() {
        if evs.is_empty() {
            // 无事件：不触碰该通道（chase 应用时跳过），
            // 避免把 mute 轨的通道控制器重置回默认值。
            continue;
        }
        // 与播放事件流一致：同 tick 参数类（RPN/CC/PC）先于 PitchBendValue；
        // 同 tick 同通道同目标再取 max（与 dispatch/GPU 的聚合同一规则）。
        evs.sort_by_key(|e| (e.tick, crate::audio_model::dispatch_priority(&e.event)));
        let reps = crate::audio_model::aggregate_max_reps(&evs, |_| true);
        let mut state = ChannelState::default();
        for &i in &reps {
            state.apply(&evs[i].event);
        }
        states[ch] = Some(state);
    }

    // 插件参数：同 (通道, uid, 参数) 取 max（与 dispatch 的插件参数聚合一致）。
    let mut plugin_params_agg: Vec<(u8, u64, u32, f32)> = Vec::new();
    for (ch, uid, param_id, value) in plugin_params {
        match plugin_params_agg
            .iter_mut()
            .find(|p| p.0 == ch && p.1 == uid && p.2 == param_id)
        {
            Some(p) => {
                if value > p.3 {
                    p.3 = value;
                }
            }
            None => plugin_params_agg.push((ch, uid, param_id, value)),
        }
    }
    (states, plugin_params_agg)
}

#[cfg(test)]
pub(crate) fn compute_chase_states_for_test(
    model: &YinModel,
    target_tick: u32,
    skip_mask: &[bool],
) -> Box<[Option<ChannelState>; 256]> {
    compute_chase_states(model, target_tick, skip_mask, &crate::spawn::AmMsMap::new()).0
}

#[cfg(test)]
pub(crate) fn compute_chase_states_with_am_ms_for_test(
    model: &YinModel,
    target_tick: u32,
    skip_mask: &[bool],
    am_ms: &crate::spawn::AmMsMap,
) -> Box<[Option<ChannelState>; 256]> {
    compute_chase_states(model, target_tick, skip_mask, am_ms).0
}
