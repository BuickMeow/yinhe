import 'package:flutter/material.dart';

import '../core/theme.dart';

/// 统一主题的对话框外壳（背景色 / 标题字号 / 顶部留白）。
///
/// 顶部留白是必须的：Material 的浮动 label 画在输入框上缘之上，
/// 内容贴顶时会被裁掉一半。
class YinheDialog extends StatelessWidget {
  const YinheDialog({
    super.key,
    required this.content,
    this.title,
    this.titleTrailing,
    this.actions = const [],
    this.width = 360,
    this.scroll = true,
  });

  final String? title;
  final Widget content;
  final Widget? titleTrailing;
  final List<Widget> actions;
  final double width;

  /// 内容是否包一层滚动视图（固定高度的自定义内容传 false）。
  final bool scroll;

  @override
  Widget build(BuildContext context) {
    // 键盘弹出时对话框居中上移到键盘上方，但位置不用 viewInsets 直接驱动：
    // 系统 inset 在输入法动画期间的上报可能是一段段跳变的"台阶"，直接绑上去
    // 会变成弹窗卡顿/一顿一顿地上升。这里用 150ms 自有曲线平滑跟随目标位置。
    // Transform 不参与 layout，比每帧改 constraints 便宜得多。
    final inset = MediaQuery.viewInsetsOf(context).bottom;
    final body = scroll
        ? SingleChildScrollView(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [const SizedBox(height: 10), content],
            ),
          )
        : Padding(padding: const EdgeInsets.only(top: 10), child: content);
    final dialog = AlertDialog(
      backgroundColor: YinheTheme.bg,
      title: title == null
          ? null
          : titleTrailing == null
          ? Text(title!, style: const TextStyle(fontSize: 16))
          : Row(
              children: [
                Expanded(
                  child: Text(
                    title!,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(fontSize: 16),
                  ),
                ),
                titleTrailing!,
              ],
            ),
      content: SizedBox(width: width, child: body),
      actions: actions,
    );
    return MediaQuery(
      data: MediaQuery.of(context).removeViewInsets(removeBottom: true),
      child: TweenAnimationBuilder<double>(
        tween: Tween<double>(end: -inset / 2),
        duration: const Duration(milliseconds: 150),
        curve: Curves.easeOutCubic,
        builder: (context, dy, child) =>
            Transform.translate(offset: Offset(0, dy), child: child),
        child: dialog,
      ),
    );
  }
}

/// 统一的输入框样式。
InputDecoration dialogInputDecoration(
  String label, {
  String? errorText,
  String? hintText,
}) {
  return InputDecoration(
    labelText: label,
    hintText: hintText,
    border: const OutlineInputBorder(),
    isDense: true,
    errorText: errorText,
  );
}
