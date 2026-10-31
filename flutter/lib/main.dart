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
  // 主题持久化在 Rust settings；先读出来再建 MaterialApp，避免首帧闪白。
  YinheTheme.setDark(rust.takeString(rust.uiTheme()) == 'dark');
  rust.setTheme(YinheTheme.isDark ? 1 : 0);
  runApp(const YinheApp());
}

class YinheApp extends StatelessWidget {
  const YinheApp({super.key});

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
