import 'dart:ffi' as ffi;

import 'package:ffi/ffi.dart';

part 'ffi/input.dart';
part 'ffi/document.dart';
part 'ffi/edit.dart';
part 'ffi/export.dart';
part 'ffi/names.dart';
part 'ffi/view.dart';

/// Rust 桥（libyinhe_flutter.so）。
final ffi.DynamicLibrary bridge = ffi.DynamicLibrary.open(
  'libyinhe_flutter.so',
);

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
