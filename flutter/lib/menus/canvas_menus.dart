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

/// 通用浮动操作菜单：贴着长按位置弹出，样式与桌面右键一致。
///
/// 适合 2–5 项的小菜单；操作很多（选框网格、插入类型）仍用底部面板。
Future<String?> showFloatingActionMenu(
  BuildContext context, {
  required Offset globalPosition,
  required List<SheetAction> actions,
  String? title,
  String? subtitle,
  IconData? headerIcon,
}) async {
  final overlay = Overlay.of(context).context.findRenderObject() as RenderBox?;
  final origin = overlay?.globalToLocal(globalPosition) ?? globalPosition;
  final size = overlay?.size ?? MediaQuery.sizeOf(context);
  final hasHeader = title != null && title.isNotEmpty;
  return showMenu<String>(
    context: context,
    position: RelativeRect.fromLTRB(
      origin.dx,
      origin.dy,
      (size.width - origin.dx).clamp(0.0, double.infinity),
      (size.height - origin.dy).clamp(0.0, double.infinity),
    ),
    items: [
      if (hasHeader)
        PopupMenuItem<String>(
          enabled: false,
          height: subtitle == null || subtitle.isEmpty ? 40 : 54,
          child: Row(
            children: [
              if (headerIcon != null) ...[
                Icon(headerIcon, size: 18, color: YinheTheme.textSecondary),
                const SizedBox(width: 10),
              ],
              Expanded(
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      title,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        color: YinheTheme.text,
                        fontSize: 13,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                    if (subtitle != null && subtitle.isNotEmpty)
                      Text(
                        subtitle,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          color: YinheTheme.textSecondary,
                          fontSize: 11,
                        ),
                      ),
                  ],
                ),
              ),
            ],
          ),
        ),
      if (hasHeader) const PopupMenuDivider(),
      for (final a in actions)
        PopupMenuItem<String>(
          value: a.value,
          enabled: a.enabled,
          height: 40,
          child: _menuRow(
            a.icon,
            a.label,
            enabled: a.enabled,
            danger: a.danger,
            quarterTurns: a.quarterTurns,
          ),
        ),
    ],
  );
}

/// 长按空白处：在手指位置弹小型浮动菜单（粘贴/插入/全选/标签）。
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
  final action = await showFloatingActionMenu(
    context,
    globalPosition: globalPosition,
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
      if (onAddMarker != null)
        SheetAction(
          value: 'marker',
          label: l.markerLabel,
          icon: Icons.bookmark_add_outlined,
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

Widget _menuRow(
  IconData icon,
  String label, {
  bool enabled = true,
  bool danger = false,
  int quarterTurns = 0,
}) {
  final color = !enabled
      ? YinheTheme.textMuted.withValues(alpha: 0.5)
      : danger
      ? YinheTheme.danger
      : YinheTheme.text;
  Widget iconWidget = Icon(
    icon,
    size: 18,
    color: danger && enabled ? YinheTheme.danger : YinheTheme.textSecondary,
  );
  if (quarterTurns != 0) {
    iconWidget = RotatedBox(quarterTurns: quarterTurns, child: iconWidget);
  }
  return Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      iconWidget,
      const SizedBox(width: 10),
      Text(label, style: TextStyle(color: color, fontSize: 14)),
    ],
  );
}

/// AR 长按音轨（未拖动）：音轨信息菜单（贴手指弹出）。
Future<void> showTrackMenu(
  BuildContext context, {
  required Offset globalPosition,
  required VoidCallback onEdit,
  required VoidCallback onSelectAll,
  VoidCallback? onDelete,
  VoidCallback? onEvents,
}) async {
  final l = AppLocalizations.of(context);
  final action = await showFloatingActionMenu(
    context,
    globalPosition: globalPosition,
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
