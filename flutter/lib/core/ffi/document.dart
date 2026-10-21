part of '../ffi.dart';

/// 音色库表 JSON（顺序 = 优先级：靠前的库覆盖靠后的）。
final CStrDart soundfontsJson = bridge.lookupFunction<CStrNative, CStrDart>(
  'yinhe_soundfonts_json',
);

typedef CStrArgNative = ffi.Int32 Function(ffi.Pointer<Utf8>);
typedef CStrArgDart = int Function(ffi.Pointer<Utf8>);

/// 设置音色库表（设置页排序/启停/删除后调用）。
final CStrArgDart setSoundfonts = bridge
    .lookupFunction<CStrArgNative, CStrArgDart>('yinhe_set_soundfonts');

/// 歌曲标题（空串 = 未设置）。
final CStrDart songTitle = bridge.lookupFunction<CStrNative, CStrDart>(
  'yinhe_song_title',
);

typedef Int64Native = ffi.Int64 Function();
typedef Int64Dart = int Function();

/// 全曲音符数。
final Int64Dart noteCount = bridge.lookupFunction<Int64Native, Int64Dart>(
  'yinhe_note_count',
);

/// 本工程默认 BPM / 拍号 JSON：`{"bpm":120,"num":4,"den":4}`。
final CStrDart projectDefaultsJson = bridge
    .lookupFunction<CStrNative, CStrDart>('yinhe_project_defaults_json');

typedef SetProjectDefaultsNative = ffi.Int32 Function(
  ffi.Float,
  ffi.Int32,
  ffi.Int32,
);
typedef SetProjectDefaultsDart = int Function(double, int, int);

/// 设置本工程默认 BPM / 拍号（分子 + 分母 2 次幂）。
final SetProjectDefaultsDart setProjectDefaults = bridge
    .lookupFunction<SetProjectDefaultsNative, SetProjectDefaultsDart>(
      'yinhe_set_project_defaults',
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

/// 取消编辑并停止惯性（模态窗口/后台切换后调用）。
final VoidActionDart resetInteraction = bridge
    .lookupFunction<VoidActionNative, VoidActionDart>(
      'yinhe_reset_interaction',
    );

/// 选框拖拽状态：0=无、1=待定（长按中）、2=移动态、3=空白处长按完成（待粘贴）。
final ActionDart selectionState = bridge
    .lookupFunction<ActionNative, ActionDart>('yinhe_selection_state');

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

/// 长按落点 tick（每帧更新指示线用；Rust 返回 f64）。
final DoubleDart longPressTick = bridge
    .lookupFunction<DoubleNative, DoubleDart>('yinhe_long_press_tick');

/// 长按落点轨道（-1 = 无）。
final ActionDart longPressTrack = bridge
    .lookupFunction<ActionNative, ActionDart>('yinhe_long_press_track');

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
