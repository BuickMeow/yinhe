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
    // 键盘弹出时对话框上移，但不用键盘高度做任何计算：
    // - 关：居中（t=0）；
    // - 开：对齐到可用区域上部，输入框中心约落在屏幕上 20%（t=1）。
    // 只用 150ms 自有曲线在两者之间过渡，键盘 inset 怎么变、动画多长都不影响
    // 终点；同时避免系统 inset 分段上报时弹窗一顿一顿地上升。
    // 对对话框子树去掉 viewInsets：AlertDialog 不会自己收缩/加内边距。
    final keyboardOpen = MediaQuery.viewInsetsOf(context).bottom > 0;
    final body = scroll
        ? SingleChildScrollView(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [const SizedBox(height: 10), content],
            ),
          )
        : Padding(padding: const EdgeInsets.only(top: 10), child: content);
    return MediaQuery(
      data: MediaQuery.of(context).removeViewInsets(removeBottom: true),
      child: TweenAnimationBuilder<double>(
        tween: Tween<double>(end: keyboardOpen ? 1 : 0),
        duration: const Duration(milliseconds: 150),
        curve: Curves.easeOutCubic,
        builder: (context, t, _) => AlertDialog(
          backgroundColor: YinheTheme.bg,
          // -0.7 = 顶部留 15% 的剩余空间：单行输入弹窗的输入框中心约在
          // 屏幕上 20% 处，多字段弹窗也整体落在键盘上方。
          alignment: Alignment(0, -0.7 * t),
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
        ),
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
