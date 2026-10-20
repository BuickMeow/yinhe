import 'dart:ffi' as ffi;

import 'package:ffi/ffi.dart';

part 'ffi/input.dart';
part 'ffi/document.dart';
part 'ffi/edit.dart';
part 'ffi/view.dart';

/// Rust 桥（libyinhe_flutter.so）。
final ffi.DynamicLibrary bridge = ffi.DynamicLibrary.open(
  'libyinhe_flutter.so',
);
