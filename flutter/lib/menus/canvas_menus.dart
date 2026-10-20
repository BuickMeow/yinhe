import 'package:flutter/material.dart';

import '../core/theme.dart';
import '../l10n/app_localizations.dart';

/// 长按选框未移动：复制菜单。
Future<void> showSelectionMenu(
  BuildContext context, {
  required VoidCallback onCopy,
}) {
  final l = AppLocalizations.of(context);
  return showModalBottomSheet<void>(
    context: context,
    backgroundColor: YinheTheme.bg,
    builder: (ctx) => SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          ListTile(
            leading: const Icon(Icons.copy),
            title: Text(l.copy),
            onTap: () {
              Navigator.of(ctx).pop();
              onCopy();
            },
          ),
          ListTile(
            leading: const Icon(Icons.close),
            title: Text(l.cancel),
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
  final l = AppLocalizations.of(context);
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
            title: Text(l.cancel),
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
  VoidCallback? onDelete,
  VoidCallback? onEvents,
}) {
  final l = AppLocalizations.of(context);
  return showModalBottomSheet<void>(
    context: context,
    backgroundColor: YinheTheme.bg,
    builder: (ctx) => SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          ListTile(
            leading: const Icon(Icons.edit),
            title: Text(l.editTrackInfo),
            onTap: () {
              Navigator.of(ctx).pop();
              onEdit();
            },
          ),
          if (onEvents != null)
            ListTile(
              leading: const Icon(Icons.format_list_bulleted),
              title: Text(l.eventList),
              onTap: () {
                Navigator.of(ctx).pop();
                onEvents();
              },
            ),
          if (onDelete != null)
            ListTile(
              leading: Icon(Icons.delete_outline, color: YinheTheme.danger),
              title: Text(
                l.deleteTrack,
                style: TextStyle(color: YinheTheme.danger),
              ),
              onTap: () {
                Navigator.of(ctx).pop();
                onDelete();
              },
            ),
          ListTile(
            leading: const Icon(Icons.close),
            title: Text(l.cancel),
            onTap: () => Navigator.of(ctx).pop(),
          ),
        ],
      ),
    ),
  );
}
