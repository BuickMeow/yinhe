import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../core/bar_beat.dart';
import '../core/theme.dart';

/// 与 Rust `push_time_grid` 同规则的竖向时间网格：
/// 合并小节 / 小节 / 拍 / 细分 / tick，四级透明度。
///
/// [leftInset] 是时间轴原点在画布内的 x：
/// PR 用键盘宽度，AR 用轨道面板宽度，AM 用 `键盘宽度 - combo 列宽`。
void paintTimeGrid(
  Canvas canvas, {
  required Size size,
  required double ppu,
  required double scrollX,
  required double leftInset,
  required BarBeatGrid grid,
  double minSpacing = 38.0,
  bool withSubTick = true,
}) {
  if (ppu <= 0 || size.width <= 0 || size.height <= 0) {
    return;
  }
  final tpb = grid.ppq;
  if (tpb <= 0) {
    return;
  }
  final subTicks = math.max(tpb ~/ 4, 1);
  final fineStep = fineGridStep(ppu, tpb);
  final startTick = ((scrollX - leftInset) / ppu).clamp(0.0, double.infinity);
  final endTick = ((scrollX - leftInset + size.width) / ppu).clamp(
    0.0,
    double.infinity,
  );
  final endI = endTick.floor();
  double xOf(double tick) => tick * ppu - scrollX + leftInset;

  final paint = Paint()..strokeWidth = 1.0;
  final line = YinheTheme.lineFg;
  Color level(int alpha) => line.withValues(alpha: alpha / 255.0);

  for (var i = 0; i < grid.segments.length; i++) {
    final (segStart, _, _) = grid.segments[i];
    final segEnd = i + 1 < grid.segments.length
        ? grid.segments[i + 1].$1
        : 1 << 31;
    if (segStart > endTick) {
      break;
    }
    final (measureTicks, beatTicks) = grid.ticksOfSegment(i);
    // 与 wgpu `push_time_grid` 相同的层级选择：合并小节/小节/拍/细分/tick。
    final divisor = BarBeatGrid.measureDivisor(measureTicks * ppu, minSpacing);
    final merged = divisor > 1;
    final mergedMeasureTicks = measureTicks * divisor;
    final showBeat = !merged;
    final showSub = withSubTick && showBeat && beatTicks * ppu >= minSpacing;
    final showTick = withSubTick && showSub && ppu >= 10.0;
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
          paint.color = level(alpha);
          canvas.drawLine(Offset(x, 0), Offset(x, size.height), paint);
        }
      }
      tick += step;
    }
  }
}

/// 与 Rust `fine_grid_preset` 同规则：在 ≤ ppq/4 的 2 分频候选里，
/// 取屏幕格宽最接近 13px 的档位（1/32、1/64… 的惯性细分）。
int fineGridStep(double ppu, int ppq) {
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

int _nextPowerOfTwo(int v) {
  var p = 1;
  while (p < v) {
    p <<= 1;
  }
  return p;
}
