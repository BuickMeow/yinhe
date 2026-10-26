/// 拍号分段与 `bar.beat.tick` 换算（与 Rust `yinhe_types::time_format` 对齐）。
///
/// 拍号随 MIDI/ying 里的原生事件变化时，小节号要跨段连续计数，
/// 且各段用各自的 measure/beat 长度（tau 这类一直变拍子的曲目）。
class BarBeatGrid {
  BarBeatGrid._(this.ppq, this.segments, this.barOffsets);

  /// 每四分音符 tick 数。
  final int ppq;

  /// 拍号分段：`(起始 tick, 分子, 分母的 2 次幂)`，首段从 0 开始、升序。
  final List<(int, int, int)> segments;

  /// 每段起点之前的累计完整小节数（bar 变化连续计数）。
  final List<int> barOffsets;

  /// 每小节 tick 数（`measure_ticks`：tpb * num / 2^den * 4）。
  static int measureTicks(int ppq, int num, int denPow) {
    if (ppq <= 0 || num <= 0) {
      return 1;
    }
    final den = 1 << denPow.clamp(0, 8);
    return ((ppq * num / den) * 4).round().clamp(1, 1 << 30);
  }

  /// 多小节合并的步长倍数（与桌面 `compute_measure_divisor` 同规则）。
  static int measureDivisor(double pixelsPerMeasure, double minPx) {
    if (pixelsPerMeasure >= minPx) {
      return 1;
    }
    var divisor = 1;
    while (pixelsPerMeasure * divisor < minPx) {
      divisor *= 2;
      if (divisor >= 65536) {
        break;
      }
    }
    return divisor;
  }

  /// 从 Rust 的平铺 `[tick,num,den_pow, ...]` 构建；数据不合法返回 null。
  static BarBeatGrid? fromFlat(double ppq, List<double> flat) {
    final tpb = ppq.round();
    if (tpb <= 0 || flat.length < 3) {
      return null;
    }
    final segments = <(int, int, int)>[];
    for (var i = 0; i + 2 < flat.length; i += 3) {
      final tick = flat[i].round().clamp(0, 1 << 31);
      final num = flat[i + 1].round().clamp(1, 32);
      final denPow = flat[i + 2].round().clamp(0, 8);
      segments.add((tick, num, denPow));
    }
    if (segments.isEmpty || segments.first.$1 != 0) {
      return null;
    }
    final offsets = List<int>.filled(segments.length, 0);
    var acc = 0;
    for (var i = 0; i + 1 < segments.length; i++) {
      offsets[i] = acc;
      final (start, num, denPow) = segments[i];
      final end = segments[i + 1].$1;
      final mticks = measureTicks(tpb, num, denPow);
      if (end > start) {
        acc += (end - start) ~/ mticks;
      }
    }
    offsets[segments.length - 1] = acc;
    return BarBeatGrid._(tpb, segments, offsets);
  }

  /// tick 所在段索引（`partition_point(start <= tick) - 1`）。
  int segmentIndexAt(int tick) {
    var lo = 0;
    var hi = segments.length;
    while (lo < hi) {
      final mid = (lo + hi) >> 1;
      if (segments[mid].$1 <= tick) {
        lo = mid + 1;
      } else {
        hi = mid;
      }
    }
    return (lo - 1).clamp(0, segments.length - 1);
  }

  /// 段的小节长 / 拍长（tick）。
  (int, int) ticksOfSegment(int index) {
    final (_, num, denPow) = segments[index];
    final mticks = measureTicks(ppq, num, denPow);
    return (mticks, (mticks ~/ num).clamp(1, mticks));
  }

  /// `1.0.000` 风格（bar 1 起、beat 0 起、tick 3 位；与旧安卓壳一致）。
  String format(double tick) {
    if (ppq <= 0) {
      return '1.0.000';
    }
    final t = tick < 0 ? 0 : tick.floor();
    final idx = segmentIndexAt(t);
    final (start, _, _) = segments[idx];
    final local = t - start;
    final (mticks, bticks) = ticksOfSegment(idx);
    final bar = barOffsets[idx] + local ~/ mticks + 1;
    final beat = (local % mticks) ~/ bticks;
    // tick 字段是"拍内 tick"：以本段实际拍长取模（分母 ≠ 4 时拍长 ≠ ppq）。
    final inBeat = local % bticks;
    return '$bar.$beat.${inBeat.toString().padLeft(3, '0')}';
  }
}
