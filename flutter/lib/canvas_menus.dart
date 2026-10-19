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

/// 空白处长按完成：粘贴 / 插入 菜单。
Future<void> showEmptyLongPressMenu(
  BuildContext context, {
  required bool canPaste,
  required VoidCallback onPaste,
  required VoidCallback? onInsert,
}) {
  return showModalBottomSheet<void>(
    context: context,
    backgroundColor: YinheTheme.bg,
    builder: (ctx) => SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          ListTile(
            enabled: canPaste,
            leading: const Icon(Icons.paste),
            title: const Text('粘贴'),
            onTap: () {
              Navigator.of(ctx).pop();
              onPaste();
            },
          ),
          ListTile(
            enabled: onInsert != null,
            leading: const Icon(Icons.add_circle_outline),
            title: const Text('插入…'),
            onTap: onInsert == null
                ? null
                : () {
                    Navigator.of(ctx).pop();
                    onInsert();
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

/// 插入事件类型：kind 与 Rust `yinhe_insert_automation` 一致。
Future<void> showInsertMenu(
  BuildContext context, {
  required bool conductor,
  required ValueChanged<int> onPick,
}) {
  // (kind, 图标, 名称)
  final items = conductor
      ? const <(int, IconData, String)>[(0, Icons.speed, '速度（Tempo）')]
      : const <(int, IconData, String)>[
          (1, Icons.swap_vert, 'Pitch Bend'),
          (2, Icons.tune, 'CC1 调制轮'),
          (3, Icons.volume_up, 'CC7 音量'),
          (4, Icons.balance, 'CC10 声像'),
          (5, Icons.emoji_emotions_outlined, 'CC11 表情'),
          (6, Icons.piano, 'CC64 延音'),
        ];
  return showModalBottomSheet<void>(
    context: context,
    backgroundColor: YinheTheme.bg,
    isScrollControlled: true,
    builder: (ctx) => SafeArea(
      child: ConstrainedBox(
        constraints: BoxConstraints(
          maxHeight: MediaQuery.sizeOf(ctx).height * 0.7,
        ),
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              for (final (kind, icon, label) in items)
                ListTile(
                  leading: Icon(icon),
                  title: Text(label),
                  onTap: () {
                    Navigator.of(ctx).pop();
                    onPick(kind);
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
