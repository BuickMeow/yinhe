import 'package:flutter/material.dart';

import 'theme.dart';

/// 时间标尺（与桌面 `time_ruler` 一致）：只有小节/拍/子拍/tick 文本标签，
/// 无刻度线；背景铺 `track_bg`（含左侧面板/键盘上方留白）。
class RulerPainter extends CustomPainter {
  RulerPainter(this.snap) : super(repaint: snap);

  final ValueNotifier<List<double>> snap;

  /// 标尺高度（逻辑像素，与桌面 `RULER_H` 一致）。
  static const double height = 30.0;
  static const double minLabelSpacing = 38.0;
  static const int subBeatDiv = 4;

  @override
  void paint(Canvas canvas, Size size) {
    // 背景：整条（含面板/键盘上方留白）。
    canvas.drawRect(
      Rect.fromLTWH(0, 0, size.width, size.height),
      Paint()..color = YinheTheme.trackBg,
    );
    final s = snap.value;
    if (s.length < 12 || size.width <= 0) {
      return;
    }
    final ppu = s[1];
    final scrollX = s[2];
    final kbW = s[5];
    final ppq = s[8] > 0 ? s[8] : 480.0;
    if (ppu <= 0) {
      return;
    }
    // 拍号（快照 10/11：分子 + 分母 2 次幂；原生事件优先，否则工程默认）。
    final sigNum = s[10] > 0 ? s[10].round() : 4;
    final sigDenPow = s[11] >= 0 ? s[11].round() : 2;
    final tpb = ppq.round().clamp(1, 1 << 24);
    final den = 1 << sigDenPow.clamp(0, 8);
    final measure = (tpb * sigNum * 4 ~/ den).clamp(1, 1 << 30);
    final beatTicks = (measure ~/ sigNum).clamp(1, measure);
    final measurePx = measure * ppu;
    var divisor = 1;
    while (measurePx * divisor < minLabelSpacing && divisor < 64) {
      divisor *= 2;
    }
    final barStep = measure * divisor;

    final showBeat = beatTicks * ppu >= minLabelSpacing;
    final subTicks = (beatTicks ~/ subBeatDiv).clamp(1, measure);
    final showSub = subTicks * ppu >= minLabelSpacing;
    final showTick = ppu >= minLabelSpacing;

    final startTick = ((-kbW + scrollX) / ppu).clamp(0.0, double.infinity);
    final endTick = ((size.width - kbW + scrollX) / ppu).clamp(
      0.0,
      double.infinity,
    );
    double xOf(int tick) => tick * ppu - scrollX + kbW;
    // 桌面：文本带中心在标尺高度 0.72 处、左对齐 +2px。
    final yCenter = size.height * 0.72;

    final step = showTick
        ? 1
        : showSub
        ? subTicks
        : showBeat
        ? beatTicks
        : barStep;
    var tick = (startTick.toInt() ~/ step) * step;
    final endI = endTick.toInt();
    while (tick <= endI + step) {
      final x = xOf(tick);
      if (x >= kbW && x <= size.width + 40) {
        final isMeasure = tick % barStep == 0;
        final isBeat = !isMeasure && tick % beatTicks == 0;
        String? label;
        Color? color;
        if (isMeasure) {
          label = '${tick ~/ measure + 1}';
          color = YinheTheme.measureLabel;
        } else if (isBeat && showBeat) {
          final bar = tick ~/ measure + 1;
          final beat = (tick % measure) ~/ beatTicks + 1;
          label = '$bar.$beat';
          color = YinheTheme.textLabel;
        } else if (showSub) {
          final bar = tick ~/ measure + 1;
          final beat = (tick % measure) ~/ beatTicks + 1;
          if (showTick) {
            label =
                '$bar.$beat.${(tick % beatTicks).toString().padLeft(3, '0')}';
            color = YinheTheme.tickLabel;
          } else {
            final sub = (tick % beatTicks) ~/ subTicks;
            label = '$bar.$beat.$sub';
            color = YinheTheme.textDisabled;
          }
        }
        if (label != null) {
          _label(canvas, label, Offset(x + 2, yCenter), color!);
        }
      }
      tick += step;
    }
  }

  void _label(Canvas canvas, String text, Offset pos, Color color) {
    final tp = TextPainter(
      text: TextSpan(
        text: text,
        style: TextStyle(fontSize: 10, color: color),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    tp.paint(canvas, Offset(pos.dx, pos.dy - tp.height / 2));
  }

  @override
  bool shouldRepaint(covariant RulerPainter oldDelegate) => true;
}
