import '../core/snap_fields.dart';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../core/theme.dart';

/// 卷帘背景的 C 音名（C0/C1…），由设置「显示键盘」控制。
///
/// 直接画在 wgpu 纹理上方，用低对比度文字，不干扰音符；随纵向滚动/缩放
/// 跟随 key 行。只在水平钢琴卷帘下使用。
class KeyLabelPainter extends CustomPainter {
  KeyLabelPainter(this.snap, this.visible)
    : super(repaint: Listenable.merge([snap, visible]));

  /// `view_snapshot`：`[4] = key_height`、`[5] = keyboard_width`、
  /// `[9] = key_to_y(0)`。
  final ValueNotifier<List<double>> snap;
  final ValueListenable<bool> visible;

  @override
  void paint(Canvas canvas, Size size) {
    if (!visible.value) {
      return;
    }
    final s = snap.value;
    if (s.length < 10) {
      return;
    }
    final kh = s[SnapField.keyHeight];
    final keyboardW = s[SnapField.leftPanelW];
    final key0Top = s[SnapField.key0Y];
    // 与键盘列同一门槛/字号；行高太小时文字会糊成一片。
    if (kh < 10 || keyboardW <= 0 || size.height <= 0) {
      return;
    }
    final fontSize = (kh * 0.5).clamp(7.0, 11.0);
    final style = TextStyle(
      color: YinheTheme.textMuted.withValues(alpha: 0.35),
      fontSize: fontSize,
      fontWeight: FontWeight.w500,
    );
    for (var key = 0; key < 128; key += 12) {
      final top = key0Top - key * kh;
      if (top + kh < 0 || top > size.height) {
        continue;
      }
      final painter = TextPainter(
        // 与 `LabelsPainter._paintKeyboardLabels` 一致：key 60 = C4。
        text: TextSpan(text: 'C${key ~/ 12 - 1}', style: style),
        textDirection: TextDirection.ltr,
      )..layout();
      painter.paint(
        canvas,
        Offset(keyboardW + 8, top + (kh - painter.height) / 2),
      );
    }
  }

  @override
  bool shouldRepaint(covariant KeyLabelPainter oldDelegate) => false;
}
