import 'package:flutter/material.dart';

import '../core/models.dart';
import '../core/theme.dart';

/// PR 自动化栏绘制：力度条或离散（Step）自动化曲线，支持拖动锚点预览。
class AutomationPainter extends CustomPainter {
  AutomationPainter({
    required this.lane,
    required this.velocity,
    required this.trackColors,
    required this.color,
    required this.snap,
    required this.preview,
  }) : super(repaint: Listenable.merge([snap, preview]));

  /// null = 力度模式。
  final AutomationLaneData? lane;

  /// 力度条（每音符/每摘要一条）：(tick, length, velocity01, track)。
  final List<(double, double, double, int)> velocity;

  /// 轨道颜色（力度条按音符所属轨道着色）。
  final List<Color> trackColors;
  final Color color;
  final ValueNotifier<List<double>> snap;

  /// 拖动中的锚点预览（tick, value）。
  final ValueNotifier<(double, double)?> preview;

  /// 左侧 combo 列宽（与 main.dart 的 SizedBox 保持一致）。
  static const double comboWidth = 60.0;

  @override
  void paint(Canvas canvas, Size size) {
    if (size.width <= 0 || size.height <= 0) {
      return;
    }
    final s = snap.value;
    if (s.length < 12) {
      return;
    }
    final ppu = s[1];
    final scrollX = s[2];
    final kbW = s[5];
    if (ppu <= 0) {
      return;
    }
    double xOf(double tick) => tick * ppu - scrollX + (kbW - comboWidth);
    double yOf(double value) => size.height * (1.0 - value.clamp(0.0, 1.0));

    final l = lane;
    if (l == null) {
      // 力度条（与桌面 wgpu 一致）：每音符一条空心描边柱，
      // 宽度 = gate × ppu（最小 2px），高度 ∝ (velocity-1)/126；按轨道着色。
      final paint = Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.0;
      // 数据按 tick 升序：二分定位第一个进入视口的条，避免每帧全量扫描。
      var lo = 0;
      var hi = velocity.length;
      while (lo < hi) {
        final mid = (lo + hi) >> 1;
        final p = velocity[mid];
        if (xOf(p.$1 + p.$2) < -4) {
          lo = mid + 1;
        } else {
          hi = mid;
        }
      }
      for (var i = lo; i < velocity.length; i++) {
        final (tick, len, vel01, track) = velocity[i];
        final rawLeft = xOf(tick);
        final rawRight = xOf(tick + len);
        if (rawRight < -4) {
          continue;
        }
        if (rawLeft > size.width + 4) {
          break;
        }
        final left = rawLeft.clamp(0.0, size.width);
        var width = rawRight - left;
        if (width < 2.0) {
          width = 2.0;
        }
        final top = yOf(vel01);
        var height = size.height - top;
        if (height < 1.0) {
          height = 1.0;
        }
        paint.color = (track >= 0 && track < trackColors.length)
            ? trackColors[track]
            : color;
        canvas.drawRect(Rect.fromLTWH(left, top, width, height), paint);
      }
      return;
    }

    final points = l.points;
    // PB 中心线（归一化 0.5）。
    if (l.kind == 0) {
      final y = size.height * 0.5;
      canvas.drawLine(
        Offset(0, y),
        Offset(size.width, y),
        Paint()..color = YinheTheme.lineFg.withValues(alpha: 0.6),
      );
    }
    if (points.isEmpty) {
      return;
    }

    // 离散（Step）显示：横向保持到下一 tick，再竖直跳变。
    final line = Paint()
      ..color = color.withValues(alpha: 0.85)
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.5;
    final path = Path();
    var started = false;
    var lastY = 0.0;
    for (final p in points) {
      final x = xOf(p.dx);
      final y = yOf(p.dy);
      if (x < -40) {
        lastY = y;
        continue;
      }
      if (x > size.width + 40) {
        break;
      }
      if (!started) {
        path.moveTo(x, y);
        started = true;
      } else {
        path.lineTo(x, lastY);
        path.lineTo(x, y);
      }
      lastY = y;
    }
    if (started) {
      path.lineTo(size.width, lastY);
      canvas.drawPath(path, line);
    }

    // 锚点
    final anchor = Paint()..color = color;
    for (final p in points) {
      final x = xOf(p.dx);
      if (x < 0 || x > size.width) {
        continue;
      }
      canvas.drawRect(
        Rect.fromCenter(center: Offset(x, yOf(p.dy)), width: 5, height: 5),
        anchor,
      );
    }

    // 拖动预览
    final pv = preview.value;
    if (pv != null) {
      final x = xOf(pv.$1);
      final y = yOf(pv.$2);
      canvas.drawLine(
        Offset(x, 0),
        Offset(x, size.height),
        Paint()
          ..color = YinheTheme.accent.withValues(alpha: 0.5)
          ..strokeWidth = 1,
      );
      canvas.drawCircle(
        Offset(x, y),
        6,
        Paint()..color = YinheTheme.accent.withValues(alpha: 0.35),
      );
      canvas.drawRect(
        Rect.fromCenter(center: Offset(x, y), width: 7, height: 7),
        Paint()..color = Colors.white,
      );
    }
  }

  @override
  bool shouldRepaint(covariant AutomationPainter oldDelegate) => true;
}
