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
    // 键盘弹出时的定位：
    // - 手机（短边 < 600）：对话框直接吸顶，只把可用高度让出键盘部分。
    //   不再跟随 viewInsets 移动，避免整个弹窗被输入法的慢动画"顶着"上升
    //   （小米输入法的 inset 动画明显偏长）。
    // - 宽屏：保持原行为，用 Transform 上移键盘高度的一半。
    // 两者都对对话框子树去掉 viewInsets，AlertDialog 不会自己收缩/加内边距。
    final inset = MediaQuery.viewInsetsOf(context).bottom;
    final phone = MediaQuery.sizeOf(context).shortestSide < 600;
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
      alignment: phone ? Alignment.topCenter : null,
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
      child: Padding(
        padding: EdgeInsets.only(
          top: phone ? 12 : 0,
          bottom: phone ? inset : 0,
        ),
        child: Transform.translate(
          offset: phone ? Offset.zero : Offset(0, -inset / 2),
          child: dialog,
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
