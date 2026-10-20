import 'package:flutter_test/flutter_test.dart';
import 'package:yinhe_flutter/core/quantize_presets.dart';

/// 档位列表由 Rust `presets_for_ppq` 生成；这里只验证 JSON 解析。
void main() {
  test('解析 label/ticks', () {
    expect(
      decodeQuantizePresets(
        '[{"label":"1/1","ticks":1920},{"label":"1/8","ticks":240}]',
      ),
      [('1/1', 1920), ('1/8', 240)],
    );
  });

  test('空表/非法数据返回空', () {
    expect(decodeQuantizePresets('[]'), isEmpty);
    expect(decodeQuantizePresets('not json'), isEmpty);
    expect(decodeQuantizePresets('{"label":"1/4"}'), isEmpty);
    expect(
      decodeQuantizePresets('[{"label":1,"ticks":"x"},{"label":"1/4"}]'),
      isEmpty,
    );
  });
}
