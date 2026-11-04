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

  test('Material 主题跟随目标明暗，不受过渡进度影响', () {
    // 模拟“深色 → 浅色”过渡仍停在深色起点：ThemeData 不能跟着 _t 变黑。
    YinheTheme.setMode(AppThemeMode.light, Brightness.dark);
    YinheTheme.setTransition(1.0);
    final theme = YinheTheme.data();
    expect(theme.colorScheme.brightness, Brightness.light);
    expect(theme.brightness, Brightness.light);

    YinheTheme.setTransition(0.0);
    YinheTheme.setMode(AppThemeMode.system, Brightness.light);
  });
}
