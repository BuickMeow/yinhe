import 'package:flutter/material.dart';

/// LIGHT 主题（与 yinhe-theme::base::BaseColors::LIGHT 同步）。
class YinheTheme {
  /// 全局亮/暗主题。切换时 `YinheApp` 监听并重建 MaterialApp。
  static final ValueNotifier<bool> darkMode = ValueNotifier(false);
  static bool _dark = false;

  static bool get isDark => _dark;

  static void setDark(bool value) {
    _dark = value;
    if (darkMode.value != value) {
      darkMode.value = value;
    }
  }

  static const _lightBg = Color(0xFFF0F0F4);
  static const _darkBg = Color(0xFF17171C);
  static const _lightText = Color(0xFF1E1E22);
  static const _darkText = Color(0xFFE8E8EE);
  static const _lightAccent = Color(0xFF1E6EDC);
  static const _darkAccent = Color(0xFF5B95F8);
  static const _black = Color(0xFF000000);
  static const _white = Color(0xFFFFFFFF);

  static Color get bg => _dark ? _darkBg : _lightBg;
  static Color get text => _dark ? _darkText : _lightText;
  static Color get accent => _dark ? _darkAccent : _lightAccent;
  static Color get danger =>
      _dark ? const Color(0xFFFF5C68) : const Color(0xFFE81123);
  static Color get warning =>
      _dark ? const Color(0xFFFFD75A) : const Color(0xFFF0C83C);

  static Color get controlBg =>
      Color.lerp(bg, _dark ? _white : _black, _dark ? 0.06 : 0.03) ?? bg;
  static Color get controlSelectedBg =>
      Color.lerp(bg, _dark ? _white : _black, _dark ? 0.12 : 0.08) ?? bg;
  static Color get btnBg =>
      Color.lerp(bg, _dark ? _white : _black, _dark ? 0.09 : 0.05) ?? bg;
  static Color get lineFg =>
      Color.lerp(bg, _dark ? _white : _black, _dark ? 0.12 : 0.07) ?? bg;
  static Color get stripeBg =>
      Color.lerp(bg, _dark ? _white : _black, _dark ? 0.05 : 0.06) ?? bg;
  static Color get trackBg =>
      Color.lerp(bg, _dark ? _white : _black, _dark ? 0.14 : 0.12) ?? bg;
  static Color get textSecondary => Color.lerp(text, bg, 0.14) ?? text;
  static Color get textMuted => Color.lerp(text, bg, 0.22) ?? text;
  static Color get textLabel => Color.lerp(text, bg, 0.36) ?? text;
  static Color get textDisabled => Color.lerp(text, bg, 0.66) ?? text;
  static Color get textFaint => Color.lerp(text, bg, 0.9) ?? text;
  static Color get measureLabel => Color.lerp(text, bg, 0.12) ?? text;
  static Color get tickLabel => Color.lerp(bg, text, 0.22) ?? text;
  static Color get keyWhite =>
      _dark ? const Color(0xFFD8D8E0) : const Color(0xFFF7F7F7);
  static Color get keyBlack =>
      _dark ? const Color(0xFF2A2A31) : const Color(0xFF333333);
  static Color get selectedBg =>
      Color.lerp(bg, accent, _dark ? 0.38 : 0.30) ?? bg;
  static Color get primaryButtonBg => _dark
      ? Color.lerp(accent, _black, 0.30) ?? accent
      : Color.lerp(accent, _white, 0.72) ?? accent;

  static ThemeData data() {
    // M3 的 selected 容器（NavigationRail 指示器、ChoiceChip 选中态等）
    // 默认用 fromSeed 生成的 secondaryContainer；这里统一改成和主界面
    // 工具按钮同源的 selectedBg，避免设置页出现另一套淡紫色。
    final scheme =
        ColorScheme.fromSeed(
          seedColor: accent,
          brightness: Brightness.light,
        ).copyWith(
          primary: accent,
          onPrimary: Colors.white,
          secondary: accent,
          onSecondary: Colors.white,
          secondaryContainer: selectedBg,
          onSecondaryContainer: accent,
          surface: bg,
        );
    return ThemeData(
      useMaterial3: true,
      colorScheme: scheme,
      scaffoldBackgroundColor: bg,
      splashFactory: InkSparkle.splashFactory,
      // 弹出菜单强制不透明底色：避免首帧只看到文字、背景晚一帧才出现的观感。
      popupMenuTheme: PopupMenuThemeData(
        color: bg,
        surfaceTintColor: Colors.transparent,
        elevation: 2,
      ),
      navigationRailTheme: NavigationRailThemeData(
        backgroundColor: bg,
        indicatorColor: selectedBg,
        selectedIconTheme: IconThemeData(color: accent),
        unselectedIconTheme: IconThemeData(color: textSecondary),
        selectedLabelTextStyle: TextStyle(color: accent, fontSize: 13),
        unselectedLabelTextStyle: TextStyle(color: text, fontSize: 13),
      ),
      navigationBarTheme: NavigationBarThemeData(
        backgroundColor: controlBg,
        indicatorColor: selectedBg,
        iconTheme: WidgetStateProperty.resolveWith((states) {
          final selected = states.contains(WidgetState.selected);
          return IconThemeData(color: selected ? accent : textSecondary);
        }),
        labelTextStyle: WidgetStateProperty.resolveWith((states) {
          final selected = states.contains(WidgetState.selected);
          return TextStyle(color: selected ? accent : text, fontSize: 12);
        }),
      ),
    );
  }
}
