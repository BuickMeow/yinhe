part of '../ffi.dart';

typedef ExportAudioStartNative = ffi.Int32 Function(
  ffi.Pointer<Utf8>,
  ffi.Int32,
);
typedef ExportAudioStartDart = int Function(ffi.Pointer<Utf8>, int);

/// 开始导出 WAV（路径 + 16/24/32 位深）。
final ExportAudioStartDart exportAudioStart = bridge
    .lookupFunction<ExportAudioStartNative, ExportAudioStartDart>(
      'yinhe_export_audio_start',
    );

/// 音频导出进度 JSON。
final CStrDart exportAudioProgressJson = bridge
    .lookupFunction<CStrNative, CStrDart>('yinhe_export_audio_progress_json');

typedef VoidVoidNative = ffi.Void Function();
typedef VoidVoidDart = void Function();

/// 取消音频导出。
final VoidVoidDart exportAudioCancel = bridge
    .lookupFunction<VoidVoidNative, VoidVoidDart>('yinhe_export_audio_cancel');

typedef ExportMidiStartNative = ffi.Int32 Function(
  ffi.Pointer<Utf8>,
  ffi.Pointer<Utf8>,
);
typedef ExportMidiStartDart = int Function(
  ffi.Pointer<Utf8>,
  ffi.Pointer<Utf8>,
);

/// 开始导出 MIDI（路径 + 选项 JSON）。
final ExportMidiStartDart exportMidiStart = bridge
    .lookupFunction<ExportMidiStartNative, ExportMidiStartDart>(
      'yinhe_export_midi_start',
    );

/// MIDI 导出状态 JSON。
final CStrDart exportMidiStatusJson = bridge
    .lookupFunction<CStrNative, CStrDart>('yinhe_export_midi_status_json');
