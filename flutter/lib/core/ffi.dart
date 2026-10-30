import 'dart:ffi' as ffi;
import 'dart:io' show Platform;

import 'package:ffi/ffi.dart';

part 'ffi/input.dart';
part 'ffi/document.dart';
part 'ffi/edit.dart';
part 'ffi/export.dart';
part 'ffi/soundfont_archive.dart';
part 'ffi/names.dart';
part 'ffi/view.dart';

/// Rust 桥。
///
/// - Android：`libyinhe_flutter.so`（按 soname 加载）。
/// - OpenHarmony：链接器命名空间不保证按 soname 搜索应用自己的 libs 目录，
///   先按 soname 试，失败再用应用 libs 的绝对路径（鸿蒙布局固定）。
/// - iOS：静态库链进 Runner（app 内不能带 dylib），符号在进程内，直接查主程序。
final ffi.DynamicLibrary bridge = _openBridge();

ffi.DynamicLibrary _openBridge() {
  if (Platform.isIOS) {
    return ffi.DynamicLibrary.process();
  }
  const candidates = <String>[
    'libyinhe_flutter.so',
    '/data/storage/el1/bundle/libs/arm64/libyinhe_flutter.so',
  ];
  Object? lastError;
  for (final path in candidates) {
    try {
      return ffi.DynamicLibrary.open(path);
    } catch (e) {
      lastError = e;
    }
  }
  throw StateError('无法加载 libyinhe_flutter.so: $lastError');
}

/// 读取并释放 Rust 返回的堆字符串。
String takeString(ffi.Pointer<Utf8> ptr) {
  if (ptr == ffi.nullptr) {
    return '';
  }
  try {
    return ptr.toDartString();
  } finally {
    freeString(ptr);
  }
}
