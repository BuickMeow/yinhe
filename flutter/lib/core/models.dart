import 'package:flutter/material.dart';

/// 覆盖层数据（选框/预览，画布内容逻辑坐标）。
class OverlayData {
  const OverlayData({this.box, this.cells = const [], this.sel = const []});

  final Rect? box;
  final List<Rect> cells;
  final List<Rect> sel;

  static const empty = OverlayData();

  static OverlayData fromJson(Map<String, dynamic> json) {
    Rect? fromList(dynamic v) {
      if (v is! List || v.length != 4) {
        return null;
      }
      return Rect.fromLTRB(
        (v[0] as num).toDouble(),
        (v[1] as num).toDouble(),
        (v[2] as num).toDouble(),
        (v[3] as num).toDouble(),
      );
    }

    List<Rect> many(dynamic v) {
      if (v is! List) {
        return const [];
      }
      return [for (final e in v) ?fromList(e)];
    }

    return OverlayData(
      box: fromList(json['box']),
      cells: many(json['cells']),
      sel: many(json['sel']),
    );
  }
}

/// 轨道面板行数据（Rust tracks_json）。
class TrackRow {
  const TrackRow({
    required this.name,
    required this.port,
    required this.channel,
    required this.muted,
    required this.soloed,
    required this.color,
    this.instrumentBank = -1,
    this.instrumentProgram = -1,
    this.master = false,
  });

  final String name;
  final int port;
  final int channel;
  final bool muted;
  final bool soloed;
  final Color color;

  /// 所属通道的默认乐器（-1 = 未设置）。
  final int instrumentBank;
  final int instrumentProgram;

  /// Master（Conductor）轨：不能有音符，只承载全局事件。
  final bool master;

  static TrackRow fromJson(Map<String, dynamic> json) {
    final c = (json['color'] as List<dynamic>?) ?? const [];
    double ch(int i, double fallback) =>
        (i < c.length ? (c[i] as num).toDouble() : fallback);
    return TrackRow(
      name: json['name'] as String? ?? '',
      port: (json['port'] as num?)?.toInt() ?? 0,
      channel: (json['channel'] as num?)?.toInt() ?? 0,
      muted: json['muted'] as bool? ?? false,
      soloed: json['soloed'] as bool? ?? false,
      color: Color.fromRGBO(
        (ch(0, 0.5) * 255).round(),
        (ch(1, 0.5) * 255).round(),
        (ch(2, 0.5) * 255).round(),
        ch(3, 1.0),
      ),
      instrumentBank: (json['ib'] as num?)?.toInt() ?? -1,
      instrumentProgram: (json['ip'] as num?)?.toInt() ?? -1,
      master: json['master'] as bool? ?? false,
    );
  }
}

/// 长按空白处的上下文（插入事件用）。
class LongPressContext {
  const LongPressContext({
    required this.tick,
    required this.track,
    required this.conductor,
    required this.pr,
  });

  final double tick;
  final int track;
  final bool conductor;
  final bool pr;

  static LongPressContext? fromJson(Map<String, dynamic> json) {
    if (!json.containsKey('tick')) {
      return null;
    }
    return LongPressContext(
      tick: (json['tick'] as num?)?.toDouble() ?? 0,
      track: (json['track'] as num?)?.toInt() ?? -1,
      conductor: json['conductor'] as bool? ?? false,
      pr: json['pr'] as bool? ?? false,
    );
  }
}

/// 最近文件项（Rust recent_json）。
class RecentItem {
  const RecentItem({
    required this.name,
    required this.path,
    required this.time,
  });

  final String name;
  final String path;
  final int time;

  static RecentItem fromJson(Map<String, dynamic> json) {
    return RecentItem(
      name: json['name'] as String? ?? '',
      path: json['path'] as String? ?? '',
      time: (json['time'] as num?)?.toInt() ?? 0,
    );
  }
}

/// 自动化 lane（PB/CC/RPN）：points 的 x=tick、y=归一化值 0..1。
class AutomationLaneData {
  const AutomationLaneData({
    required this.kind,
    required this.number,
    required this.max,
    required this.name,
    required this.points,
  });

  final int kind;

  /// CC / RPN 编号（PB 等无编号时为 -1）。
  final int number;

  /// 归一化值还原上限（CC=127、PB=16383 等）。
  final double max;
  final String name;
  final List<Offset> points;

  /// 归一化值 → 顶部栏显示的原始值文本。
  ///
  /// Tempo（kind 0）用 BPM，保留最多两位小数；其他 target 取整。
  String formatValue(double value) {
    final raw = value.clamp(0.0, 1.0) * max;
    if (kind == 0) {
      final text = raw.toStringAsFixed(2);
      return text.replaceFirst(RegExp(r'\.?0+$'), '');
    }
    return '${raw.round()}';
  }

  static AutomationLaneData fromJson(Map<String, dynamic> json) {
    final raw = (json['points'] as List<dynamic>?) ?? const [];
    return AutomationLaneData(
      kind: (json['kind'] as num?)?.toInt() ?? 1,
      number: (json['num'] as num?)?.toInt() ?? -1,
      max: (json['max'] as num?)?.toDouble() ?? 1.0,
      name: json['name'] as String? ?? '',
      points: [
        for (final p in raw)
          Offset(
            ((p as List<dynamic>)[0] as num).toDouble(),
            (p[1] as num).toDouble(),
          ),
      ],
    );
  }
}

/// 标尺标签（MIDI marker）。
class RulerMarker {
  const RulerMarker({required this.tick, required this.text});

  final int tick;
  final String text;

  static RulerMarker fromJson(Map<String, dynamic> json) => RulerMarker(
    tick: (json['tick'] as num?)?.toInt() ?? 0,
    text: json['text'] as String? ?? '',
  );
}

/// 长按空白处的位置指示（插入/粘贴落点）。
class LongPressIndicator {
  const LongPressIndicator({
    required this.tick,
    required this.track,
    required this.pr,
  });

  final double tick;
  final int track;
  final bool pr;
}
