import 'package:flutter/material.dart';

import '../core/theme.dart';
import '../dialogs/sheet.dart';
import '../l10n/app_localizations.dart';

/// 选区操作选择器：弹网格 → 危险操作二次确认 → 返回动作编号。
Future<int?> showSelectionActionPicker(
  BuildContext context, {
  required bool automation,
}) async {
  final l = AppLocalizations.of(context);
  final value = await showSelectionActionGrid(
    context,
    title: l.selectionActions,
    cancelLabel: l.cancel,
    actions: selectionActionItems(l, automation: automation),
  );
  if (value == null) {
    return null;
  }
  final id = int.tryParse(value);
  if (id == null) {
    return null;
  }
  if (id == 11) {
    if (!context.mounted) {
      return null;
    }
    final ok = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text(l.deleteSelectionConfirm),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(ctx).pop(false),
            child: Text(l.cancel),
          ),
          FilledButton(
            onPressed: () => Navigator.of(ctx).pop(true),
            child: Text(l.delete),
          ),
        ],
      ),
    );
    if (ok != true) {
      return null;
    }
  }
  return id;
}

/// 14 个选区操作，顺序 = 两行 7 列网格。
///
/// `automation`：AM 锚点只启用删除/重复/变速/水平翻转，其余置灰。
List<SheetAction> selectionActionItems(
  AppLocalizations l, {
  bool automation = false,
}) {
  bool noteOnly() => !automation;
  return [
    SheetAction(
      value: '1',
      label: l.copy,
      icon: Icons.content_copy,
      enabled: noteOnly(),
    ),
    SheetAction(
      value: '2',
      label: l.cut,
      icon: Icons.content_cut,
      enabled: noteOnly(),
    ),
    SheetAction(value: '11', label: l.delete, icon: Icons.delete, danger: true),
    SheetAction(value: '10', label: l.opDuplicate, icon: Icons.content_copy),
    SheetAction(
      value: '12',
      label: l.opOctaveUp,
      icon: Icons.download,
      quarterTurns: 2,
      enabled: noteOnly(),
    ),
    SheetAction(
      value: '13',
      label: l.opOctaveDown,
      icon: Icons.download,
      enabled: noteOnly(),
    ),
    SheetAction(
      value: '20',
      label: l.opLengthHalf,
      icon: Icons.download,
      quarterTurns: 1,
    ),
    SheetAction(
      value: '19',
      label: l.opLengthDouble,
      icon: Icons.upload,
      quarterTurns: 1,
    ),
    SheetAction(value: '21', label: l.opFlipHorizontal, icon: Icons.flip),
    SheetAction(
      value: '22',
      label: l.opFlipVertical,
      icon: Icons.flip,
      quarterTurns: 1,
      enabled: noteOnly(),
    ),
    SheetAction(
      value: '16',
      label: l.opHumanize,
      icon: Icons.person,
      enabled: noteOnly(),
    ),
    SheetAction(
      value: '17',
      label: l.opFillGaps,
      icon: Icons.compare_arrows,
      enabled: noteOnly(),
    ),
    SheetAction(
      value: '18',
      label: l.opMergeNotes,
      icon: Icons.line_style,
      enabled: noteOnly(),
    ),
    SheetAction(
      value: '23',
      label: l.opGridSplit,
      icon: Icons.grid_on,
      enabled: noteOnly(),
    ),
  ];
}

/// 长按选框/AM 选区：两行图标网格操作面板。
///
/// 每格「图标 + 文字」，最多两行；点一格返回其 `value`，点蒙版/关闭返回 null。
Future<String?> showSelectionActionGrid(
  BuildContext context, {
  required String title,
  required String cancelLabel,
  required List<SheetAction> actions,
  int columns = 7,
}) {
  return showModalBottomSheet<String>(
    context: context,
    backgroundColor: YinheTheme.bg,
    barrierColor: Colors.black.withValues(alpha: 0.62),
    isScrollControlled: true,
    useSafeArea: false,
    builder: (ctx) => SafeArea(
      top: false,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(8, 4, 8, 8),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Row(
              children: [
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    title,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      color: YinheTheme.textLabel,
                      fontSize: 13,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                ),
                SizedBox(
                  width: 32,
                  height: 32,
                  child: IconButton(
                    padding: EdgeInsets.zero,
                    iconSize: 18,
                    tooltip: cancelLabel,
                    icon: const Icon(Icons.close),
                    onPressed: () => Navigator.of(ctx).pop(),
                  ),
                ),
              ],
            ),
            GridView.count(
              crossAxisCount: columns,
              shrinkWrap: true,
              physics: const NeverScrollableScrollPhysics(),
              childAspectRatio: 0.92,
              mainAxisSpacing: 2,
              crossAxisSpacing: 2,
              children: [
                for (final a in actions)
                  _SelectionGridCell(
                    action: a,
                    onTap: a.enabled
                        ? () => Navigator.of(ctx).pop(a.value)
                        : null,
                  ),
              ],
            ),
          ],
        ),
      ),
    ),
  );
}

/// 网格里的一格：图标在上、文字在下（最多两行）。
class _SelectionGridCell extends StatelessWidget {
  const _SelectionGridCell({required this.action, this.onTap});

  final SheetAction action;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final enabled = onTap != null;
    final color = !enabled
        ? YinheTheme.textMuted.withValues(alpha: 0.4)
        : action.danger
        ? YinheTheme.danger
        : YinheTheme.textLabel;
    Widget icon = Icon(action.icon, size: 22, color: color);
    if (action.quarterTurns != 0) {
      icon = RotatedBox(quarterTurns: action.quarterTurns, child: icon);
    }
    return InkWell(
      borderRadius: BorderRadius.circular(8),
      onTap: onTap,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 2, vertical: 4),
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            icon,
            const SizedBox(height: 4),
            Text(
              action.label,
              maxLines: 2,
              textAlign: TextAlign.center,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(color: color, fontSize: 10, height: 1.15),
            ),
          ],
        ),
      ),
    );
  }
}

/// 空白处长按完成：粘贴 / 插入 / 全选 菜单。
Future<void> showEmptyLongPressMenu(
  BuildContext context, {
  required bool canPaste,
  required VoidCallback onPaste,
  required VoidCallback? onInsert,
  required VoidCallback onSelectAll,
}) async {
  final l = AppLocalizations.of(context);
  final action = await showActionSheet(
    context,
    cancelLabel: l.cancel,
    actions: [
      SheetAction(
        value: 'select_all',
        label: l.selectAll,
        icon: Icons.select_all,
      ),
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
  switch (action) {
    case 'select_all':
      onSelectAll();
    case 'paste':
      onPaste();
    case 'insert':
      onInsert?.call();
  }
}

/// AR 长按音轨（未拖动）：音轨信息菜单。
Future<void> showTrackMenu(
  BuildContext context, {
  required VoidCallback onEdit,
  required VoidCallback onSelectAll,
  VoidCallback? onDelete,
  VoidCallback? onEvents,
}) async {
  final l = AppLocalizations.of(context);
  final action = await showActionSheet(
    context,
    cancelLabel: l.cancel,
    actions: [
      SheetAction(
        value: 'select_all',
        label: l.selectAllTrack,
        icon: Icons.select_all,
      ),
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
    case 'select_all':
      onSelectAll();
    case 'edit':
      onEdit();
    case 'events':
      onEvents?.call();
    case 'delete':
      onDelete?.call();
  }
}
