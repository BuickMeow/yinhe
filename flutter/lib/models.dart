import 'package:flutter/material.dart';

/// 轨道面板行数据（Rust tracks_json）。
class TrackRow {
  const TrackRow({
    required this.name,
    required this.port,
    required this.channel,
    required this.muted,
    required this.soloed,
    required this.color,
  });

  final String name;
  final int port;
  final int channel;
  final bool muted;
  final bool soloed;
  final Color color;

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
    );
  }
}

/// 最近文件项（Rust recent_json）。
class RecentItem {  const RecentItem({
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
    required this.name,
    required this.points,
  });

  final int kind;
  final String name;
  final List<Offset> points;

  static AutomationLaneData fromJson(Map<String, dynamic> json) {
    final raw = (json['points'] as List<dynamic>?) ?? const [];
    return AutomationLaneData(
      kind: (json['kind'] as num?)?.toInt() ?? 1,
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
