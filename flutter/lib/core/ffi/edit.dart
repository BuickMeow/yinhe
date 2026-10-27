part of '../ffi.dart';

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

typedef MoveAutomationAnchorsNative = ffi.Int32 Function(
  ffi.Int32,
  ffi.Int32,
  ffi.Pointer<ffi.Double>,
  ffi.Uint32,
);
typedef MoveAutomationAnchorsDart = int Function(
  int,
  int,
  ffi.Pointer<ffi.Double>,
  int,
);

final MoveAutomationAnchorsDart _moveAutomationAnchors = bridge
    .lookupFunction<MoveAutomationAnchorsNative, MoveAutomationAnchorsDart>(
      'yinhe_move_automation_anchors',
    );

typedef EnsureAutomationLaneNative = ffi.Int32 Function(
  ffi.Int32,
  ffi.Int32,
  ffi.Int32,
);
typedef EnsureAutomationLaneDart = int Function(int, int, int);

/// 确保自动化 lane 存在（kind/number 与插入自动化一致）。
final EnsureAutomationLaneDart ensureAutomationLane = bridge
    .lookupFunction<EnsureAutomationLaneNative, EnsureAutomationLaneDart>(
      'yinhe_ensure_automation_lane',
    );

/// 批量移动自动化锚点：`moves = (oldTick, newTick, 归一化值)`，一笔一个 undo。
int moveAutomationAnchors(
  int track,
  int laneIndex,
  List<(int, int, double)> moves,
) {
  if (moves.isEmpty) {
    return 0;
  }
  final ptr = calloc<ffi.Double>(moves.length * 3);
  try {
    for (var i = 0; i < moves.length; i++) {
      ptr[i * 3] = moves[i].$1.toDouble();
      ptr[i * 3 + 1] = moves[i].$2.toDouble();
      ptr[i * 3 + 2] = moves[i].$3;
    }
    return _moveAutomationAnchors(track, laneIndex, ptr, moves.length * 3);
  } finally {
    calloc.free(ptr);
  }
}

/// 标尺标签（marker）JSON。
final CStrDart markersJson = bridge.lookupFunction<CStrNative, CStrDart>(
  'yinhe_markers_json',
);

/// 拍号分段 JSON：`[[tick,num,den_pow], ...]`（标尺按分段排版）。
final CStrDart timeSigSegmentsJson = bridge
    .lookupFunction<CStrNative, CStrDart>('yinhe_time_sig_segments_json');

typedef InsertSigNative = ffi.Int32 Function(ffi.Double, ffi.Int32, ffi.Int32);
typedef InsertSigDart = int Function(double, int, int);

/// 在 conductor 指定 tick 插入拍号（分子 + 分母 2 次幂）。
final InsertSigDart insertTimeSig = bridge
    .lookupFunction<InsertSigNative, InsertSigDart>('yinhe_insert_time_sig');

/// 在 conductor 指定 tick 插入调号（主音 0=C..11=B；scale 0=大调 1=小调）。
final InsertSigDart insertKeySig = bridge
    .lookupFunction<InsertSigNative, InsertSigDart>('yinhe_insert_key_sig');

/// 删除音轨（连同其音符；一步一 undo）。
final IntArgDart deleteTrack = bridge.lookupFunction<IntArgNative, IntArgDart>(
  'yinhe_delete_track',
);

typedef TickToSecondsNative = ffi.Double Function(ffi.Double);
typedef TickToSecondsDart = double Function(double);

typedef StrToIntNative = ffi.Int32 Function(ffi.Pointer<Utf8>);
typedef StrToIntDart = int Function(ffi.Pointer<Utf8>);

typedef IntCStrCStrNative = ffi.Int32 Function(
  ffi.Pointer<Utf8>,
  ffi.Pointer<Utf8>,
);
typedef IntCStrCStrDart = int Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>);

typedef EventsJsonNative = ffi.Pointer<Utf8> Function(ffi.Double, ffi.Int32);
typedef EventsJsonDart = ffi.Pointer<Utf8> Function(double, int);

/// 界面语言设置（system/zh/en）。
final CStrDart uiLanguage = bridge.lookupFunction<CStrNative, CStrDart>(
  'yinhe_ui_language',
);

/// 设置界面语言并持久化。
final CStrArgDart setUiLanguage = bridge
    .lookupFunction<CStrArgNative, CStrArgDart>('yinhe_set_ui_language');

/// 事件列表 JSON（从 from_tick 起，最多 limit 条）。
final EventsJsonDart eventsJson = bridge
    .lookupFunction<EventsJsonNative, EventsJsonDart>('yinhe_events_json');

/// 删除事件行（JSON）。
final CStrArgDart eventDelete = bridge
    .lookupFunction<CStrArgNative, CStrArgDart>('yinhe_event_delete');

typedef EventUpdateNative = ffi.Int32 Function(
  ffi.Pointer<Utf8>,
  ffi.Double,
  ffi.Double,
);
typedef EventUpdateDart = int Function(ffi.Pointer<Utf8>, double, double);

/// 修改事件行：音符 = (力度, 结束 tick)；自动化 = 显示值。
final EventUpdateDart eventUpdate = bridge
    .lookupFunction<EventUpdateNative, EventUpdateDart>('yinhe_event_update');

/// 修改标签文本。
final IntCStrCStrDart eventSetText = bridge
    .lookupFunction<IntCStrCStrNative, IntCStrCStrDart>('yinhe_event_set_text');

/// 粘贴事件行（source → target 的时间位置/音轨）。
final IntCStrCStrDart eventPaste = bridge
    .lookupFunction<IntCStrCStrNative, IntCStrCStrDart>('yinhe_event_paste');

/// 压缩包状态：0=无 1=打开中 2=就绪 3=需要密码 4=失败。
final ActionDart archiveStatus = bridge
    .lookupFunction<ActionNative, ActionDart>('yinhe_archive_status');

/// 压缩包信息 JSON（条目列表）。
final CStrDart archiveJson = bridge.lookupFunction<CStrNative, CStrDart>(
  'yinhe_archive_json',
);

/// 打开压缩包（后台，轮询 archiveStatus）。
final StrToIntDart archiveOpen = bridge
    .lookupFunction<StrToIntNative, StrToIntDart>('yinhe_archive_open');

/// 带密码打开压缩包。
final IntCStrCStrDart archiveOpenPassword = bridge
    .lookupFunction<IntCStrCStrNative, IntCStrCStrDart>(
      'yinhe_archive_open_password',
    );

/// 打开压缩包内第 index 个 MIDI 条目。
final IntArgDart archiveOpenEntry = bridge
    .lookupFunction<IntArgNative, IntArgDart>('yinhe_archive_open_entry');

typedef SelectionActionNative = ffi.Int32 Function(ffi.Int32, ffi.Int32);
typedef SelectionActionDart = int Function(int, int);

/// 框选批量操作（编号与桌面「操作」菜单一致）。
///
/// `scope`: 0 = 音符（PR/AR），1 = AM 当前 lane 的锚点。
final SelectionActionDart selectionAction = bridge
    .lookupFunction<SelectionActionNative, SelectionActionDart>(
      'yinhe_selection_action',
    );

typedef SelectAllNative = ffi.Int32 Function(ffi.Int32, ffi.Int32);
typedef SelectAllDart = int Function(int, int);

/// 全选：`scope` 0 = PR，1 = AR；`track < 0` = 全工程。
final SelectAllDart selectAll = bridge
    .lookupFunction<SelectAllNative, SelectAllDart>('yinhe_select_all');

typedef ClearSelectionNative = ffi.Int32 Function();
typedef ClearSelectionDart = int Function();

/// 清空音符/AR 选区；返回 1 = 原本有选区。
final ClearSelectionDart clearSelection = bridge
    .lookupFunction<ClearSelectionNative, ClearSelectionDart>(
      'yinhe_clear_selection',
    );

typedef HumanizeNative = ffi.Int32 Function(ffi.Uint32, ffi.Uint32, ffi.Uint32);
typedef HumanizeDart = int Function(int, int, int);

/// 人性化：时间/力度/时值三路随机偏差幅度。
final HumanizeDart humanize = bridge
    .lookupFunction<HumanizeNative, HumanizeDart>('yinhe_humanize');

typedef BatchEditNative = ffi.Int32 Function(
  ffi.Int32,
  ffi.Double,
  ffi.Int32,
  ffi.Double,
  ffi.Int32,
  ffi.Double,
  ffi.Int32,
  ffi.Double,
  ffi.Int32,
);
typedef BatchEditDart = int Function(
  int,
  double,
  int,
  double,
  int,
  double,
  int,
  double,
  int,
);

/// 批量处理（音符）：四个字段各带操作码，NaN = 跳过。
final BatchEditDart batchEdit = bridge
    .lookupFunction<BatchEditNative, BatchEditDart>('yinhe_batch_edit');

typedef AutomationBatchEditNative = ffi.Int32 Function(
  ffi.Int32,
  ffi.Double,
  ffi.Int32,
  ffi.Double,
);
typedef AutomationBatchEditDart = int Function(int, double, int, double);

/// 批量处理（AM 当前 lane）：值与位置各带操作码，NaN = 跳过。
final AutomationBatchEditDart automationBatchEdit = bridge
    .lookupFunction<AutomationBatchEditNative, AutomationBatchEditDart>(
      'yinhe_automation_batch_edit',
    );
