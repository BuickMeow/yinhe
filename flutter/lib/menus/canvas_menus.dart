import 'package:flutter/material.dart';

import '../core/theme.dart';
import '../dialogs/sheet.dart';
import '../l10n/app_localizations.dart';

/// 长按选区操作菜单：网格常驻，点外侧/返回才关闭。
///
/// 点某个操作只执行回调，不自动关闭，方便连续多次操作。
Future<void> showSelectionActionMenu(
  BuildContext context, {
  required bool automation,
  required Future<void> Function(int id) onAction,
}) {
  return showModalBottomSheet<void>(
    context: context,
    backgroundColor: YinheTheme.bg,
    barrierColor: Colors.black.withValues(alpha: 0.62),
    isScrollControlled: true,
    useSafeArea: false,
    builder: (ctx) {
      final l = AppLocalizations.of(ctx);
      return SafeArea(
        top: false,
        child: Padding(
          padding: const EdgeInsets.fromLTRB(4, 4, 4, 4),
          child: GridView.count(
            crossAxisCount: 8,
            shrinkWrap: true,
            physics: const NeverScrollableScrollPhysics(),
            childAspectRatio: 1.05,
            mainAxisSpacing: 2,
            crossAxisSpacing: 2,
            children: [
              for (final a in selectionActionItems(l, automation: automation))
                _SelectionGridCell(
                  action: a,
                  onTap: a.enabled
                      ? () => _pickSelectionAction(ctx, a.value, onAction)
                      : null,
                ),
            ],
          ),
        ),
      );
    },
  );
}

/// 危险操作先二次确认；普通操作直接交给常驻菜单的回调。
Future<void> _pickSelectionAction(
  BuildContext ctx,
  String value,
  Future<void> Function(int id) onAction,
) async {
  final id = int.tryParse(value);
  if (id == null) {
    return;
  }
  if (id == 11) {
    if (!ctx.mounted) {
      return;
    }
    final l = AppLocalizations.of(ctx);
    final ok = await showDialog<bool>(
      context: ctx,
      builder: (dialogCtx) => AlertDialog(
        title: Text(l.deleteSelectionConfirm),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(dialogCtx).pop(false),
            child: Text(l.cancel),
          ),
          FilledButton(
            onPressed: () => Navigator.of(dialogCtx).pop(true),
            child: Text(l.delete),
          ),
        ],
      ),
    );
    if (ok != true) {
      return;
    }
  }
  await onAction(id);
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
    SheetAction(value: '24', label: l.batchProcess, icon: Icons.tune),
  ];
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

/// 主视口空白处长按完成：粘贴 / 全选 菜单。
///
/// 插入位置与插入菜单已移到时间标尺（见 `MarkerLayer` 的长按回调）。
Future<void> showEmptyLongPressMenu(
  BuildContext context, {
  required bool canPaste,
  required VoidCallback onPaste,
  required VoidCallback onSelectAll,
}) async {
  final l = AppLocalizations.of(context);
  final action = await showActionSheet(
    context,
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
    ],
  );
  switch (action) {
    case 'select_all':
      onSelectAll();
    case 'paste':
      onPaste();
  }
}

/// 长按空白处：在手指位置弹小型浮动菜单（替代底部 sheet，减少手指位移）。
Future<void> showFloatingEmptyLongPressMenu(
  BuildContext context, {
  required Offset globalPosition,
  required bool canPaste,
  required VoidCallback onPaste,
  required VoidCallback? onInsert,
  required VoidCallback onSelectAll,
  VoidCallback? onAddMarker,
}) async {
  final l = AppLocalizations.of(context);
  final overlay = Overlay.of(context).context.findRenderObject() as RenderBox?;
  final origin = overlay?.globalToLocal(globalPosition) ?? globalPosition;
  final size = overlay?.size ?? MediaQuery.sizeOf(context);
  final action = await showMenu<String>(
    context: context,
    position: RelativeRect.fromLTRB(
      origin.dx,
      origin.dy,
      (size.width - origin.dx).clamp(0.0, double.infinity),
      (size.height - origin.dy).clamp(0.0, double.infinity),
    ),
    items: [
      PopupMenuItem(
        value: 'select_all',
        height: 40,
        child: _menuRow(Icons.select_all, l.selectAll),
      ),
      PopupMenuItem(
        value: 'paste',
        height: 40,
        enabled: canPaste,
        child: _menuRow(Icons.content_paste, l.paste),
      ),
      if (onInsert != null)
        PopupMenuItem(
          value: 'insert',
          height: 40,
          child: _menuRow(Icons.add_circle_outline, l.insert),
        ),
      if (onAddMarker != null)
        PopupMenuItem(
          value: 'marker',
          height: 40,
          child: _menuRow(Icons.bookmark_add_outlined, l.markerLabel),
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
    case 'marker':
      onAddMarker?.call();
  }
}

Widget _menuRow(IconData icon, String label) {
  return Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      Icon(icon, size: 18, color: YinheTheme.textSecondary),
      const SizedBox(width: 10),
      Text(label, style: TextStyle(color: YinheTheme.text, fontSize: 14)),
    ],
  );
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
