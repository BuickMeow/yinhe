import 'package:flutter_test/flutter_test.dart';
import 'package:yinhe_flutter/core/tools.dart';

void main() {
  test('PR 两个工具下拉框使用同一套工具', () {
    expect(quickToolsPr, toolPrAllowed);
    expect(quickToolsPr.contains(defaultQuickToolPr), isTrue);
  });

  test('AR 两个工具下拉框使用同一套工具', () {
    expect(quickToolsAr, toolArAllowed);
  });
}
