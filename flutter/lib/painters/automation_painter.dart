import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../core/bar_beat.dart';
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
    required this.grid,
    required this.selectedTicks,
    required this.movePreview,
    required this.marquee,
    required this.velocityPreview,
    required this.paintBaseData,
    this.paintGrid = true,
    this.paintOverlay = true,
  }) : super(repaint: Listenable.merge([snap, grid, velocityPreview]));

  /// null = 力度模式。
  final AutomationLaneData? lane;

  /// 力度条（每音符/每摘要一条）：(tick, length, velocity01, track)。
  final List<(double, double, double, int)> velocity;

  /// 轨道颜色（力度条按音符所属轨道着色）。
  final List<Color> trackColors;
  final Color color;
  final ValueNotifier<List<double>> snap;

  /// 拍号分段（与 PR 共用，保证小节/拍网格对齐）。
  final ValueNotifier<BarBeatGrid?> grid;

  /// 已选锚点（tick）。
  final Set<int> selectedTicks;

  /// old tick → (new tick, new value)：拖动中的移动预览。
  final Map<int, (int, double)> movePreview;

  /// 框选矩形（绘制区局部坐标，全高）。
  final Rect? marquee;

  /// 力度笔划预览：(tick, length, velocity01, track)。
  final ValueNotifier<List<(double, double, double, int)>> velocityPreview;

  /// 基础力度条/曲线是否仍由 Flutter 绘制。
  /// false = 已由 wgpu 画进主纹理，Flutter 只补网格、选框和拖动预览。
  final bool paintBaseData;

  /// 是否绘制网格。网格可以单独放在 wgpu 纹理下方。
  final bool paintGrid;

  /// 是否绘制选框/锚点高亮/拖动预览等交互层。
  final bool paintOverlay;

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

    if (paintGrid) {
      _paintGrid(canvas, size, ppu, scrollX, kbW);
    }
    if (!paintOverlay) {
      return;
    }

    double yOf(double value) => size.height * (1.0 - value.clamp(0.0, 1.0));

    final l = lane;
    if (l == null) {
      // 力度条（与桌面 wgpu 一致）：每音符一条空心描边柱，
      // 宽度 = gate × ppu（最小 2px），高度 ∝ (velocity-1)/126；按轨道着色。
      if (paintBaseData) {
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
      }
      // 笔划预览：用较粗描边显示松手后将写入的高度，原力度条保留作参照。
      final previewPaint = Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 2.0;
      for (final (tick, len, vel01, track) in velocityPreview.value) {
        final left = xOf(tick);
        final right = xOf(tick + len);
        if (right < -4 || left > size.width + 4) {
          continue;
        }
        final w = (right - left).clamp(2.0, double.infinity).toDouble();
        final top = yOf(vel01);
        previewPaint.color =
            ((track >= 0 && track < trackColors.length)
                    ? trackColors[track]
                    : color)
                .withValues(alpha: 0.85);
        canvas.drawRect(
          Rect.fromLTWH(left, top, w, size.height - top),
          previewPaint,
        );
      }
      return;
    }

    final points = l.points;
    if (points.isEmpty) {
      _paintMarquee(canvas);
      return;
    }

    if (!paintBaseData) {
      if (movePreview.isNotEmpty) {
        final moved = <Offset>[
          for (final p in points)
            if (movePreview[p.dx.round()] case (final int tick, final double v))
              Offset(tick.toDouble(), v)
            else
              p,
        ]..sort((a, b) => a.dx.compareTo(b.dx));
        final movedSelected = <int>{
          for (final e in movePreview.entries)
            if (selectedTicks.contains(e.key)) e.value.$1,
        };
        _paintLane(canvas, size, moved, color, 1.0, movedSelected, xOf, yOf);
      } else {
        _paintSelectedAnchors(canvas, size, points, xOf, yOf);
      }
    } else if (movePreview.isEmpty) {
      _paintLane(canvas, size, points, color, 1.0, selectedTicks, xOf, yOf);
    } else {
      // 原曲线淡化作参照，移动后的曲线/锚点用原色。
      _paintLane(canvas, size, points, color, 0.22, const {}, xOf, yOf);
      final moved = <Offset>[
        for (final p in points)
          if (movePreview[p.dx.round()] case (final int tick, final double v))
            Offset(tick.toDouble(), v)
          else
            p,
      ]..sort((a, b) => a.dx.compareTo(b.dx));
      final movedSelected = <int>{
        for (final e in movePreview.entries)
          if (selectedTicks.contains(e.key)) e.value.$1,
      };
      _paintLane(canvas, size, moved, color, 1.0, movedSelected, xOf, yOf);
    }
    _paintMarquee(canvas);
  }

  /// 离散（Step）曲线 + 锚点；`alpha < 1` 时作为拖动参照层。
  void _paintLane(
    Canvas canvas,
    Size size,
    List<Offset> points,
    Color color,
    double alpha,
    Set<int> selected,
    double Function(double) xOf,
    double Function(double) yOf,
  ) {
    final line = Paint()
      ..color = color.withValues(alpha: 0.85 * alpha)
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

    final anchor = Paint()..color = color.withValues(alpha: alpha);
    final selectedPaint = Paint()
      ..color = YinheTheme.accent.withValues(alpha: alpha);
    final selectedEdge = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1
      ..color = Colors.white.withValues(alpha: alpha);
    for (final p in points) {
      final x = xOf(p.dx);
      if (x < 0 || x > size.width) {
        continue;
      }
      final y = yOf(p.dy);
      if (selected.contains(p.dx.round())) {
        final r = Rect.fromCenter(center: Offset(x, y), width: 7, height: 7);
        canvas.drawRect(r, selectedPaint);
        canvas.drawRect(r, selectedEdge);
      } else {
        canvas.drawRect(
          Rect.fromCenter(center: Offset(x, y), width: 5, height: 5),
          anchor,
        );
      }
    }
  }

  /// wgpu 已画基础曲线/锚点，Flutter 只补选中锚点的高亮。
  void _paintSelectedAnchors(
    Canvas canvas,
    Size size,
    List<Offset> points,
    double Function(double) xOf,
    double Function(double) yOf,
  ) {
    if (selectedTicks.isEmpty) {
      return;
    }
    final fill = Paint()..color = YinheTheme.accent;
    final edge = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1
      ..color = Colors.white;
    for (final p in points) {
      if (!selectedTicks.contains(p.dx.round())) {
        continue;
      }
      final x = xOf(p.dx);
      if (x < 0 || x > size.width) {
        continue;
      }
      final r = Rect.fromCenter(
        center: Offset(x, yOf(p.dy)),
        width: 7,
        height: 7,
      );
      canvas.drawRect(r, fill);
      canvas.drawRect(r, edge);
    }
  }

  /// 框选：全高竖向色带。
  void _paintMarquee(Canvas canvas) {
    final m = marquee;
    if (m == null) {
      return;
    }
    canvas.drawRect(
      m,
      Paint()..color = YinheTheme.accent.withValues(alpha: 0.08),
    );
    canvas.drawRect(
      m,
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1
        ..color = YinheTheme.accent.withValues(alpha: 0.5),
    );
  }

  /// PR 网格最小间距（与 Rust `GRID_MIN_SPACING` 一致）。
  static const double _gridMinSpacing = 38.0;

  /// 网格：横向 0/25/50/75/100% 值参考线 + 与 PR 同规则的小节/拍/细分线。
  void _paintGrid(
    Canvas canvas,
    Size size,
    double ppu,
    double scrollX,
    double kbW,
  ) {
    final line = YinheTheme.lineFg;
    final hPaint = Paint()..strokeWidth = 1.0;
    // 50% 是双极参数（PB/Fine Tune/Pan 等）的中心，略强。
    for (final f in const [0.0, 0.25, 0.5, 0.75, 1.0]) {
      final y = size.height * (1.0 - f);
      hPaint.color = line.withValues(alpha: f == 0.5 ? 0.55 : 0.34);
      canvas.drawLine(Offset(0, y), Offset(size.width, y), hPaint);
    }

    final g = grid.value;
    if (g == null || size.width <= 0 || size.height <= 0) {
      return;
    }
    final tpb = g.ppq;
    if (tpb <= 0) {
      return;
    }
    double xOf(double tick) => tick * ppu - scrollX + (kbW - comboWidth);
    final subTicks = math.max(tpb ~/ 4, 1);
    final fineStep = _fineGridStep(ppu, tpb);
    // 可见 tick：AM 绘制区从 combo 列右侧开始，与 PR 共用同一 ppu/scroll。
    final startTick = ((comboWidth - kbW + scrollX) / ppu).clamp(
      0.0,
      double.infinity,
    );
    final endTick = ((size.width + comboWidth - kbW + scrollX) / ppu).clamp(
      0.0,
      double.infinity,
    );
    final endI = endTick.floor();
    final vPaint = Paint()..strokeWidth = 1.0;
    Color level(int alpha) => line.withValues(alpha: alpha / 255.0);

    for (var i = 0; i < g.segments.length; i++) {
      final (segStart, _, _) = g.segments[i];
      final segEnd = i + 1 < g.segments.length ? g.segments[i + 1].$1 : 1 << 31;
      if (segStart > endTick) {
        break;
      }
      final (measureTicks, beatTicks) = g.ticksOfSegment(i);
      // 与 wgpu `push_time_grid` 相同的层级选择：合并小节/小节/拍/细分/tick。
      final divisor = BarBeatGrid.measureDivisor(
        measureTicks * ppu,
        _gridMinSpacing,
      );
      final merged = divisor > 1;
      final mergedMeasureTicks = measureTicks * divisor;
      final showBeat = !merged;
      final showSub = showBeat && beatTicks * ppu >= _gridMinSpacing;
      final showTick = showSub && ppu >= 10.0;
      final gridMeasureTicks = merged
          ? math.max(mergedMeasureTicks ~/ 2, 1)
          : measureTicks;
      final step = showTick
          ? 1
          : showSub
          ? math.max(math.min(fineStep, subTicks), 1)
          : showBeat
          ? math.max(beatTicks, 1)
          : math.max(gridMeasureTicks, 1);
      final firstTick = math.max(segStart, startTick.floor());
      var tick = segStart + ((firstTick - segStart) ~/ step) * step;
      while (tick <= endI && tick < segEnd) {
        final local = tick - segStart;
        final isMeasure = local % measureTicks == 0;
        final beatLocal = local % measureTicks;
        final isBeat = beatLocal % beatTicks == 0 && beatLocal > 0;
        final isSub = local % subTicks == 0;
        int? alpha;
        if (isMeasure) {
          alpha = 255;
        } else if (showBeat && isBeat) {
          alpha = 140;
        } else if (showSub && isSub) {
          alpha = 110;
        } else if (showTick || showSub) {
          alpha = 60;
        }
        if (alpha != null) {
          final x = xOf(tick.toDouble());
          if (x >= 0 && x <= size.width) {
            vPaint.color = level(alpha);
            canvas.drawLine(Offset(x, 0), Offset(x, size.height), vPaint);
          }
        }
        tick += step;
      }
    }
  }

  /// 与 Rust `fine_grid_preset` 同规则：在 ≤ ppq/4 的 2 分频候选里，
  /// 取屏幕格宽最接近 13px 的档位（1/32、1/64… 的惯性细分）。
  static int _fineGridStep(double ppu, int ppq) {
    final p = math.max(ppq, 1);
    final whole = p * 4;
    final candidates = <int>[];
    var den = 1;
    var lastTick = whole;
    while (den <= whole && whole % den == 0) {
      lastTick = (whole + den - 1) ~/ den;
      candidates.add(lastTick);
      if (den > 1 << 30) {
        break;
      }
      den *= 2;
    }
    var t = _nextPowerOfTwo(lastTick) >> 1;
    while (t >= 1) {
      candidates.add(t);
      t >>= 1;
    }
    final maxInterval = math.max(p ~/ 4, 1);
    var best = 1;
    var bestErr = double.infinity;
    var last = 1 << 62;
    for (final interval in candidates) {
      if (interval == 0 || interval > maxInterval || interval >= last) {
        continue;
      }
      last = interval;
      final px = interval * ppu;
      final err = math.log(px / 13.0).abs();
      if (err < bestErr) {
        bestErr = err;
        best = interval;
      }
    }
    return best;
  }

  static int _nextPowerOfTwo(int v) {
    var p = 1;
    while (p < v) {
      p <<= 1;
    }
    return p;
  }

  @override
  bool shouldRepaint(covariant AutomationPainter oldDelegate) => true;
}
