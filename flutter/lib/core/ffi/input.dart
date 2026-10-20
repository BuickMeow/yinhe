part of '../ffi.dart';

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
