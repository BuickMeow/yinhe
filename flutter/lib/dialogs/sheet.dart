import 'package:flutter/material.dart';

import '../core/theme.dart';

/// 底部操作表的一项。
class SheetAction {
  const SheetAction({
    required this.value,
    required this.label,
    required this.icon,
    this.enabled = true,
    this.danger = false,
  });

  final String value;
  final String label;
  final IconData icon;
  final bool enabled;
  final bool danger;
}

/// 统一的底部操作表（安全区 / 主题 / 取消项）。
///
/// 返回被点项的 `value`；点取消或点外部返回 null。
Future<String?> showActionSheet(
  BuildContext context, {
  String? title,
  String? subtitle,
  IconData? headerIcon,
  required List<SheetAction> actions,
  required String cancelLabel,
}) {
  final hasHeader = title != null && title.isNotEmpty;
  return showModalBottomSheet<String>(
    context: context,
    backgroundColor: YinheTheme.bg,
    builder: (ctx) => SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (hasHeader)
            ListTile(
              leading: headerIcon == null ? null : Icon(headerIcon),
              title: Text(title, maxLines: 1, overflow: TextOverflow.ellipsis),
              subtitle: subtitle == null
                  ? null
                  : Text(
                      subtitle,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
            ),
          if (hasHeader) const Divider(height: 1),
          for (final a in actions)
            ListTile(
              enabled: a.enabled,
              leading: Icon(a.icon, color: a.danger ? YinheTheme.danger : null),
              title: Text(
                a.label,
                style: a.danger ? TextStyle(color: YinheTheme.danger) : null,
              ),
              onTap: a.enabled ? () => Navigator.of(ctx).pop(a.value) : null,
            ),
          ListTile(
            leading: const Icon(Icons.close),
            title: Text(cancelLabel),
            onTap: () => Navigator.of(ctx).pop(),
          ),
        ],
      ),
    ),
  );
}
