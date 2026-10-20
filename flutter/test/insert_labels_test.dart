import 'package:flutter_test/flutter_test.dart';
import 'package:yinhe_flutter/dialogs/insert_flow.dart';

void main() {
  group('插入文案', () {
    test('CC 名称', () {
      expect(ccInsertLabel(11), 'CC 11 Expression');
      expect(ccInsertLabel(64), 'CC 64 Sustain');
      expect(ccInsertLabel(99), 'CC 99');
    });

    test('RPN 名称', () {
      expect(rpnInsertLabel(0), 'RPN 0 Pitch Bend Sensitivity');
      expect(rpnInsertLabel(2), 'RPN 2 Coarse Tune');
      expect(rpnInsertLabel(9), 'RPN 9');
    });
  });

  group('insertDisplayMax', () {
    test('按目标类型返回上限', () {
      expect(insertDisplayMax(0, 0), 60000000); // Tempo
      expect(insertDisplayMax(1, 0), 16383); // Pitch Bend
      expect(insertDisplayMax(2, 7), 127); // CC
      expect(insertDisplayMax(3, 0), 127); // RPN 0
      expect(insertDisplayMax(3, 1), 16383); // RPN 1
      expect(insertDisplayMax(3, 2), 127); // RPN 2
      expect(insertDisplayMax(3, 9), 16383); // 自定义 RPN
      expect(insertDisplayMax(99, 0), 127);
    });
  });

  group('insertDefaultValue', () {
    test('各目标的常用默认值', () {
      expect(insertDefaultValue(0, 0), 120);
      expect(insertDefaultValue(1, 0), 8192);
      expect(insertDefaultValue(2, 7), 100);
      expect(insertDefaultValue(2, 10), 64);
      expect(insertDefaultValue(2, 11), 127);
      expect(insertDefaultValue(2, 99), 0);
      expect(insertDefaultValue(3, 0), 2);
      expect(insertDefaultValue(3, 1), 8192);
      expect(insertDefaultValue(3, 2), 64);
      expect(insertDefaultValue(3, 9), 0);
      expect(insertDefaultValue(99, 0), 0);
    });
  });
}
