import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

/// 系统栏策略：竖屏显示状态栏/导航栏（小白条或三键），横屏保持沉浸式。
///
/// 放在 `MaterialApp.builder` 里，这样能拿到 MediaQuery，并在旋转/回前台时
/// 重新下发；页面布局用 [visible] 决定是否让出 [MediaQuery.viewPadding]。
class SystemBarsMode extends StatefulWidget {
  const SystemBarsMode({super.key, required this.child});

  final Widget child;

  /// 当前是否显示系统栏（竖屏）。
  static bool visible(BuildContext context) {
    final size = MediaQuery.sizeOf(context);
    return size.height >= size.width;
  }

  @override
  State<SystemBarsMode> createState() => _SystemBarsModeState();
}

class _SystemBarsModeState extends State<SystemBarsMode>
    with WidgetsBindingObserver {
  bool? _visible;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _apply();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) {
      // 系统可能已重置系统栏，回到前台按当前方向重新应用。
      _visible = null;
      _apply();
    }
  }

  void _apply() {
    final visible = SystemBarsMode.visible(context);
    if (_visible == visible) {
      return;
    }
    _visible = visible;
    SystemChrome.setEnabledSystemUIMode(
      visible ? SystemUiMode.edgeToEdge : SystemUiMode.immersiveSticky,
    );
    SystemChrome.setSystemUIOverlayStyle(
      const SystemUiOverlayStyle(
        statusBarColor: Colors.transparent,
        systemNavigationBarColor: Colors.transparent,
        statusBarIconBrightness: Brightness.dark,
        systemNavigationBarIconBrightness: Brightness.dark,
        statusBarBrightness: Brightness.light,
      ),
    );
  }

  @override
  Widget build(BuildContext context) => widget.child;
}
