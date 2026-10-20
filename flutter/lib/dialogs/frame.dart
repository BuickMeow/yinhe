import 'package:flutter/material.dart';

import '../core/theme.dart';

/// 统一主题的对话框外壳（背景色 / 标题字号 / 顶部留白）。
///
/// 顶部留白是必须的：Material 的浮动 label 画在输入框上缘之上，
/// 内容贴顶时会被裁掉一半。
class YinheDialog extends StatelessWidget {
  const YinheDialog({
    super.key,
    required this.title,
    required this.content,
    this.titleTrailing,
    this.actions = const [],
    this.width = 360,
    this.scroll = true,
  });

  final String title;
  final Widget content;
  final Widget? titleTrailing;
  final List<Widget> actions;
  final double width;

  /// 内容是否包一层滚动视图（固定高度的自定义内容传 false）。
  final bool scroll;

  @override
  Widget build(BuildContext context) {
    // 键盘弹出时只移动对话框，不改变它的布局尺寸：
    // 1. 对对话框子树去掉 viewInsets，AlertDialog 不会收缩/加键盘内边距；
    // 2. 用 Transform 把整块对话框上移键盘高度的一半（居中于键盘上方），
    //    Transform 不参与 layout，比每帧改 constraints 便宜得多。
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
      title: titleTrailing == null
          ? Text(title, style: const TextStyle(fontSize: 16))
          : Row(
              children: [
                Expanded(
                  child: Text(
                    title,
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
      child: Transform.translate(offset: Offset(0, -inset / 2), child: dialog),
    );
  }
}

/// 统一的输入框样式。
InputDecoration dialogInputDecoration(String label, {String? errorText}) {
  return InputDecoration(
    labelText: label,
    border: const OutlineInputBorder(),
    isDense: true,
    errorText: errorText,
  );
}
