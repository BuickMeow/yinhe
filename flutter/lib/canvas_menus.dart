import 'package:flutter/material.dart';

import 'theme.dart';

/// 长按选框未移动：复制菜单。
Future<void> showSelectionMenu(
  BuildContext context, {
  required VoidCallback onCopy,
}) {
  return showModalBottomSheet<void>(
    context: context,
    backgroundColor: YinheTheme.bg,
    builder: (ctx) => SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          ListTile(
            leading: const Icon(Icons.copy),
            title: const Text('复制'),
            onTap: () {
              Navigator.of(ctx).pop();
              onCopy();
            },
          ),
          ListTile(
            leading: const Icon(Icons.close),
            title: const Text('取消'),
            onTap: () => Navigator.of(ctx).pop(),
          ),
        ],
      ),
    ),
  );
}

/// 空白处长按完成：粘贴菜单（剪贴板空时置灰）。
Future<void> showPasteMenu(
  BuildContext context, {
  required bool enabled,
  required VoidCallback onPaste,
}) {
  return showModalBottomSheet<void>(
    context: context,
    backgroundColor: YinheTheme.bg,
    builder: (ctx) => SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          ListTile(
            enabled: enabled,
            leading: const Icon(Icons.paste),
            title: const Text('粘贴'),
            onTap: () {
              Navigator.of(ctx).pop();
              onPaste();
            },
          ),
          ListTile(
            leading: const Icon(Icons.close),
            title: const Text('取消'),
            onTap: () => Navigator.of(ctx).pop(),
          ),
        ],
      ),
    ),
  );
}

/// AR 长按音轨（未拖动）：音轨信息菜单。
Future<void> showTrackMenu(
  BuildContext context, {
  required VoidCallback onEdit,
}) {
  return showModalBottomSheet<void>(
    context: context,
    backgroundColor: YinheTheme.bg,
    builder: (ctx) => SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          ListTile(
            leading: const Icon(Icons.edit),
            title: const Text('编辑音轨信息'),
            onTap: () {
              Navigator.of(ctx).pop();
              onEdit();
            },
          ),
          ListTile(
            leading: const Icon(Icons.close),
            title: const Text('取消'),
            onTap: () => Navigator.of(ctx).pop(),
          ),
        ],
      ),
    ),
  );
}

/// 重命名音轨对话框。
Future<void> showRenameTrackDialog(
  BuildContext context, {
  required String initialName,
  required ValueChanged<String> onSave,
}) {
  final controller = TextEditingController(text: initialName);
  // 对话框关闭动画期间 TextField 仍持有 controller：延迟到完全关闭后释放。
  return showDialog<void>(
    context: context,
    builder: (ctx) => AlertDialog(
      backgroundColor: YinheTheme.bg,
      title: const Text('编辑音轨信息', style: TextStyle(fontSize: 17)),
      content: TextField(
        controller: controller,
        style: const TextStyle(fontSize: 14),
        decoration: const InputDecoration(
          labelText: '音轨名称',
          border: OutlineInputBorder(),
          isDense: true,
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(ctx).pop(),
          child: const Text('取消'),
        ),
        FilledButton(
          onPressed: () {
            final name = controller.text.trim();
            Navigator.of(ctx).pop();
            onSave(name);
          },
          child: const Text('保存'),
        ),
      ],
    ),
  ).whenComplete(() {
    Future<void>.delayed(const Duration(milliseconds: 300), controller.dispose);
  });
}
