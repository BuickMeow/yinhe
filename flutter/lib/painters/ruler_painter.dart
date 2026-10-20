import 'package:flutter/material.dart';

import '../core/bar_beat.dart';
import '../core/theme.dart';

/// 时间标尺（与桌面 `time_ruler` 一致）：只有小节/拍/子拍/tick 文本标签，
/// 无刻度线；背景铺 `track_bg`（含左侧面板/键盘上方留白）。
///
/// 拍号按分段排版：拍号变化时各段用各自的 measure/beat 长，小节号跨段连续。
class RulerPainter extends CustomPainter {
  RulerPainter(this.snap, this.grid)
    : super(repaint: Listenable.merge([snap, grid]));

  final ValueNotifier<List<double>> snap;
  final ValueNotifier<BarBeatGrid?> grid;

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
    final g = grid.value;
    if (s.length < 12 || size.width <= 0 || g == null) {
      return;
    }
    final ppu = s[1];
    final scrollX = s[2];
    final kbW = s[5];
    if (ppu <= 0) {
      return;
    }
    final startTick = ((-kbW + scrollX) / ppu).clamp(0.0, double.infinity);
    final endTick = ((size.width - kbW + scrollX) / ppu).clamp(
      0.0,
      double.infinity,
    );
    double xOf(int tick) => tick * ppu - scrollX + kbW;
    // 桌面：文本带中心在标尺高度 0.72 处、左对齐 +2px。
    final yCenter = size.height * 0.72;
    final endI = endTick.floor();

    for (var i = 0; i < g.segments.length; i++) {
      final (segStart, num, denPow) = g.segments[i];
      final segEnd = i + 1 < g.segments.length ? g.segments[i + 1].$1 : 1 << 31;
      if (segStart > endTick) {
        break;
      }
      final (mticks, bticks) = g.ticksOfSegment(i);
      final subTicks = (bticks ~/ subBeatDiv).clamp(1, mticks);
      final divisor = BarBeatGrid.measureDivisor(mticks * ppu, minLabelSpacing);
      final mergedTicks = mticks * divisor;
      final showBeat = bticks * ppu >= minLabelSpacing;
      final showSub = subTicks * ppu >= minLabelSpacing;
      final showTick = ppu >= minLabelSpacing;
      // 主循环步长：显示 beat/sub 时用 subTicks，否则用合并小节步长。
      final step = showBeat || showSub ? subTicks : mergedTicks;
      // 对齐到段内网格：变拍子段的 segStart 通常不在全局步长网格上。
      final firstTick = segStart > startTick ? segStart : startTick.floor();
      var tick = segStart + ((firstTick - segStart) ~/ step) * step;
      while (tick <= endI && tick < segEnd) {
        final x = xOf(tick);
        if (x >= kbW && x <= size.width + 40) {
          final local = tick - segStart;
          final isMeasure = local % mergedTicks == 0;
          final isBeat = !isMeasure && (local % mticks) % bticks == 0;
          String? label;
          Color? color;
          if (isMeasure) {
            label = '${g.barOffsets[i] + local ~/ mticks + 1}';
            color = YinheTheme.measureLabel;
          } else if (isBeat && showBeat) {
            final bar = g.barOffsets[i] + local ~/ mticks + 1;
            final beat = (local % mticks) ~/ bticks + 1;
            label = '$bar.$beat';
            color = YinheTheme.textLabel;
          } else if (showSub) {
            final bar = g.barOffsets[i] + local ~/ mticks + 1;
            final beat = (local % mticks) ~/ bticks + 1;
            if (showTick) {
              final inBeat = local % bticks;
              label = '$bar.$beat.${inBeat.toString().padLeft(3, '0')}';
              color = YinheTheme.tickLabel;
            } else {
              final sub = (local % bticks) ~/ subTicks;
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
