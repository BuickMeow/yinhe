use super::*;

/// 全局浮层：插件选择器 + 参数面板。
///
/// 从 MIX 视图的 `show` 里拆出（三视图通用：底部设备栏在 AR/EDIT 下也能打开
/// 选择器与参数面板），由 main_loop 的 overlay 阶段统一调用。
pub(crate) fn show_global_overlays(app: &mut App, ctx: &egui::Context) {
    let Some(idx) = app.workspace.active_doc else {
        return;
    };
    let mut actions: Vec<MixAction> = Vec::new();
    if let Some(target) = app.mix.picker_for {
        strip::plugin_picker(app, ctx, target, &mut actions);
    }
    if let Some(ch) = app.mix.instrument_picker_for {
        strip::instrument_picker(app, ctx, ch, &mut actions);
    }
    for action in actions {
        actions::apply_action(app, ctx, idx, action);
    }
    param_panel::show(app, ctx);
    crate::dialogs::instrument_params::show(app, ctx);
}

/// insert 目标的持久化链（越界/不存在返回 None）。
pub(crate) fn insert_refs(
    mixer: &mut yinhe_mixer::MixerParams,
    target: InsertTarget,
) -> Option<&mut Vec<yinhe_mixer::InsertRef>> {
    mixer.node_inserts_mut(target)
}

/// 移除混音台里所有内置效果器（`PluginFormat::Builtin`）insert（迁移）。
/// 返回是否有移除：无移除时调用方不应标脏（打开旧工程不该弹保存）。
pub(crate) fn strip_builtin_inserts(mixer: &mut yinhe_mixer::MixerParams) -> bool {
    use yinhe_mixer::PluginFormat;
    let mut removed = false;
    for node in mixer.nodes_mut() {
        let before = node.inserts.len();
        node.inserts.retain(|r| r.format != PluginFormat::Builtin);
        removed |= node.inserts.len() != before;
    }
    let before = mixer.master.inserts.len();
    mixer
        .master
        .inserts
        .retain(|r| r.format != PluginFormat::Builtin);
    removed |= mixer.master.inserts.len() != before;
    removed
}

/// 给通道挂载一个乐器插件（新建音轨对话框 / MixAction 共用）。
pub(crate) fn assign_instrument_plugin(
    app: &mut App,
    idx: usize,
    channel: u8,
    plugin: &PluginEntry,
) {
    let uid;
    {
        let doc = &mut app.workspace.documents[idx];
        let m = doc.mixer_mut();
        m.ensure_len();
        uid = m.alloc_instrument_uid();
        m.racks[channel as usize]
            .instruments
            .push(yinhe_mixer::InsertRef {
                uid,
                plugin_path: plugin.path.clone(),
                plugin_id: plugin.id.clone(),
                name: plugin.name.clone(),
                format: plugin.format,
                bypassed: false,
                state: None,
            });
    }
    if idx < app.instrument_racks.len() {
        let hosts = app.workspace.documents[idx].instrument_hosts(channel);
        let rack = &mut app.instrument_racks[idx];
        for lc in &hosts {
            if let Err(e) = rack.add(
                channel,
                *lc,
                uid,
                plugin.format,
                &plugin.path,
                &plugin.id,
                &plugin.name,
                None,
            ) {
                rack.last_error = Some(e.0);
            }
        }
    }
    app.workspace.documents[idx].data.bump_revision();
    app.push_mixer_state_to_engine(idx);
}

/// 启动插件扫描子进程（不阻塞 UI）；已有扫描进行中则忽略。
pub(crate) fn start_plugin_scan(app: &mut App) {
    if app.mix.scan_in_progress {
        return;
    }
    match crate::plugin_scan::spawn_scan_worker() {
        Some(rx) => {
            app.mix.scan_rx = Some(rx);
            app.mix.scan_in_progress = true;
            app.mix.scanned = None;
            app.mix.scan_errors = 0;
        }
        None => {
            // worker 起不来也标记"扫描完成（空结果）"，否则每帧重试。
            tracing::warn!("启动插件扫描子进程失败");
            app.mix.scanned = Some(Vec::new());
        }
    }
}

/// 每帧轮询扫描子进程结果。
pub(crate) fn poll_plugin_scan(app: &mut App) {
    let Some(rx) = &app.mix.scan_rx else {
        return;
    };
    match rx.try_recv() {
        Ok(ScanProgress::Batch(entries)) => {
            let scanned = app.mix.scanned.get_or_insert_with(Vec::new);
            scanned.extend(entries);
            scanned.sort_by(|a, b| a.name.cmp(&b.name));
        }
        Ok(ScanProgress::Finished { errors }) => {
            app.mix.scan_errors = errors;
            app.mix.scan_rx = None;
            app.mix.scan_in_progress = false;
            // 零结果（无插件目录/全部未产出条目）也要落 Some，
            // 否则"scanned.is_none() 才扫描"的条件会每帧重扫。
            app.mix.scanned.get_or_insert_with(Vec::new);
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => {}
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            app.mix.scan_rx = None;
            app.mix.scan_in_progress = false;
        }
    }
}
