import 'package:flutter_test/flutter_test.dart';
import 'package:yinhe_flutter/dialogs/insert_flow.dart';

void main() {
  group('insertDefaultValue', () {
    test('各目标的常用默认值（移动端插入体验，桌面沿用自身策略）', () {
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
