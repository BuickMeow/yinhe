part of '../ffi.dart';

// ── 手势 / 编辑 ──

typedef Float2IntNative = ffi.Int32 Function(ffi.Float, ffi.Float);
typedef Float2IntDart = int Function(double, double);

/// 单指按下：返回 1 = 进入编辑，0 = 交给平移。
final Float2IntDart editBegin = bridge
    .lookupFunction<Float2IntNative, Float2IntDart>('yinhe_edit_begin');

/// 单指落在音符右端：进入改长度，返回 1 表示已接管。
final Float2IntDart beginNoteResize = bridge
    .lookupFunction<Float2IntNative, Float2IntDart>('yinhe_begin_note_resize');

/// 单指落在选框控制点：进入选框缩放，返回 1 表示已接管。
final Float2IntDart beginSelectionHandle = bridge
    .lookupFunction<Float2IntNative, Float2IntDart>(
      'yinhe_begin_selection_handle',
    );

/// 单指落在已有选框内：直接移动选框，返回 1 表示已接管。
final Float2IntDart beginSelectionMove = bridge
    .lookupFunction<Float2IntNative, Float2IntDart>(
      'yinhe_begin_selection_move',
    );

/// 单指落在音符中部：进入移动，返回 1 表示已接管。
final Float2IntDart beginNoteMove = bridge
    .lookupFunction<Float2IntNative, Float2IntDart>('yinhe_begin_note_move');

/// 时间标尺长按：从标尺位置起手插入/粘贴定位。
final Float2IntDart beginRulerInsert = bridge
    .lookupFunction<Float2IntNative, Float2IntDart>('yinhe_begin_ruler_insert');

typedef IntNoArgNative = ffi.Int32 Function();
typedef IntNoArgDart = int Function();

/// 本次长按是否来自时间标尺（只有标尺插入显示竖线）。
final IntNoArgDart insertFromRuler = bridge
    .lookupFunction<IntNoArgNative, IntNoArgDart>('yinhe_insert_from_ruler');

/// 以选框语义起手（不修改全局工具；长按拖选框用）。
final Float2IntDart beginMarquee = bridge
    .lookupFunction<Float2IntNative, Float2IntDart>('yinhe_begin_marquee');

/// 长按点是否落在已有选框内。
final Float2IntDart selectionBoxAt = bridge
    .lookupFunction<Float2IntNative, Float2IntDart>('yinhe_selection_box_at');

/// 单指移动：返回 1 = 仍在编辑，0 = 已取消（转平移）。
final Float2IntDart editUpdate = bridge
    .lookupFunction<Float2IntNative, Float2IntDart>('yinhe_edit_update');

/// PR 单击：命中音符删除，否则量化位置插入。
final Float2IntDart tapNote = bridge
    .lookupFunction<Float2IntNative, Float2IntDart>('yinhe_tap_note');

typedef Float2PtrNative = ffi.Pointer<Utf8> Function(ffi.Float, ffi.Float);
typedef Float2PtrDart = ffi.Pointer<Utf8> Function(double, double);

/// 长按位置上下文 JSON（只用于弹浮动菜单）。
final Float2PtrDart longPressContextAt = bridge
    .lookupFunction<Float2PtrNative, Float2PtrDart>(
      'yinhe_long_press_context_at',
    );

typedef Float2VoidNative = ffi.Void Function(ffi.Float, ffi.Float);
typedef Float2VoidDart = void Function(double, double);

/// 单指释放：提交编辑。
final Float2VoidDart editEnd = bridge
    .lookupFunction<Float2VoidNative, Float2VoidDart>('yinhe_edit_end');

/// 取消编辑拖拽。
final ActionDart editCancel = bridge.lookupFunction<ActionNative, ActionDart>(
  'yinhe_edit_cancel',
);

/// Flutter 长按完成：待定选框/空白态推进。
final ActionDart editLongPress = bridge
    .lookupFunction<ActionNative, ActionDart>('yinhe_edit_long_press');

typedef ViewportGestureNative = ffi.Int32 Function(
  ffi.Int32,
  ffi.Float,
  ffi.Float,
  ffi.Float,
  ffi.Float,
  ffi.Float,
);
typedef ViewportGestureDart = int Function(
  int,
  double,
  double,
  double,
  double,
  double,
);

/// 应用一次视口手势：mode / panX / panY / scale / focusX / focusY。
final ViewportGestureDart viewportGesture = bridge
    .lookupFunction<ViewportGestureNative, ViewportGestureDart>(
      'yinhe_viewport_gesture',
    );

/// 按逻辑像素平移视口（鼠标右键）。
final Float2IntDart panBy = bridge
    .lookupFunction<Float2IntNative, Float2IntDart>('yinhe_pan_by');

/// 手势结束速度（逻辑像素/秒）启动惯性。
final Float2VoidDart startFling = bridge
    .lookupFunction<Float2VoidNative, Float2VoidDart>('yinhe_start_fling');

/// 停止惯性。
final VoidActionDart stopFling = bridge
    .lookupFunction<VoidActionNative, VoidActionDart>('yinhe_stop_fling');

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

typedef VelocityBarsNative = ffi.Pointer<ffi.Double> Function(
  ffi.Pointer<ffi.Uint32>,
);
typedef VelocityBarsDart = ffi.Pointer<ffi.Double> Function(
  ffi.Pointer<ffi.Uint32>,
);

/// PR 力度条二进制数据：`[tick, length, velocity01, track, ...]`。
/// 返回内部缓冲区指针，下一次调用前必须复制。
final VelocityBarsDart velocityBars = bridge
    .lookupFunction<VelocityBarsNative, VelocityBarsDart>(
      'yinhe_velocity_bars',
    );

typedef VelocityStrokeNative = ffi.Int32 Function(
  ffi.Pointer<ffi.Double>,
  ffi.Uint32,
);
typedef VelocityStrokeDart = int Function(ffi.Pointer<ffi.Double>, int);

/// 提交一笔力度绘制：扁平 `[tick, velocity, ...]`，第二参数是 f64 元素数（点数 × 2）。
final VelocityStrokeDart velocityStroke = bridge
    .lookupFunction<VelocityStrokeNative, VelocityStrokeDart>(
      'yinhe_velocity_stroke',
    );

typedef VelocityLineNative = ffi.Int32 Function(
  ffi.Double,
  ffi.Double,
  ffi.Double,
  ffi.Double,
);
typedef VelocityLineDart = int Function(double, double, double, double);

/// AM 直线力度：起止 `(tick, velocity01)`，一笔一个 undo。
final VelocityLineDart velocityLine = bridge
    .lookupFunction<VelocityLineNative, VelocityLineDart>(
      'yinhe_velocity_line',
    );

typedef DragDeltaNative = ffi.Int32 Function(
  ffi.Pointer<ffi.Int64>,
  ffi.Pointer<ffi.Int32>,
  ffi.Pointer<ffi.Int32>,
);
typedef DragDeltaDart = int Function(
  ffi.Pointer<ffi.Int64>,
  ffi.Pointer<ffi.Int32>,
  ffi.Pointer<ffi.Int32>,
);

/// 选框整体移动增量：dt(tick) / dk(半音) / dtr(轨道)，返回 1 = 移动态。
final DragDeltaDart dragDelta = bridge
    .lookupFunction<DragDeltaNative, DragDeltaDart>('yinhe_drag_delta');

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
