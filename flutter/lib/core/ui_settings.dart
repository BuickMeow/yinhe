import 'package:flutter/foundation.dart';

/// 卷帘背景音名开关（设置页写入，Shell/画布读取）。
///
/// 与 Rust `AudioSettings.show_keyboard_labels` 同步持久化；这里只做
/// Flutter 侧的即时通知，避免每帧读一次设置 JSON。
final ValueNotifier<bool> showKeyboardLabels = ValueNotifier<bool>(false);
