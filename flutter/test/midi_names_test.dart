import 'package:flutter_test/flutter_test.dart';
import 'package:yinhe_flutter/core/midi_names.dart';

/// 名称数据本身由 Rust `yinhe-types` 的测试保证；
/// 这里验证 FFI JSON 的解析与移动端展示格式。
void main() {
  final names = MidiNames.fromJson({
    'gm': {
      'melodic': ['Acoustic Grand Piano', 'Bright Acoustic Piano'],
      'drums': ['Standard Kit', 'Room Kit'],
    },
    'cc': {'7': 'Volume', '11': 'Expression', '64': 'Sustain'},
    'rpn': {'0': 'PB Sensitivity'},
    'notes': ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'],
    'limits': {
      'tempo': 60000000,
      'pitch_bend': 16383,
      'cc': 127,
      'rpn_7bit': 127,
      'rpn_14bit': 16383,
    },
  });

  group('CC/RPN 文案', () {
    test('有名称与无名称', () {
      expect(names.ccLabel(11), 'CC 11 Expression');
      expect(names.ccLabel(64), 'CC 64 Sustain');
      expect(names.ccLabel(99), 'CC 99');
      expect(names.rpnLabel(0), 'RPN 0 PB Sensitivity');
      expect(names.rpnLabel(9), 'RPN 9');
    });
  });

  group('GM 音色', () {
    test('旋律/鼓组/越界', () {
      expect(names.gmProgramLabel(0, 0), 'Acoustic Grand Piano');
      expect(names.gmProgramLabel(0, 1), 'Bright Acoustic Piano');
      expect(names.gmProgramLabel(0, 9), 'Program 9');
      expect(names.gmProgramLabel(128, 0), 'Standard Kit');
      expect(names.gmProgramLabel(128, 9), 'Drum Kit 9');
      expect(names.gmProgramLabel(1, 5), 'Program 5');
    });
  });

  group('音名与上限', () {
    test('移动端八度约定 C4 = MIDI 60', () {
      expect(names.noteName(0), 'C-1');
      expect(names.noteName(60), 'C4');
      expect(names.noteName(61), 'C#4');
      expect(names.noteName(127), 'G9');
    });

    test('插入值上限', () {
      expect(names.displayMax(0, 0), 60000000);
      expect(names.displayMax(1, 0), 16383);
      expect(names.displayMax(2, 11), 127);
      expect(names.displayMax(3, 0), 127);
      expect(names.displayMax(3, 1), 16383);
      expect(names.displayMax(3, 2), 127);
      expect(names.displayMax(3, 9), 16383);
      expect(names.displayMax(99, 0), 127);
    });
  });
}
