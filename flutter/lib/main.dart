import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'app/studio_shell.dart';
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
  runApp(const YinheApp());
}

class YinheApp extends StatelessWidget {
  const YinheApp({super.key});

  @override
  Widget build(BuildContext context) {
    return ValueListenableBuilder<String>(
      valueListenable: L10n.appLanguage,
      builder: (context, _, _) => MaterialApp(
        debugShowCheckedModeBanner: false,
        theme: YinheTheme.data(),
        locale: L10n.locale,
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: const StudioShell(),
      ),
    );
  }
}
