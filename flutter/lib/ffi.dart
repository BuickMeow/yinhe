import 'dart:ffi' as ffi;

import 'package:ffi/ffi.dart';

/// Rust 桥（libyinhe_flutter.so）。
final ffi.DynamicLibrary bridge = ffi.DynamicLibrary.open(
  'libyinhe_flutter.so',
);

// ── 触摸 ──

typedef TouchNative = ffi.Void Function(
  ffi.Int32,
  ffi.Float,
  ffi.Float,
  ffi.Int32,
);
typedef TouchDart = void Function(int, double, double, int);

final TouchDart touch = bridge.lookupFunction<TouchNative, TouchDart>(
  'yinhe_touch',
);

// ── 文档 / 工程 ──

typedef CStrNative = ffi.Pointer<Utf8> Function();
typedef CStrDart = ffi.Pointer<Utf8> Function();

final CStrDart recentJson = bridge.lookupFunction<CStrNative, CStrDart>(
  'yinhe_recent_json',
);
final CStrDart tracksJson = bridge.lookupFunction<CStrNative, CStrDart>(
  'yinhe_tracks_json',
);

typedef IntCStrNative = ffi.Pointer<Utf8> Function(ffi.Int32);
typedef IntCStrDart = ffi.Pointer<Utf8> Function(int);

/// PR 自动化栏数据（PB/CC/RPN）JSON。
final IntCStrDart automationJson = bridge
    .lookupFunction<IntCStrNative, IntCStrDart>('yinhe_automation_json');

/// 力度条数据 JSON（所有可见轨）。
final CStrDart velocityJson = bridge.lookupFunction<CStrNative, CStrDart>(
  'yinhe_velocity_json',
);

typedef ScrollAtNative = ffi.Int32 Function(
  ffi.Float,
  ffi.Float,
  ffi.Float,
  ffi.Float,
  ffi.Int32,
);
typedef ScrollAtDart = int Function(double, double, double, double, int);

/// 滚轮/触控板：ctrl=1 缩放（键盘区副轴，其余主轴），否则平移。
final ScrollAtDart scrollAt = bridge
    .lookupFunction<ScrollAtNative, ScrollAtDart>('yinhe_scroll_at');

typedef ZoomAtNative = ffi.Int32 Function(ffi.Float, ffi.Float, ffi.Float);
typedef ZoomAtDart = int Function(double, double, double);

/// 触控板捏合缩放（factor > 1 放大）。
final ZoomAtDart zoomAt = bridge.lookupFunction<ZoomAtNative, ZoomAtDart>(
  'yinhe_zoom_at',
);

typedef SetAutoNative = ffi.Int32 Function(
  ffi.Int32,
  ffi.Int32,
  ffi.Uint32,
  ffi.Float,
);
typedef SetAutoDart = int Function(int, int, int, double);

/// 编辑自动化锚点：track / lane_index(1-based) / tick / value(0..1)。
final SetAutoDart setAutomationValue = bridge
    .lookupFunction<SetAutoNative, SetAutoDart>('yinhe_set_automation_value');

/// 当前音色库路径（空串 = 未选择）。
final CStrDart soundfontPath = bridge.lookupFunction<CStrNative, CStrDart>(
  'yinhe_soundfont_path',
);

/// 歌曲标题（空串 = 未设置）。
final CStrDart songTitle = bridge.lookupFunction<CStrNative, CStrDart>(
  'yinhe_song_title',
);

typedef IntArgNative = ffi.Int32 Function(ffi.Int32);
typedef IntArgDart = int Function(int);

/// 设置“显示轨道”过滤（-1 = 全部轨道）。
final IntArgDart setDisplayTrack = bridge
    .lookupFunction<IntArgNative, IntArgDart>('yinhe_set_display_track');

typedef ActionNative = ffi.Int32 Function();
typedef ActionDart = int Function();

typedef VoidActionNative = ffi.Void Function();
typedef VoidActionDart = void Function();

/// 清空 Rust 触摸状态（模态窗口/后台切换后调用，避免残留触点）。
final VoidActionDart resetTouch = bridge
    .lookupFunction<VoidActionNative, VoidActionDart>('yinhe_reset_touch');

/// 选框拖拽状态：0=无、1=待定（长按中）、2=移动态、3=空白处长按完成（待粘贴）。
final ActionDart selectionState = bridge
    .lookupFunction<ActionNative, ActionDart>('yinhe_selection_state');

/// 取消待定选框（未完成长按）。
final ActionDart cancelSelectionPending = bridge
    .lookupFunction<ActionNative, ActionDart>('yinhe_cancel_selection_pending');

/// 复制当前选中音符到应用内剪贴板。
final ActionDart copySelection = bridge
    .lookupFunction<ActionNative, ActionDart>('yinhe_copy_selection');

/// 剪贴板是否为空：1 = 空。
final ActionDart clipboardEmpty = bridge
    .lookupFunction<ActionNative, ActionDart>('yinhe_clipboard_empty');

typedef GenerationNative = ffi.Uint32 Function();
typedef GenerationDart = int Function();

/// 文档/选区世代号（变化时刷新轨道/自动化浮层）。
final GenerationDart documentGeneration = bridge
    .lookupFunction<GenerationNative, GenerationDart>(
      'yinhe_document_generation',
    );

typedef FloatArgNative = ffi.Int32 Function(ffi.Float);
typedef FloatArgDart = int Function(double);

/// 在长按位置（画布逻辑 x）粘贴剪贴板。
final FloatArgDart pasteSelection = bridge
    .lookupFunction<FloatArgNative, FloatArgDart>('yinhe_paste_selection');

/// 长按空白处上下文 JSON（插入事件用）。
final CStrDart longPressContextJson = bridge
    .lookupFunction<CStrNative, CStrDart>('yinhe_long_press_context_json');

/// 通道级默认乐器 JSON（16 项：`[bank, program]` 或 null）。
final CStrDart channelInstrumentsJson = bridge
    .lookupFunction<CStrNative, CStrDart>('yinhe_channel_instruments_json');

typedef SetTrackInfoNative = ffi.Int32 Function(
  ffi.Int32,
  ffi.Pointer<Utf8>,
  ffi.Int32,
  ffi.Int32,
  ffi.Int32,
  ffi.Int32,
  ffi.Int32,
);
typedef SetTrackInfoDart = int Function(
  int,
  ffi.Pointer<Utf8>,
  int,
  int,
  int,
  int,
  int,
);

/// 切换轨道 mute。
final IntArgDart toggleTrackMute = bridge
    .lookupFunction<IntArgNative, IntArgDart>('yinhe_toggle_track_mute');

/// 切换轨道 solo。
final IntArgDart toggleTrackSolo = bridge
    .lookupFunction<IntArgNative, IntArgDart>('yinhe_toggle_track_solo');

typedef AddTracksNative = ffi.Int32 Function(
  ffi.Int32,
  ffi.Int32,
  ffi.Int32,
  ffi.Int32,
);
typedef AddTracksDart = int Function(int, int, int, int);

/// 新建 MIDI 轨（cycle=1：同 port 内循环通道）。
final AddTracksDart addTracks = bridge
    .lookupFunction<AddTracksNative, AddTracksDart>('yinhe_add_tracks');

/// 编辑音轨信息（名称/端口/通道 + 通道默认乐器）。
final SetTrackInfoDart setTrackInfo = bridge
    .lookupFunction<SetTrackInfoNative, SetTrackInfoDart>(
      'yinhe_set_track_info',
    );

typedef InsertAutomationNative = ffi.Int32 Function(
  ffi.Int32,
  ffi.Int32,
  ffi.Double,
  ffi.Int32,
  ffi.Double,
);
typedef InsertAutomationDart = int Function(int, int, double, int, double);

/// 长按"插入"自动化事件（value 为显示值：PB/CC/RPN 原始值域，Tempo 为 BPM）。
final InsertAutomationDart insertAutomation = bridge
    .lookupFunction<InsertAutomationNative, InsertAutomationDart>(
      'yinhe_insert_automation',
    );

typedef DisplayValueNative = ffi.Double Function(
  ffi.Int32,
  ffi.Int32,
  ffi.Double,
  ffi.Int32,
);
typedef DisplayValueDart = double Function(int, int, double, int);

/// 该位置当前显示值（无历史值返回 NaN）。
final DisplayValueDart automationDisplayValue = bridge
    .lookupFunction<DisplayValueNative, DisplayValueDart>(
      'yinhe_automation_display_value',
    );

/// 标尺标签（marker）JSON。
final CStrDart markersJson = bridge.lookupFunction<CStrNative, CStrDart>(
  'yinhe_markers_json',
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
