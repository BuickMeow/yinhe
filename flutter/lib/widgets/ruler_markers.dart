import 'package:ffi/ffi.dart';
import 'package:flutter/material.dart';

import '../core/bar_beat.dart';
import '../core/ffi.dart' as rust;
import '../core/models.dart';
import '../painters/painters.dart';
import '../core/theme.dart';
import '../dialogs/inputs.dart';
import '../dialogs/sheet.dart';
import '../l10n/app_localizations.dart';

/// 时间标尺上的标签（marker）层：pill 显示 + 点击跳转 / 长按菜单 / 拖动移动。
///
/// 与桌面 `time_ruler` 的标签行为对齐（桌面：点击跳转、双击改名、右键删除、
/// 拖动移动；移动端映射为点击跳转、长按菜单、水平拖动）。
class MarkerLayer extends StatefulWidget {
  const MarkerLayer({
    super.key,
    required this.snap,
    required this.grid,
    required this.markers,
    required this.onChanged,
  });

  final ValueNotifier<List<double>> snap;
  final ValueNotifier<BarBeatGrid?> grid;
  final List<RulerMarker> markers;
  final VoidCallback onChanged;

  @override
  State<MarkerLayer> createState() => _MarkerLayerState();
}

class _MarkerLayerState extends State<MarkerLayer> {
  /// 拖动预览：原 tick + 当前 x（画布逻辑像素）。
  int? _dragTick;
  double? _dragX;

  /// 小节.拍.tick（1.0.000 风格；按拍号分段 = 原生事件优先，否则工程默认）。
  String _formatTick(double tick, double ppq) {
    final grid = widget.grid.value;
    if (grid != null) {
      return grid.format(tick);
    }
    return '${tick.round()}';
  }

  Future<void> _showMarkerMenu(RulerMarker m) async {
    final l = AppLocalizations.of(context);
    final action = await showActionSheet(
      context,
      cancelLabel: l.cancel,
      actions: [
        SheetAction(value: 'rename', label: l.markerRename, icon: Icons.edit),
        SheetAction(
          value: 'delete',
          label: l.markerDelete,
          icon: Icons.delete_outline,
          danger: true,
        ),
      ],
    );
    if (!mounted || action == null) {
      return;
    }
    if (action == 'delete') {
      if (rust.markerDelete(m.tick.toDouble()) == 1) {
        widget.onChanged();
      }
      return;
    }
    final text = await showTextInputDialog(
      context,
      title: l.markerRename,
      label: l.markerText,
      initial: m.text,
      confirmText: l.save,
    );
    if (!mounted || text == null) {
      return;
    }
    final p = text.toNativeUtf8();
    var ok = 0;
    try {
      ok = rust.markerRename(m.tick.toDouble(), p);
    } finally {
      calloc.free(p);
    }
    if (ok == 1) {
      widget.onChanged();
    }
  }

  /// 空白标尺长按：在该位置添加标签（文本可留空 = 显示小节位置）。
  Future<void> _addMarkerAt(double x) async {
    final l = AppLocalizations.of(context);
    final tick = rust.tickAtX(x);
    final text = await showTextInputDialog(
      context,
      title: '${l.add} ${l.markerLabel}',
      label: l.markerTextOptional,
      confirmText: l.add,
    );
    if (!mounted || text == null) {
      return;
    }
    final p = text.toNativeUtf8();
    var ok = 0;
    try {
      ok = rust.markerInsert(tick, p);
    } finally {
      calloc.free(p);
    }
    if (ok == 1) {
      widget.onChanged();
    }
  }

  @override
  Widget build(BuildContext context) {
    return ValueListenableBuilder<List<double>>(
      valueListenable: widget.snap,
      builder: (context, s, _) {
        if (s.length < 12) {
          return const SizedBox.shrink();
        }
        final ppu = s[1];
        final scrollX = s[2];
        final origin = s[5];
        final ppq = s[8] > 0 ? s[8] : 480.0;
        if (ppu <= 0) {
          return const SizedBox.shrink();
        }
        final width = MediaQuery.sizeOf(context).width;
        double xOf(int tick) => tick * ppu - scrollX + origin;
        final sorted = [...widget.markers]..sort((a, b) => a.tick - b.tick);
        return GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTapUp: (d) => rust.jumpToTick(rust.tickAtX(d.localPosition.dx)),
          onLongPressStart: (d) => _addMarkerAt(d.localPosition.dx),
          child: Stack(
            children: [
              for (final m in sorted)
                if (_visible(xOf(m.tick), width))
                  _pill(
                    m,
                    _dragTick == m.tick && _dragX != null
                        ? _dragX!
                        : xOf(m.tick),
                    width,
                    ppq,
                  ),
              // AR：面板上方的占位矩形（与时间标尺夹着），盖住跨入面板区域的
              // 标签 pill。只画在 AR（快照 mode 0）。
              if (s[0] == 0.0 && origin > 0)
                Positioned(
                  left: 0,
                  top: 0,
                  child: Container(
                    width: origin,
                    height: RulerPainter.height,
                    decoration: BoxDecoration(
                      color: YinheTheme.trackBg,
                      border: Border(
                        bottom: BorderSide(color: YinheTheme.lineFg),
                      ),
                    ),
                  ),
                ),
            ],
          ),
        );
      },
    );
  }

  bool _visible(double x, double width) => x >= -60 && x <= width + 60;

  Widget _pill(RulerMarker m, double x, double width, double ppq) {
    final text = m.text.isNotEmpty
        ? m.text
        : _formatTick(m.tick.toDouble(), ppq);
    return Positioned(
      left: x + 1,
      top: 0,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () => rust.jumpToTick(m.tick.toDouble()),
        onLongPress: () => _showMarkerMenu(m),
        onHorizontalDragStart: (_) => setState(() {
          _dragTick = m.tick;
          _dragX = x;
        }),
        onHorizontalDragUpdate: (d) => setState(() {
          _dragX = (_dragX! + d.delta.dx).clamp(0.0, width);
        }),
        onHorizontalDragEnd: (_) {
          final to = _dragX;
          setState(() {
            _dragTick = null;
            _dragX = null;
          });
          if (to != null && (to - x).abs() > 1.0) {
            if (rust.markerMove(m.tick.toDouble(), to) == 1) {
              widget.onChanged();
            }
          }
        },
        child: Container(
          margin: const EdgeInsets.only(top: 1, bottom: 1),
          padding: const EdgeInsets.symmetric(horizontal: 4),
          decoration: BoxDecoration(
            color: const Color(0xFFFAFAFA),
            borderRadius: BorderRadius.circular(3),
            border: Border.all(color: YinheTheme.lineFg),
          ),
          child: Text(
            text,
            maxLines: 1,
            style: TextStyle(color: YinheTheme.text, fontSize: 10),
          ),
        ),
      ),
    );
  }
}
