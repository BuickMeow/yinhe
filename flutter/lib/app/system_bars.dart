import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../core/theme.dart';

/// 系统栏策略：竖屏与平板横屏显示状态栏/导航栏，普通手机横屏保持沉浸式。
///
/// 放在 `MaterialApp.builder` 里，这样能拿到 MediaQuery，并在旋转/回前台时
/// 重新下发；页面布局用 [visible] 决定是否让出 [MediaQuery.viewPadding]。
class SystemBarsMode extends StatefulWidget {
  const SystemBarsMode({super.key, required this.child});

  final Widget child;

  /// 当前是否显示系统栏。
  ///
  /// 竖屏手机/平板都显示。横屏按「平板形态」判断：短边 >= 600dp
  /// （标准平板断点）或屏幕比窄于 16:9（4:3 / 16:10 平板），
  /// 这样 16:9 及以上比例的横屏手机仍然保持沉浸式。
  static bool visible(BuildContext context) {
    final size = MediaQuery.sizeOf(context);
    if (size.height >= size.width) {
      return true;
    }
    final shortSide = math.min(size.width, size.height);
    final longSide = math.max(size.width, size.height);
    return shortSide >= 600 || longSide / shortSide < 16 / 9;
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
    final dark = Theme.of(context).brightness == Brightness.dark;
    if (_visible != visible) {
      _visible = visible;
      SystemChrome.setEnabledSystemUIMode(
        visible ? SystemUiMode.edgeToEdge : SystemUiMode.immersiveSticky,
      );
    }
    SystemChrome.setSystemUIOverlayStyle(
      SystemUiOverlayStyle(
        // 状态栏底色与顶部栏一致（iOS 忽略 statusBarColor，由工作区顶部
        // 画的同色带兜底；Android 直接用这个颜色）。
        statusBarColor: YinheTheme.controlBg,
        systemNavigationBarColor: Colors.transparent,
        statusBarIconBrightness: dark ? Brightness.light : Brightness.dark,
        systemNavigationBarIconBrightness: dark
            ? Brightness.light
            : Brightness.dark,
        statusBarBrightness: dark ? Brightness.dark : Brightness.light,
      ),
    );
  }

  @override
  Widget build(BuildContext context) => widget.child;
}
