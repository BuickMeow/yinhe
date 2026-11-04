import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:yinhe_flutter/core/theme.dart';

void main() {
  test('主题偏好解析与系统明暗解析', () {
    expect(YinheTheme.parseMode('auto'), AppThemeMode.system);
    expect(YinheTheme.parseMode('system'), AppThemeMode.system);
    expect(YinheTheme.parseMode('light'), AppThemeMode.light);
    expect(YinheTheme.parseMode('dark'), AppThemeMode.dark);
    expect(YinheTheme.parseMode('unknown'), AppThemeMode.system);

    expect(
      YinheTheme.resolveDark(AppThemeMode.system, Brightness.dark),
      isTrue,
    );
    expect(
      YinheTheme.resolveDark(AppThemeMode.system, Brightness.light),
      isFalse,
    );
    expect(
      YinheTheme.resolveDark(AppThemeMode.light, Brightness.dark),
      isFalse,
    );
    expect(YinheTheme.resolveDark(AppThemeMode.dark, Brightness.light), isTrue);
  });
}
