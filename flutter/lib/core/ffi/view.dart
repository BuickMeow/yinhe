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

/// 播放光标跳到 tick。
final DoubleVoidDart jumpToTick = bridge
    .lookupFunction<DoubleVoidNative, DoubleVoidDart>('yinhe_jump_to_tick');

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
  ffi.Uint32,
  ffi.Uint32,
);
typedef SetQuantizeDart = int Function(int, int, int);

/// 设置量化吸附档位：kind 0=Off 1=Auto 2=Fraction 3=Absolute。
final SetQuantizeDart setQuantize = bridge
    .lookupFunction<SetQuantizeNative, SetQuantizeDart>('yinhe_set_quantize');

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

/// AR 折叠态自动化竖线 JSON。
final CStrDart arAutomationJson = bridge.lookupFunction<CStrNative, CStrDart>(
  'yinhe_ar_automation_json',
);

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

/// 工程元数据 JSON（回显）。
final CStrDart projectMetaJson = bridge.lookupFunction<CStrNative, CStrDart>(
  'yinhe_project_meta_json',
);

typedef SaveYingNative = ffi.Int32 Function(ffi.Pointer<Utf8>);
typedef SaveYingDart = int Function(ffi.Pointer<Utf8>);

/// 导出 `.ying` 工程到指定路径。
final SaveYingDart saveYing = bridge
    .lookupFunction<SaveYingNative, SaveYingDart>('yinhe_save_ying');

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

typedef FreeNative = ffi.Void Function(ffi.Pointer<Utf8>);
typedef FreeDart = void Function(ffi.Pointer<Utf8>);

final FreeDart freeString = bridge.lookupFunction<FreeNative, FreeDart>(
  'yinhe_free_string',
);

typedef OpenPathNative = ffi.Void Function(ffi.Pointer<Utf8>);
typedef OpenPathDart = void Function(ffi.Pointer<Utf8>);

final OpenPathDart openPath = bridge
    .lookupFunction<OpenPathNative, OpenPathDart>('yinhe_open_path');

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
