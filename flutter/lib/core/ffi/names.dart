part of '../ffi.dart';

/// MIDI 静态名称表 JSON（GM/CC/RPN/音名/插入上限）。
final CStrDart midiNamesJson = bridge.lookupFunction<CStrNative, CStrDart>(
  'yinhe_midi_names_json',
);

typedef QuantizePresetsNative = ffi.Pointer<Utf8> Function(ffi.Double);
typedef QuantizePresetsDart = ffi.Pointer<Utf8> Function(double);

/// 常用量化档位 JSON（按 PPQ 推导）。
final QuantizePresetsDart quantizePresetsJson = bridge
    .lookupFunction<QuantizePresetsNative, QuantizePresetsDart>(
      'yinhe_quantize_presets_json',
    );
