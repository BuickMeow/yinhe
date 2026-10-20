import 'package:flutter/material.dart';

import '../dialogs/sheet.dart';
import '../l10n/app_localizations.dart';

/// 长按选框未移动：复制菜单。
Future<void> showSelectionMenu(
  BuildContext context, {
  required VoidCallback onCopy,
}) async {
  final l = AppLocalizations.of(context);
  final action = await showActionSheet(
    context,
    cancelLabel: l.cancel,
    actions: [SheetAction(value: 'copy', label: l.copy, icon: Icons.copy)],
  );
  if (action == 'copy') {
    onCopy();
  }
}

/// 空白处长按完成：粘贴 / 插入 菜单。
Future<void> showEmptyLongPressMenu(
  BuildContext context, {
  required bool canPaste,
  required VoidCallback onPaste,
  required VoidCallback? onInsert,
}) async {
  final l = AppLocalizations.of(context);
  final action = await showActionSheet(
    context,
    cancelLabel: l.cancel,
    actions: [
      SheetAction(
        value: 'paste',
        label: l.paste,
        icon: Icons.content_paste,
        enabled: canPaste,
      ),
      if (onInsert != null)
        SheetAction(
          value: 'insert',
          label: l.insert,
          icon: Icons.add_circle_outline,
        ),
    ],
  );
  if (action == 'paste') {
    onPaste();
  } else if (action == 'insert') {
    onInsert?.call();
  }
}

/// AR 长按音轨（未拖动）：音轨信息菜单。
Future<void> showTrackMenu(
  BuildContext context, {
  required VoidCallback onEdit,
  VoidCallback? onDelete,
  VoidCallback? onEvents,
}) async {
  final l = AppLocalizations.of(context);
  final action = await showActionSheet(
    context,
    cancelLabel: l.cancel,
    actions: [
      SheetAction(value: 'edit', label: l.editTrackInfo, icon: Icons.edit),
      if (onEvents != null)
        SheetAction(
          value: 'events',
          label: l.eventList,
          icon: Icons.format_list_bulleted,
        ),
      if (onDelete != null)
        SheetAction(
          value: 'delete',
          label: l.deleteTrack,
          icon: Icons.delete_outline,
          danger: true,
        ),
    ],
  );
  switch (action) {
    case 'edit':
      onEdit();
    case 'events':
      onEvents?.call();
    case 'delete':
      onDelete?.call();
  }
}
