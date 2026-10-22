import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:yinhe_flutter/core/models.dart';

void main() {
  group('OverlayData', () {
    test('解析选框/单元格/选区矩形', () {
      final data = OverlayData.fromJson({
        'box': [1.0, 2.0, 3.0, 4.0],
        'cells': [
          [5, 6, 7, 8],
        ],
        'sel': [1, 2],
      });
      expect(data.box, const Rect.fromLTRB(1, 2, 3, 4));
      expect(data.cells, const [Rect.fromLTRB(5, 6, 7, 8)]);
      expect(data.sel, isEmpty);
    });

    test('缺省字段为空', () {
      final data = OverlayData.fromJson(const {});
      expect(data.box, isNull);
      expect(data.cells, isEmpty);
      expect(data.sel, isEmpty);
    });
  });

  group('TrackRow', () {
    test('解析轨道字段与颜色', () {
      final row = TrackRow.fromJson({
        'name': 'Piano',
        'port': 2,
        'channel': 3,
        'muted': true,
        'soloed': true,
        'color': [0.5, 0.25, 0, 1],
        'ib': 128,
        'ip': 25,
        'master': true,
      });
      expect(row.name, 'Piano');
      expect(row.port, 2);
      expect(row.channel, 3);
      expect(row.muted, isTrue);
      expect(row.soloed, isTrue);
      expect(row.color, const Color.fromRGBO(128, 64, 0, 1));
      expect(row.instrumentBank, 128);
      expect(row.instrumentProgram, 25);
      expect(row.master, isTrue);
    });

    test('缺省值', () {
      final row = TrackRow.fromJson(const {});
      expect(row.name, '');
      expect(row.port, 0);
      expect(row.channel, 0);
      expect(row.muted, isFalse);
      expect(row.instrumentBank, -1);
      expect(row.instrumentProgram, -1);
      expect(row.master, isFalse);
    });
  });

  group('自动化与标签', () {
    test('AutomationLaneData 解析点', () {
      final lane = AutomationLaneData.fromJson({
        'kind': 2,
        'name': 'CC 11 Expression',
        'points': [
          [0, 0.5],
          [480, 1.0],
        ],
      });
      expect(lane.kind, 2);
      expect(lane.name, 'CC 11 Expression');
      expect(lane.points, const [Offset(0, 0.5), Offset(480, 1.0)]);
    });

    test('RulerMarker 缺省值', () {
      final marker = RulerMarker.fromJson(const {});
      expect(marker.tick, 0);
      expect(marker.text, '');
    });

    test('RecentItem 缺省值', () {
      final item = RecentItem.fromJson(const {});
      expect(item.name, '');
      expect(item.path, '');
      expect(item.time, 0);
    });
  });

  group('LongPressContext', () {
    test('没有 tick 字段返回 null', () {
      expect(LongPressContext.fromJson(const {}), isNull);
    });

    test('解析位置与标志', () {
      final ctx = LongPressContext.fromJson({
        'tick': 123.5,
        'track': 4,
        'conductor': true,
        'pr': true,
      })!;
      expect(ctx.tick, 123.5);
      expect(ctx.track, 4);
      expect(ctx.conductor, isTrue);
      expect(ctx.pr, isTrue);
    });
  });
}
