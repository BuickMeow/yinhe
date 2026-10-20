import 'dart:convert';
import 'dart:ffi' as ffi;

import 'package:ffi/ffi.dart';

import 'ffi.dart' as rust;

/// 常用量化档位（`label`, tick 间隔）；数据源 Rust `presets_for_ppq`。
List<(String, int)> quantizePresetsForPpq(double ppq) {
  final ptr = rust.quantizePresetsJson(ppq);
  if (ptr == ffi.nullptr) {
    return const [];
  }
  try {
    return decodeQuantizePresets(ptr.toDartString());
  } finally {
    rust.freeString(ptr);
  }
}

/// 解析 `[{"label":"1/8","ticks":240}, ...]`；格式错误返回空表。
List<(String, int)> decodeQuantizePresets(String json) {
  try {
    final data = jsonDecode(json);
    if (data is! List) {
      return const [];
    }
    return [
      for (final e in data)
        if (e is Map && e['label'] is String && e['ticks'] is num)
          (e['label'] as String, (e['ticks'] as num).toInt()),
    ];
  } catch (_) {
    return const [];
  }
}
