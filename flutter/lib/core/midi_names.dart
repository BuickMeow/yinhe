import 'dart:convert';
import 'dart:ffi' as ffi;

import 'package:ffi/ffi.dart';

import 'ffi.dart' as rust;

/// MIDI 静态名称表（GM 音色/鼓组、CC/RPN、音名、插入值上限）。
///
/// 数据源在 Rust `yinhe-types`，这里只保留一份只读快照并负责展示格式化。
/// 名称不随工程/语言变化，第一次使用时读取一次。
class MidiNames {
  MidiNames._({
    required this.gmMelodic,
    required this.gmDrums,
    required this.cc,
    required this.rpn,
    required this.notes,
    required this.limits,
  });

  factory MidiNames.fromJson(Map<String, dynamic> json) {
    List<String> strList(dynamic v) => [
      for (final e in v as List? ?? const []) e as String,
    ];
    Map<int, String> strMap(dynamic v) => {
      for (final e in (v as Map<String, dynamic>? ?? const {}).entries)
        int.parse(e.key): e.value as String,
    };
    final gm = json['gm'] as Map<String, dynamic>? ?? const {};
    return MidiNames._(
      gmMelodic: strList(gm['melodic']),
      gmDrums: strList(gm['drums']),
      cc: strMap(json['cc']),
      rpn: strMap(json['rpn']),
      notes: strList(json['notes']),
      limits: InsertLimits.fromJson(
        json['limits'] as Map<String, dynamic>? ?? const {},
      ),
    );
  }

  /// 本地库未加载时的兜底：只影响名称显示，不影响编辑功能。
  static final MidiNames empty = MidiNames._(
    gmMelodic: const [],
    gmDrums: const [],
    cc: const {},
    rpn: const {},
    notes: const [
      'C',
      'C#',
      'D',
      'D#',
      'E',
      'F',
      'F#',
      'G',
      'G#',
      'A',
      'A#',
      'B',
    ],
    limits: const InsertLimits(
      tempo: 60000000,
      pitchBend: 16383,
      cc: 127,
      rpn7Bit: 127,
      rpn14Bit: 16383,
    ),
  );

  static MidiNames? _instance;

  static MidiNames get instance => _instance ??= _load();

  static MidiNames _load() {
    final ptr = rust.midiNamesJson();
    if (ptr == ffi.nullptr) {
      return empty;
    }
    try {
      return MidiNames.fromJson(
        jsonDecode(ptr.toDartString()) as Map<String, dynamic>,
      );
    } catch (_) {
      return empty;
    } finally {
      rust.freeString(ptr);
    }
  }

  final List<String> gmMelodic;
  final List<String> gmDrums;
  final Map<int, String> cc;
  final Map<int, String> rpn;
  final List<String> notes;
  final InsertLimits limits;

  /// 移动端文案风格：`CC 11 Expression`（不补零，与桌面 `CC 011` 不同）。
  String ccLabel(int controller) {
    final name = cc[controller];
    return name == null || name.isEmpty
        ? 'CC $controller'
        : 'CC $controller $name';
  }

  /// 移动端文案风格：`RPN 0 PB Sensitivity`。
  String rpnLabel(int parameter) {
    final name = rpn[parameter];
    return name == null || name.isEmpty
        ? 'RPN $parameter'
        : 'RPN $parameter $name';
  }

  /// GM 显示名：Bank 0 旋律音色、Bank 128 鼓组，其余显示编号。
  String gmProgramLabel(int bank, int program) {
    if (bank == 128) {
      return program >= 0 && program < gmDrums.length
          ? gmDrums[program]
          : 'Drum Kit $program';
    }
    if (bank == 0 && program >= 0 && program < gmMelodic.length) {
      return gmMelodic[program];
    }
    return 'Program $program';
  }

  /// 音名：移动端沿用 C4 = MIDI 60（桌面键盘用 C5 = 60）。
  String noteName(int key) {
    final pitchClass = notes.isEmpty ? '$key' : notes[key % 12];
    return '$pitchClass${key ~/ 12 - 1}';
  }

  /// 插入值上限：kind 0=Tempo、1=PB、2=CC、3=RPN。
  int displayMax(int kind, int number) {
    final l = limits;
    return switch (kind) {
      0 => l.tempo,
      1 => l.pitchBend,
      2 => l.cc,
      3 => (number == 0 || number == 2) ? l.rpn7Bit : l.rpn14Bit,
      _ => l.cc,
    };
  }
}

/// 插入值域上限（与 Rust `AutomationTarget::display_max` 同规则）。
class InsertLimits {
  const InsertLimits({
    required this.tempo,
    required this.pitchBend,
    required this.cc,
    required this.rpn7Bit,
    required this.rpn14Bit,
  });

  factory InsertLimits.fromJson(Map<String, dynamic> json) => InsertLimits(
    tempo: (json['tempo'] as num).toInt(),
    pitchBend: (json['pitch_bend'] as num).toInt(),
    cc: (json['cc'] as num).toInt(),
    rpn7Bit: (json['rpn_7bit'] as num).toInt(),
    rpn14Bit: (json['rpn_14bit'] as num).toInt(),
  );

  final int tempo;
  final int pitchBend;
  final int cc;
  final int rpn7Bit;
  final int rpn14Bit;
}
