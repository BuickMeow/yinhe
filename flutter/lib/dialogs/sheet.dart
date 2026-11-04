import 'package:flutter/material.dart';

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
