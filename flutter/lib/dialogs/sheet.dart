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
    this.quarterTurns = 0,
  });

  final String value;
  final String label;
  final IconData icon;
  final bool enabled;
  final bool danger;

  /// 图标顺时针旋转 90° 的倍数（与桌面浮动工具条一致）。
  final int quarterTurns;
}

/// 统一的底部操作表（安全区 / 主题）。
///
/// 返回被点项的 `value`；点蒙版、按返回键或下滑关闭返回 null。
Future<String?> showActionSheet(
  BuildContext context, {
  String? title,
  String? subtitle,
  IconData? headerIcon,
  required List<SheetAction> actions,
}) {
  final hasHeader = title != null && title.isNotEmpty;
  return showModalBottomSheet<String>(
    context: context,
    backgroundColor: YinheTheme.bg,
    isScrollControlled: true,
    builder: (ctx) => SafeArea(
      child: ConstrainedBox(
        constraints: BoxConstraints(
          maxHeight: MediaQuery.sizeOf(ctx).height * 0.85,
        ),
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              if (hasHeader)
                ListTile(
                  leading: headerIcon == null ? null : Icon(headerIcon),
                  title: Text(
                    title,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
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
                  leading: Icon(
                    a.icon,
                    color: a.danger ? YinheTheme.danger : null,
                  ),
                  title: Text(
                    a.label,
                    style: a.danger
                        ? TextStyle(color: YinheTheme.danger)
                        : null,
                  ),
                  onTap: a.enabled
                      ? () => Navigator.of(ctx).pop(a.value)
                      : null,
                ),
            ],
          ),
        ),
      ),
    ),
  );
}
