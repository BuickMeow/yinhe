import 'package:flutter/material.dart';

/// LIGHT 主题（与 yinhe-theme::base::BaseColors::LIGHT 同步）。
class YinheTheme {
  static const bg = Color(0xFFF0F0F4);
  static const text = Color(0xFF1E1E22);
  static const accent = Color(0xFF1E6EDC);
  static const danger = Color(0xFFE81123);
  static const warning = Color(0xFFF0C83C);

  static const _black = Color(0xFF000000);

  static Color get controlBg => Color.lerp(bg, _black, 0.03) ?? bg;
  static Color get controlSelectedBg => Color.lerp(bg, _black, 0.08) ?? bg;
  static Color get btnBg => Color.lerp(bg, _black, 0.05) ?? bg;
  static Color get lineFg => Color.lerp(bg, _black, 0.07) ?? bg;
  static Color get stripeBg => Color.lerp(bg, _black, 0.06) ?? bg;
  static Color get textSecondary => Color.lerp(text, bg, 0.14) ?? text;
  static Color get textMuted => Color.lerp(text, bg, 0.22) ?? text;
  static Color get textLabel => Color.lerp(text, bg, 0.36) ?? text;
  static Color get textDisabled => Color.lerp(text, bg, 0.66) ?? text;
  static Color get textFaint => Color.lerp(text, bg, 0.9) ?? text;
  static Color get selectedBg => Color.lerp(bg, accent, 0.30) ?? bg;
  static Color get primaryButtonBg =>
      Color.lerp(accent, Colors.white, 0.72) ?? accent;

  static ThemeData data() {
    final scheme = ColorScheme.fromSeed(
      seedColor: accent,
      brightness: Brightness.light,
    ).copyWith(primary: accent, surface: bg);
    return ThemeData(
      useMaterial3: true,
      colorScheme: scheme,
      scaffoldBackgroundColor: bg,
      splashFactory: InkSparkle.splashFactory,
    );
  }
}
