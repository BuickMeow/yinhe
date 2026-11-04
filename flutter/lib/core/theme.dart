import 'package:flutter/material.dart';

/// 用户选择的主题偏好。
enum AppThemeMode { system, light, dark }

/// LIGHT/DARK 主题（与 yinhe-theme::base::BaseColors 同步）。
class YinheTheme {
  /// 主题偏好：跟随系统 / 浅色 / 深色。
  static final ValueNotifier<AppThemeMode> mode = ValueNotifier(
    AppThemeMode.system,
  );

  /// 解析后的实际明暗。切换时 `YinheApp` 监听并重建 MaterialApp。
  static final ValueNotifier<bool> darkMode = ValueNotifier(false);
  static AppThemeMode _mode = AppThemeMode.system;
  static bool _dark = false;
  static double _t = 0.0;

  /// 主题过渡进度：0 = 浅色，1 = 深色。StudioShell 监听它逐帧重建。
  static final ValueNotifier<double> transition = ValueNotifier(0.0);

  static double get transitionValue => _t;

  static void setTransition(double value) {
    final t = value.clamp(0.0, 1.0);
    _t = t;
    if ((transition.value - t).abs() > 0.0001) {
      transition.value = t;
    }
  }

  static AppThemeMode get currentMode => _mode;

  static bool get isDark => _dark;

  static AppThemeMode parseMode(String value) {
    return switch (value.trim().toLowerCase()) {
      'dark' => AppThemeMode.dark,
      'light' => AppThemeMode.light,
      _ => AppThemeMode.system,
    };
  }

  static String modeName(AppThemeMode value) {
    return switch (value) {
      AppThemeMode.system => 'auto',
      AppThemeMode.light => 'light',
      AppThemeMode.dark => 'dark',
    };
  }

  static bool resolveDark(AppThemeMode value, Brightness platformBrightness) {
    return switch (value) {
      AppThemeMode.system => platformBrightness == Brightness.dark,
      AppThemeMode.light => false,
      AppThemeMode.dark => true,
    };
  }

  static void setMode(AppThemeMode value, Brightness platformBrightness) {
    _mode = value;
    if (mode.value != value) {
      mode.value = value;
    }
    setDark(resolveDark(value, platformBrightness));
  }

  /// 系统亮度变化：仅“跟随系统”时生效，返回实际明暗是否变化。
  static bool applySystemBrightness(Brightness platformBrightness) {
    if (_mode != AppThemeMode.system) {
      return false;
    }
    return setDark(resolveDark(_mode, platformBrightness));
  }

  static bool setDark(bool value) {
    final changed = _dark != value;
    _dark = value;
    if (darkMode.value != value) {
      darkMode.value = value;
    }
    return changed;
  }

  static const _lightBg = Color(0xFFF0F0F4);
  static const _darkBg = Color(0xFF17171C);
  static const _lightText = Color(0xFF1E1E22);
  static const _darkText = Color(0xFFE8E8EE);
  static const _lightAccent = Color(0xFF1E6EDC);
  static const _darkAccent = Color(0xFF5B95F8);
  static const _black = Color(0xFF000000);
  static const _white = Color(0xFFFFFFFF);

  /// 在浅色/深色两套色之间按过渡进度插值。
  static Color _pair(Color light, Color dark) =>
      Color.lerp(light, dark, _t) ?? light;

  static Color _shade(Color base, Color toward, double amount) =>
      Color.lerp(base, toward, amount) ?? base;

  static Color get bg => _pair(_lightBg, _darkBg);
  static Color get text => _pair(_lightText, _darkText);
  static Color get accent => _pair(_lightAccent, _darkAccent);
  static Color get danger =>
      _pair(const Color(0xFFE81123), const Color(0xFFFF5C68));
  static Color get warning =>
      _pair(const Color(0xFFF0C83C), const Color(0xFFFFD75A));

  static Color get controlBg =>
      _pair(_shade(_lightBg, _black, 0.03), _shade(_darkBg, _white, 0.06));
  static Color get controlSelectedBg =>
      _pair(_shade(_lightBg, _black, 0.08), _shade(_darkBg, _white, 0.12));
  static Color get btnBg =>
      _pair(_shade(_lightBg, _black, 0.05), _shade(_darkBg, _white, 0.09));
  static Color get lineFg =>
      _pair(_shade(_lightBg, _black, 0.07), _shade(_darkBg, _white, 0.12));
  static Color get stripeBg =>
      _pair(_shade(_lightBg, _black, 0.06), _shade(_darkBg, _white, 0.05));
  static Color get trackBg =>
      _pair(_shade(_lightBg, _black, 0.12), _shade(_darkBg, _white, 0.14));
  static Color get textSecondary => _pair(
    _shade(_lightText, _lightBg, 0.14),
    _shade(_darkText, _darkBg, 0.14),
  );
  static Color get textMuted => _pair(
    _shade(_lightText, _lightBg, 0.22),
    _shade(_darkText, _darkBg, 0.22),
  );
  static Color get textLabel => _pair(
    _shade(_lightText, _lightBg, 0.36),
    _shade(_darkText, _darkBg, 0.36),
  );
  static Color get textDisabled => _pair(
    _shade(_lightText, _lightBg, 0.66),
    _shade(_darkText, _darkBg, 0.66),
  );
  static Color get textFaint =>
      _pair(_shade(_lightText, _lightBg, 0.9), _shade(_darkText, _darkBg, 0.9));
  static Color get measureLabel => _pair(
    _shade(_lightText, _lightBg, 0.12),
    _shade(_darkText, _darkBg, 0.12),
  );
  static Color get tickLabel => _pair(
    _shade(_lightBg, _lightText, 0.22),
    _shade(_darkBg, _darkText, 0.22),
  );
  static Color get keyWhite =>
      _pair(const Color(0xFFF7F7F7), const Color(0xFFD8D8E0));
  static Color get keyBlack =>
      _pair(const Color(0xFF333333), const Color(0xFF2A2A31));
  static Color get selectedBg => _pair(
    _shade(_lightBg, _lightAccent, 0.30),
    _shade(_darkBg, _darkAccent, 0.38),
  );
  static Color get primaryButtonBg => _pair(
    _shade(_lightAccent, _white, 0.72),
    _shade(_darkAccent, _black, 0.30),
  );

  static ThemeData data() {
    // M3 的 selected 容器（NavigationRail 指示器、ChoiceChip 选中态等）
    // 默认用 fromSeed 生成的 secondaryContainer；这里统一改成和主界面
    // 工具按钮同源的 selectedBg，避免设置页出现另一套淡紫色。
    final scheme =
        ColorScheme.fromSeed(
          seedColor: accent,
          brightness: _dark ? Brightness.dark : Brightness.light,
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
