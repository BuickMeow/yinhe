import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'app/studio_shell.dart';
import 'app/system_bars.dart';
import 'core/ffi.dart' as rust;
import 'core/theme.dart';
import 'l10n.dart';
import 'l10n/app_localizations.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  // 沉浸式全屏：隐藏顶部状态栏与底部小白条（滑动可临时唤出）。
  SystemChrome.setEnabledSystemUIMode(SystemUiMode.immersiveSticky);
  SystemChrome.setSystemUIOverlayStyle(
    const SystemUiOverlayStyle(
      statusBarColor: Colors.transparent,
      systemNavigationBarColor: Colors.transparent,
    ),
  );
  // 主题偏好（auto/light/dark）持久化在 Rust settings；先解析系统亮度再建
  // MaterialApp，避免首帧闪白。
  final mode = YinheTheme.parseMode(rust.takeString(rust.uiTheme()));
  YinheTheme.setMode(
    mode,
    WidgetsBinding.instance.platformDispatcher.platformBrightness,
  );
  YinheTheme.setTransition(YinheTheme.isDark ? 1 : 0);
  rust.setTheme(YinheTheme.isDark ? 1 : 0);
  runApp(const YinheApp());
}

class YinheApp extends StatefulWidget {
  const YinheApp({super.key});

  @override
  State<YinheApp> createState() => _YinheAppState();
}

class _YinheAppState extends State<YinheApp>
    with WidgetsBindingObserver, SingleTickerProviderStateMixin {
  late final AnimationController _themeAnim;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _themeAnim = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 220),
      value: YinheTheme.transitionValue,
    )..addListener(() => YinheTheme.setTransition(_themeAnim.value));
    YinheTheme.darkMode.addListener(_onThemeTargetChanged);
  }

  @override
  void dispose() {
    YinheTheme.darkMode.removeListener(_onThemeTargetChanged);
    _themeAnim.dispose();
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  void _onThemeTargetChanged() {
    final target = YinheTheme.isDark ? 1.0 : 0.0;
    if ((YinheTheme.transitionValue - target).abs() <= 0.001) {
      YinheTheme.setTransition(target);
      return;
    }
    _themeAnim.stop();
    _themeAnim.value = YinheTheme.transitionValue;
    _themeAnim.animateTo(
      target,
      duration: const Duration(milliseconds: 220),
      curve: Curves.easeInOutCubic,
    );
  }

  @override
  void didChangePlatformBrightness() {
    final changed = YinheTheme.applySystemBrightness(
      WidgetsBinding.instance.platformDispatcher.platformBrightness,
    );
    if (changed) {
      rust.setTheme(YinheTheme.isDark ? 1 : 0);
    }
  }

  @override
  Widget build(BuildContext context) {
    return ValueListenableBuilder<String>(
      valueListenable: L10n.appLanguage,
      builder: (context, _, _) => ValueListenableBuilder<bool>(
        valueListenable: YinheTheme.darkMode,
        builder: (context, _, _) => MaterialApp(
          debugShowCheckedModeBanner: false,
          theme: YinheTheme.data(),
          locale: L10n.locale,
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          // 竖屏显示系统栏，横屏沉浸式；页面据此给系统栏让位。
          builder: (context, child) => SystemBarsMode(child: child!),
          home: const StudioShell(),
        ),
      ),
    );
  }
}
