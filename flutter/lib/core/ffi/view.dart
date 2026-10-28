part of '../ffi.dart';

/// tick → 秒（长按微调时时间码预显示用）。
final TickToSecondsDart secondsAtTick = bridge
    .lookupFunction<TickToSecondsNative, TickToSecondsDart>(
      'yinhe_seconds_at_tick',
    );

typedef TickAtXNative = ffi.Double Function(ffi.Float);
typedef TickAtXDart = double Function(double);

/// 画布逻辑 x → 当前视图 tick（量化吸附；标尺交互用）。
final TickAtXDart tickAtX = bridge.lookupFunction<TickAtXNative, TickAtXDart>(
  'yinhe_tick_at_x',
);

typedef MarkerInsertNative = ffi.Int32 Function(ffi.Double, ffi.Pointer<Utf8>);
typedef MarkerInsertDart = int Function(double, ffi.Pointer<Utf8>);

/// 在 tick 处插入标签。
final MarkerInsertDart markerInsert = bridge
    .lookupFunction<MarkerInsertNative, MarkerInsertDart>(
      'yinhe_marker_insert',
    );

typedef MarkerRenameNative = ffi.Int32 Function(ffi.Double, ffi.Pointer<Utf8>);
typedef MarkerRenameDart = int Function(double, ffi.Pointer<Utf8>);

/// 重命名标签。
final MarkerRenameDart markerRename = bridge
    .lookupFunction<MarkerRenameNative, MarkerRenameDart>(
      'yinhe_marker_rename',
    );

typedef DoubleArgNative = ffi.Int32 Function(ffi.Double);
typedef DoubleArgDart = int Function(double);

/// 删除标签。
final DoubleArgDart markerDelete = bridge
    .lookupFunction<DoubleArgNative, DoubleArgDart>('yinhe_marker_delete');

typedef MarkerMoveNative = ffi.Int32 Function(ffi.Double, ffi.Float);
typedef MarkerMoveDart = int Function(double, double);

/// 移动标签（第二参数为画布逻辑 x）。
final MarkerMoveDart markerMove = bridge
    .lookupFunction<MarkerMoveNative, MarkerMoveDart>('yinhe_marker_move');

typedef DoubleVoidNative = ffi.Void Function(ffi.Double);
typedef DoubleVoidDart = void Function(double);

/// 播放光标跳到 tick（标尺点击，会吸附到量化格）。
final DoubleVoidDart jumpToTick = bridge
    .lookupFunction<DoubleVoidNative, DoubleVoidDart>('yinhe_jump_to_tick');

/// 事件列表跳转：光标精确落点 + PR 视口居中。
final DoubleVoidDart jumpToEvent = bridge
    .lookupFunction<DoubleVoidNative, DoubleVoidDart>('yinhe_jump_to_event');

typedef SetTrackNameNative = ffi.Int32 Function(ffi.Int32, ffi.Pointer<Utf8>);
typedef SetTrackNameDart = int Function(int, ffi.Pointer<Utf8>);

/// 重命名音轨。
final SetTrackNameDart setTrackName = bridge
    .lookupFunction<SetTrackNameNative, SetTrackNameDart>(
      'yinhe_set_track_name',
    );

/// 拖拽边缘自动滚动（每帧调用）。返回 1 = 已滚动。
final ActionDart dragTick = bridge.lookupFunction<ActionNative, ActionDart>(
  'yinhe_drag_tick',
);

/// 撤销一步。
final ActionDart undo = bridge.lookupFunction<ActionNative, ActionDart>(
  'yinhe_undo',
);

/// 重做一步。
final ActionDart redo = bridge.lookupFunction<ActionNative, ActionDart>(
  'yinhe_redo',
);

/// 设置当前工具（0=选框 1=手形 2=铅笔 3=橡皮 4=刷子）。
final IntArgDart setTool = bridge.lookupFunction<IntArgNative, IntArgDart>(
  'yinhe_set_tool',
);

/// 设置当前编辑轨（-1 = 无）。
final IntArgDart setActiveTrack = bridge
    .lookupFunction<IntArgNative, IntArgDart>('yinhe_set_active_track');

typedef SetQuantizeNative = ffi.Int32 Function(
  ffi.Int32,
  ffi.Int32,
  ffi.Uint32,
  ffi.Uint32,
);
typedef SetQuantizeDart = int Function(int, int, int, int);

/// 设置量化吸附档位：view 0=AR 1=PR；kind 0=Off 1=Auto 2=Fraction 3=Absolute。
final SetQuantizeDart setQuantize = bridge
    .lookupFunction<SetQuantizeNative, SetQuantizeDart>('yinhe_set_quantize');

typedef QuantizeTickIntervalNative = ffi.Int64 Function();
typedef QuantizeTickIntervalDart = int Function();

/// 当前视图量化档位的 tick 间隔（AM 选框吸附用）。
final QuantizeTickIntervalDart quantizeTickInterval = bridge
    .lookupFunction<QuantizeTickIntervalNative, QuantizeTickIntervalDart>(
      'yinhe_quantize_tick_interval',
    );

/// 覆盖层修订号（变化时重取 JSON）。
typedef OverlayRevNative = ffi.Uint32 Function();
typedef OverlayRevDart = int Function();

final OverlayRevDart overlayRevision = bridge
    .lookupFunction<OverlayRevNative, OverlayRevDart>('yinhe_overlay_revision');

/// 覆盖层 JSON（选框/预览）。
final CStrDart overlayJson = bridge.lookupFunction<CStrNative, CStrDart>(
  'yinhe_overlay_json',
);

/// 新建空工程。
final ActionDart newProject = bridge.lookupFunction<ActionNative, ActionDart>(
  'yinhe_new_project',
);

typedef SetMetaNative = ffi.Int32 Function(
  ffi.Pointer<Utf8>,
  ffi.Pointer<Utf8>,
  ffi.Pointer<Utf8>,
  ffi.Uint32,
);
typedef SetMetaDart = int Function(
  ffi.Pointer<Utf8>,
  ffi.Pointer<Utf8>,
  ffi.Pointer<Utf8>,
  int,
);

/// 工程元数据编辑（歌名/艺术家/简介/PPQ）。
final SetMetaDart setProjectMeta = bridge
    .lookupFunction<SetMetaNative, SetMetaDart>('yinhe_set_project_meta');

/// 音频设置 JSON。
final CStrDart audioSettingsJson = bridge.lookupFunction<CStrNative, CStrDart>(
  'yinhe_audio_settings_json',
);

typedef SetAudioSettingsNative = ffi.Int32 Function(
  ffi.Uint32,
  ffi.Uint32,
  ffi.Uint32,
);
typedef SetAudioSettingsDart = int Function(int, int, int);

/// 设置音频参数（max_voices=0 自动；buffer_size=0 设备默认）。
final SetAudioSettingsDart setAudioSettings = bridge
    .lookupFunction<SetAudioSettingsNative, SetAudioSettingsDart>(
      'yinhe_set_audio_settings',
    );

typedef SetAllowOverlapNative = ffi.Int32 Function(ffi.Int32);
typedef SetAllowOverlapDart = int Function(int);

/// 是否允许新音符/移动后的音符与其他音符重叠（桌面同款设置）。
final SetAllowOverlapDart setAllowOverlappingNotes = bridge
    .lookupFunction<SetAllowOverlapNative, SetAllowOverlapDart>(
      'yinhe_set_allow_overlapping_notes',
    );

typedef SetShowKeyboardLabelsNative = ffi.Int32 Function(ffi.Int32);
typedef SetShowKeyboardLabelsDart = int Function(int);

/// 卷帘背景显示 C 音名开关。
final SetShowKeyboardLabelsDart setShowKeyboardLabels = bridge
    .lookupFunction<SetShowKeyboardLabelsNative, SetShowKeyboardLabelsDart>(
      'yinhe_set_show_keyboard_labels',
    );

typedef ShowKeyboardLabelsNative = ffi.Int32 Function();
typedef ShowKeyboardLabelsDart = int Function();

final ShowKeyboardLabelsDart showKeyboardLabels = bridge
    .lookupFunction<ShowKeyboardLabelsNative, ShowKeyboardLabelsDart>(
      'yinhe_show_keyboard_labels',
    );

/// 工程元数据 JSON（回显）。
final CStrDart projectMetaJson = bridge.lookupFunction<CStrNative, CStrDart>(
  'yinhe_project_meta_json',
);

typedef SaveProjectNative = ffi.Int32 Function(ffi.Pointer<Utf8>);
typedef SaveProjectDart = int Function(ffi.Pointer<Utf8>);

/// 开始后台保存 `.yin` 工程到指定路径。
final SaveProjectDart saveProjectStart = bridge
    .lookupFunction<SaveProjectNative, SaveProjectDart>(
      'yinhe_save_project_start',
    );

/// 工程保存进度 JSON。
final CStrDart saveProjectStatusJson = bridge
    .lookupFunction<CStrNative, CStrDart>('yinhe_save_project_status_json');

/// 保存完成后标记工程状态（清除 dirty）。
final ActionDart markSaved = bridge.lookupFunction<ActionNative, ActionDart>(
  'yinhe_mark_saved',
);

typedef DocStateNative = ffi.Int32 Function(ffi.Pointer<ffi.Int32>);
typedef DocStateDart = int Function(ffi.Pointer<ffi.Int32>);

/// 工程状态写入 out：[can_undo, can_redo, dirty]。
final DocStateDart docState = bridge
    .lookupFunction<DocStateNative, DocStateDart>('yinhe_doc_state');

typedef RenderFrameNative = ffi.Int32 Function();
typedef RenderFrameDart = int Function();

/// Flutter 帧驱动渲染：渲染一帧并同步等待完成。
final RenderFrameDart renderFrame = bridge
    .lookupFunction<RenderFrameNative, RenderFrameDart>('yinhe_render_frame');

typedef SetAutomationPanelNative = ffi.Void Function(
  ffi.Float,
  ffi.Float,
  ffi.Int32,
  ffi.Int32,
  ffi.Int32,
  ffi.Int32,
);
typedef SetAutomationPanelDart = void Function(
  double,
  double,
  int,
  int,
  int,
  int,
);

/// 同步 AM 面板逻辑尺寸、可见性、当前 lane 和锚点显示。
///
/// `lane < 0` 表示力度模式；AM 使用独立 Surface/Texture。
final SetAutomationPanelDart setAutomationPanel = bridge
    .lookupFunction<SetAutomationPanelNative, SetAutomationPanelDart>(
      'yinhe_set_automation_panel',
    );

typedef SetAutomationGhostNative = ffi.Void Function(
  ffi.Pointer<ffi.Double>,
  ffi.Uint32,
);
typedef SetAutomationGhostDart = void Function(ffi.Pointer<ffi.Double>, int);

final SetAutomationGhostDart _setAutomationGhost = bridge
    .lookupFunction<SetAutomationGhostNative, SetAutomationGhostDart>(
      'yinhe_set_automation_ghost',
    );

/// 推送 AM 拖动 ghost：`(oldTick, newTick, value)`；空 = 清除。
void pushAutomationGhost(List<(int, int, double)> moves) {
  if (moves.isEmpty) {
    _setAutomationGhost(ffi.nullptr, 0);
    return;
  }
  final ptr = calloc<ffi.Double>(moves.length * 3);
  try {
    for (var i = 0; i < moves.length; i++) {
      final (oldTick, newTick, value) = moves[i];
      ptr[i * 3] = oldTick.toDouble();
      ptr[i * 3 + 1] = newTick.toDouble();
      ptr[i * 3 + 2] = value;
    }
    _setAutomationGhost(ptr, moves.length * 3);
  } finally {
    calloc.free(ptr);
  }
}

typedef SetAutomationSelectionNative = ffi.Void Function(
  ffi.Pointer<ffi.Double>,
  ffi.Uint32,
);
typedef SetAutomationSelectionDart = void Function(
  ffi.Pointer<ffi.Double>,
  int,
);

final SetAutomationSelectionDart _setAutomationSelection = bridge
    .lookupFunction<SetAutomationSelectionNative, SetAutomationSelectionDart>(
      'yinhe_set_automation_selection',
    );

/// 同步 AM 当前 lane 的选中锚点 tick；空集合 = 清除。
void setAutomationSelection(Iterable<int> ticks) {
  final list = ticks.where((t) => t >= 0).toList(growable: false);
  if (list.isEmpty) {
    _setAutomationSelection(ffi.nullptr, 0);
    return;
  }
  final ptr = calloc<ffi.Double>(list.length);
  try {
    for (var i = 0; i < list.length; i++) {
      ptr[i] = list[i].toDouble();
    }
    _setAutomationSelection(ptr, list.length);
  } finally {
    calloc.free(ptr);
  }
}

typedef FreeNative = ffi.Void Function(ffi.Pointer<Utf8>);
typedef FreeDart = void Function(ffi.Pointer<Utf8>);
final FreeDart freeString = bridge.lookupFunction<FreeNative, FreeDart>(
  'yinhe_free_string',
);

typedef OpenPathNative = ffi.Void Function(ffi.Pointer<Utf8>);
typedef OpenPathDart = void Function(ffi.Pointer<Utf8>);

final OpenPathDart openPath = bridge
    .lookupFunction<OpenPathNative, OpenPathDart>('yinhe_open_path');

/// 文件加载状态 JSON。
final CStrDart loadStatusJson = bridge.lookupFunction<CStrNative, CStrDart>(
  'yinhe_load_status_json',
);

/// 当前文档来源路径（无文档时为空串）。
final CStrDart currentPath = bridge.lookupFunction<CStrNative, CStrDart>(
  'yinhe_current_path',
);

// ── 视图 / 走带 ──

typedef IntVoidNative = ffi.Void Function(ffi.Int32);
typedef IntVoidDart = void Function(int);

/// 切换视图：0=AR 走带，1=PR 卷帘。
final IntVoidDart setViewMode = bridge
    .lookupFunction<IntVoidNative, IntVoidDart>('yinhe_set_view_mode');

/// 走带控制：0=播放，1=暂停，2=停止。
final IntVoidDart transport = bridge.lookupFunction<IntVoidNative, IntVoidDart>(
  'yinhe_transport',
);

typedef IntIntIntNative = ffi.Int32 Function(ffi.Int32, ffi.Int32);
typedef IntIntIntDart = int Function(int, int);

/// AR 长按排序：把 from 轨移动到 to 位置（成功 1）。
final IntIntIntDart moveTrack = bridge
    .lookupFunction<IntIntIntNative, IntIntIntDart>('yinhe_move_track');

typedef IntNative = ffi.Int32 Function();
typedef IntDart = int Function();

final IntDart isPlaying = bridge.lookupFunction<IntNative, IntDart>(
  'yinhe_is_playing',
);

typedef DoubleNative = ffi.Double Function();
typedef DoubleDart = double Function();

final DoubleDart playbackSeconds = bridge
    .lookupFunction<DoubleNative, DoubleDart>('yinhe_playback_seconds');
final DoubleDart playheadTick = bridge.lookupFunction<DoubleNative, DoubleDart>(
  'yinhe_playhead_tick',
);

typedef FloatNative = ffi.Float Function();
typedef FloatDart = double Function();

/// 播放光标 x（Rust 返回 f32，必须按 Float 读）。
final FloatDart playheadX = bridge.lookupFunction<FloatNative, FloatDart>(
  'yinhe_playhead_x',
);

typedef SnapNative = ffi.Int32 Function(ffi.Pointer<ffi.Float>);
typedef SnapDart = int Function(ffi.Pointer<ffi.Float>);

final SnapDart viewSnapshot = bridge.lookupFunction<SnapNative, SnapDart>(
  'yinhe_view_snapshot',
);
