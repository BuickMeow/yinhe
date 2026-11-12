import 'package:flutter/material.dart';

import '../core/ffi.dart' as rust;
import '../core/models.dart';
import '../core/snap_fields.dart';
import '../core/theme.dart';

/// AR 事件行行高（逻辑像素）。
const double kEventRowHeight = 18.0;

/// 事件行顺序与短标签（与桌面 `EventRowKind` 一致，英文标签避免新增 15 语言 key）。
const List<String> kEventRowKinds = [
  'loop',
  'marker',
  'meter',
  'key',
  'chord',
  'lyrics',
];

String eventRowLabel(String kind) => switch (kind) {
  'loop' => 'LOOP',
  'marker' => 'MARKER',
  'meter' => 'METER',
  'key' => 'KEY',
  'chord' => 'CHORD',
  _ => 'LYRIC',
};

/// 时间标尺与音轨之间的固定事件行。
///
/// LOOP 行可整段拖动/两端改边界；其余行只读展示，点击定位播放头。
class EventRowsBar extends StatefulWidget {
  const EventRowsBar({
    super.key,
    required this.snap,
    required this.loop,
    required this.data,
    required this.visibleKinds,
  });

  final ValueNotifier<List<double>> snap;
  final ValueNotifier<LoopState> loop;
  final EventRowsData data;
  final List<String> visibleKinds;

  @override
  State<EventRowsBar> createState() => _EventRowsBarState();
}

class _EventRowsBarState extends State<EventRowsBar> {
  String? _dragMode;
  double _dragGrabOffset = 0;
  int _origStart = 0;
  int _origEnd = 0;

  @override
  Widget build(BuildContext context) {
    final rows = widget.visibleKinds.length;
    if (rows == 0) {
      return const SizedBox.shrink();
    }
    return SizedBox(
      height: rows * kEventRowHeight,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTapUp: _handleTap,
        onHorizontalDragStart: _handleDragStart,
        onHorizontalDragUpdate: _handleDragUpdate,
        onHorizontalDragEnd: (_) => _endDrag(),
        onHorizontalDragCancel: _endDrag,
        child: ValueListenableBuilder<List<double>>(
          valueListenable: widget.snap,
          builder: (context, snap, _) => ValueListenableBuilder<LoopState>(
            valueListenable: widget.loop,
            builder: (context, loop, _) => CustomPaint(
              painter: _EventRowsPainter(
                snap: snap,
                loop: loop,
                data: widget.data,
                kinds: widget.visibleKinds,
              ),
              child: const SizedBox.expand(),
            ),
          ),
        ),
      ),
    );
  }

  void _handleTap(TapUpDetails d) {
    final s = widget.snap.value;
    if (s.length <= SnapField.ppu) {
      return;
    }
    final kind = _kindAt(d.localPosition.dy);
    if (kind == 'loop') {
      final (x0, x1) = _loopX(s);
      if (d.localPosition.dx >= x0 - 10 && d.localPosition.dx <= x1 + 10) {
        return;
      }
    }
    rust.jumpToTick(rust.tickAtX(d.localPosition.dx));
  }

  String? _kindAt(double y) {
    final idx = (y / kEventRowHeight).floor();
    if (idx < 0 || idx >= widget.visibleKinds.length) {
      return null;
    }
    return widget.visibleKinds[idx];
  }

  (double, double) _loopX(List<double> s) {
    final ppu = s[SnapField.ppu];
    final scrollX = s[SnapField.scrollX];
    final origin = s[SnapField.leftPanelW];
    final loop = widget.loop.value;
    if (ppu <= 0) {
      return (origin, origin);
    }
    return (
      loop.start * ppu - scrollX + origin,
      loop.end * ppu - scrollX + origin,
    );
  }

  void _handleDragStart(DragStartDetails d) {
    final kind = _kindAt(d.localPosition.dy);
    _dragMode = null;
    if (kind != 'loop') {
      return;
    }
    final s = widget.snap.value;
    if (s.length <= SnapField.ppu || s[SnapField.ppu] <= 0) {
      return;
    }
    final (x0, x1) = _loopX(s);
    final x = d.localPosition.dx;
    final loop = widget.loop.value;
    _origStart = loop.start;
    _origEnd = loop.end;
    if ((x - x0).abs() <= 12) {
      _dragMode = 'left';
    } else if ((x - x1).abs() <= 12) {
      _dragMode = 'right';
    } else if (x > x0 && x < x1) {
      _dragMode = 'move';
      final grabTick = rust.tickAtX(x);
      _dragGrabOffset = _origStart - grabTick;
    }
  }

  void _handleDragUpdate(DragUpdateDetails d) {
    final mode = _dragMode;
    if (mode == null) {
      return;
    }
    final s = widget.snap.value;
    if (s.length <= SnapField.ppu) {
      return;
    }
    final x = d.localPosition.dx;
    if (mode == 'move') {
      final tick = rust.tickAtX(x) + _dragGrabOffset;
      final start = tick.round().clamp(0, 0xFFFFFFFF);
      final len = _origEnd - _origStart;
      _setRange(start, start + len);
      return;
    }
    final tick = rust.tickAtX(x).round();
    if (mode == 'left') {
      _setRange(tick.clamp(0, _origEnd - 1), _origEnd);
    } else {
      _setRange(_origStart, tick.clamp(_origStart + 1, 0xFFFFFFFF));
    }
  }

  void _setRange(int start, int end) {
    if (end <= start) {
      return;
    }
    if (rust.loopSetRange(start.toDouble(), end.toDouble()) != 1) {
      return;
    }
    widget.loop.value = LoopState(enabled: true, start: start, end: end);
  }

  void _endDrag() {
    _dragMode = null;
  }
}

/// 左上角（音轨面板/键盘与时间标尺夹角）的 EVENT 菜单按钮。
class EventMenuButton extends StatelessWidget {
  const EventMenuButton({
    super.key,
    required this.loop,
    required this.enabledKinds,
    required this.onToggleRow,
  });

  final ValueNotifier<LoopState> loop;
  final Set<String> enabledKinds;
  final void Function(String kind, bool enabled) onToggleRow;

  Future<void> _openMenu(BuildContext context) async {
    var loopOn = loop.value.enabled;
    final enabled = Set<String>.of(enabledKinds);
    await showDialog<void>(
      context: context,
      builder: (ctx) => StatefulBuilder(
        builder: (ctx, setLocal) => AlertDialog(
          backgroundColor: YinheTheme.bg,
          title: Text(
            'EVENT',
            style: TextStyle(color: YinheTheme.text, fontSize: 15),
          ),
          contentPadding: const EdgeInsets.symmetric(vertical: 8),
          content: SizedBox(
            width: 200,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                for (final kind in kEventRowKinds)
                  CheckboxListTile(
                    dense: true,
                    visualDensity: VisualDensity.compact,
                    contentPadding: const EdgeInsets.symmetric(horizontal: 8),
                    controlAffinity: ListTileControlAffinity.leading,
                    value: kind == 'loop' ? loopOn : enabled.contains(kind),
                    title: Text(
                      eventRowLabel(kind),
                      style: TextStyle(color: YinheTheme.text, fontSize: 13),
                    ),
                    onChanged: (value) {
                      final on = value ?? false;
                      setLocal(() {
                        if (kind == 'loop') {
                          loopOn = on;
                        } else if (on) {
                          enabled.add(kind);
                        } else {
                          enabled.remove(kind);
                        }
                      });
                      onToggleRow(kind, on);
                    },
                  ),
              ],
            ),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.of(ctx).pop(),
              child: const Text('OK'),
            ),
          ],
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return InkWell(
      onTap: () => _openMenu(context),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
        child: Text(
          'EVENT',
          style: TextStyle(
            color: YinheTheme.textSecondary,
            fontSize: 11,
            fontWeight: FontWeight.w600,
          ),
        ),
      ),
    );
  }
}

class _EventRowsPainter extends CustomPainter {
  _EventRowsPainter({
    required this.snap,
    required this.loop,
    required this.data,
    required this.kinds,
  });

  final List<double> snap;
  final LoopState loop;
  final EventRowsData data;
  final List<String> kinds;

  void _text(Canvas canvas, String text, Offset pos, Color color) {
    final tp = TextPainter(
      text: TextSpan(
        text: text,
        style: TextStyle(fontSize: 10, color: color),
      ),
      textDirection: TextDirection.ltr,
      maxLines: 1,
      ellipsis: '…',
    )..layout(maxWidth: 120);
    tp.paint(canvas, Offset(pos.dx, pos.dy - tp.height / 2));
  }

  @override
  void paint(Canvas canvas, Size size) {
    if (size.width <= 0 || size.height <= 0 || snap.length <= SnapField.ppu) {
      return;
    }
    final ppu = snap[SnapField.ppu];
    final scrollX = snap[SnapField.scrollX];
    final origin = snap[SnapField.leftPanelW];
    if (ppu <= 0 || origin <= 0) {
      return;
    }
    final contentW = size.width - origin;
    if (contentW <= 0) {
      return;
    }
    final textColor = YinheTheme.textSecondary;
    final muted = YinheTheme.textMuted;
    for (var i = 0; i < kinds.length; i++) {
      final kind = kinds[i];
      final top = i * kEventRowHeight;
      final row = Rect.fromLTWH(0, top, size.width, kEventRowHeight);
      canvas.drawRect(
        row,
        Paint()..color = i.isEven ? YinheTheme.bg : YinheTheme.controlBg,
      );
      canvas.drawRect(
        Rect.fromLTWH(0, top, origin, kEventRowHeight),
        Paint()..color = YinheTheme.controlBg,
      );
      canvas.drawLine(
        Offset(0, top + kEventRowHeight - 0.5),
        Offset(size.width, top + kEventRowHeight - 0.5),
        Paint()..color = YinheTheme.lineFg,
      );
      _text(
        canvas,
        eventRowLabel(kind),
        Offset(8, top + kEventRowHeight / 2),
        muted,
      );

      final clip = Rect.fromLTWH(origin, top, contentW, kEventRowHeight);
      canvas.save();
      canvas.clipRect(clip);
      if (kind == 'loop') {
        _paintLoop(canvas, clip, ppu, scrollX, origin);
      } else {
        _paintEvents(
          canvas,
          clip,
          _eventsFor(kind),
          ppu,
          scrollX,
          origin,
          kind == 'chord' || kind == 'lyrics' ? muted : textColor,
        );
      }
      canvas.restore();
    }
  }

  List<RulerMarker> _eventsFor(String kind) => switch (kind) {
    'marker' => data.markers,
    'meter' => data.meter,
    'key' => data.key,
    'chord' => data.chord,
    _ => data.lyrics,
  };

  void _paintLoop(
    Canvas canvas,
    Rect clip,
    double ppu,
    double scrollX,
    double origin,
  ) {
    final x0 = (loop.start * ppu - scrollX + origin).clamp(
      clip.left,
      clip.right,
    );
    final x1 = (loop.end * ppu - scrollX + origin).clamp(clip.left, clip.right);
    if (x1 <= x0) {
      return;
    }
    final band = Rect.fromLTRB(x0, clip.top + 2, x1, clip.bottom - 2);
    final accent = YinheTheme.accent;
    canvas.drawRRect(
      RRect.fromRectAndRadius(band, const Radius.circular(2)),
      Paint()..color = accent.withValues(alpha: 0.18),
    );
    canvas.drawRRect(
      RRect.fromRectAndRadius(band, const Radius.circular(2)),
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1
        ..color = accent,
    );
    final handle = Paint()..color = accent;
    canvas.drawRect(Rect.fromLTWH(x0 - 1, band.top, 2, band.height), handle);
    canvas.drawRect(Rect.fromLTWH(x1 - 1, band.top, 2, band.height), handle);
  }

  void _paintEvents(
    Canvas canvas,
    Rect clip,
    List<RulerMarker> events,
    double ppu,
    double scrollX,
    double origin,
    Color color,
  ) {
    var lastTextX = -1e9;
    for (final e in events) {
      final x = e.tick * ppu - scrollX + origin;
      if (x < clip.left) {
        continue;
      }
      if (x > clip.right) {
        break;
      }
      canvas.drawLine(
        Offset(x, clip.top + 2),
        Offset(x, clip.bottom - 2),
        Paint()..color = YinheTheme.lineFg,
      );
      if (e.text.isEmpty || x - lastTextX < 38) {
        continue;
      }
      _text(canvas, e.text, Offset(x + 4, clip.center.dy), color);
      lastTextX = x;
    }
  }

  @override
  bool shouldRepaint(covariant _EventRowsPainter oldDelegate) => true;
}
