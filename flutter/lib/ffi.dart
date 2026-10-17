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

// ── 文档 / 演示 ──

typedef VoidNative = ffi.Void Function();
typedef VoidDart = void Function();

final VoidDart demoLoad = bridge.lookupFunction<VoidNative, VoidDart>(
  'yinhe_demo_load',
);

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

/// 力度条数据 JSON（当前轨道）。
final IntCStrDart velocityJson = bridge
    .lookupFunction<IntCStrNative, IntCStrDart>('yinhe_velocity_json');

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
