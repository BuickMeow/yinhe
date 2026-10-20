import 'package:flutter_test/flutter_test.dart';
import 'package:yinhe_flutter/core/bar_beat.dart';

void main() {
  group('measureTicks', () {
    test('4/4、3/4、6/8、7/8', () {
      expect(BarBeatGrid.measureTicks(480, 4, 2), 1920);
      expect(BarBeatGrid.measureTicks(480, 3, 2), 1440);
      expect(BarBeatGrid.measureTicks(480, 6, 3), 1440);
      expect(BarBeatGrid.measureTicks(480, 7, 3), 1680);
    });

    test('非法输入返回 1，避免除零', () {
      expect(BarBeatGrid.measureTicks(0, 4, 2), 1);
      expect(BarBeatGrid.measureTicks(480, 0, 2), 1);
    });
  });

  group('fromFlat', () {
    test('ppq/长度/首段起点非法时返回 null', () {
      expect(BarBeatGrid.fromFlat(0, [0, 4, 2]), isNull);
      expect(BarBeatGrid.fromFlat(480, [0, 4]), isNull);
      expect(BarBeatGrid.fromFlat(480, [1, 4, 2]), isNull);
    });

    test('读取分段并夹取范围', () {
      final grid = BarBeatGrid.fromFlat(480, [0, 4, 2, 1920, 99, 3])!;
      expect(grid.segments.length, 2);
      expect(grid.segments[1], (1920, 32, 3));
    });
  });

  group('format', () {
    test('单个 4/4：小节/拍/拍内 tick', () {
      final grid = BarBeatGrid.fromFlat(480, [0, 4, 2])!;
      expect(grid.format(0), '1.0.000');
      expect(grid.format(479), '1.0.479');
      expect(grid.format(480), '1.1.000');
      expect(grid.format(1919), '1.3.479');
      expect(grid.format(-5), '1.0.000');
    });

    test('拍内 tick 以本段实际拍长取模（不是固定 ppq）', () {
      // 6/8：每小节 1440 tick，每拍 240 tick。
      final grid = BarBeatGrid.fromFlat(480, [0, 6, 3])!;
      expect(grid.format(239), '1.0.239');
      expect(grid.format(240), '1.1.000');
      expect(grid.format(480), '1.2.000');
      expect(grid.format(1439), '1.5.239');
      expect(grid.format(1440), '2.0.000');
      // tick 480 若按 ppq=480 取模会错误显示 1.2.480。
      expect(grid.format(720), '1.3.000');
    });

    test('变拍号跨段：小节号连续、各段用自己的拍长', () {
      final grid = BarBeatGrid.fromFlat(480, [0, 4, 2, 1920, 3, 2])!;
      expect(grid.format(1919), '1.3.479');
      expect(grid.format(1920), '2.0.000');
      expect(grid.format(1920 + 480), '2.1.000');
      expect(grid.format(1920 + 1440), '3.0.000');
    });

    test('segmentIndexAt 边界与越界', () {
      final grid = BarBeatGrid.fromFlat(480, [0, 4, 2, 1920, 3, 2])!;
      expect(grid.segmentIndexAt(0), 0);
      expect(grid.segmentIndexAt(1919), 0);
      expect(grid.segmentIndexAt(1920), 1);
      expect(grid.segmentIndexAt(1 << 30), 1);
    });
  });

  group('measureDivisor', () {
    test('按最小像素宽度合并多小节', () {
      expect(BarBeatGrid.measureDivisor(120, 24), 1);
      expect(BarBeatGrid.measureDivisor(12, 24), 2);
      expect(BarBeatGrid.measureDivisor(3, 24), 8);
      expect(BarBeatGrid.measureDivisor(0.1, 24), 64);
    });
  });
}
