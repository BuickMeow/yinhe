import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../core/bar_beat.dart';
import '../core/edit_status.dart';
import '../core/ffi.dart' as rust;
import '../core/midi_names.dart';
import '../core/models.dart';
import '../painters/painters.dart';
import '../l10n/app_localizations.dart';
import '../core/theme.dart';

/// PR 底部自动化栏：combo 列 + 自动化/力度绘制 + 锚点拖动。
///
/// 面板高度由外部控制（[height]），顶部有一条拖拽手柄；展开时 PR 视口会
/// 同步缩短，两者上下排列、互不覆盖。
class AutomationPanel extends StatefulWidget {
  const AutomationPanel({
    super.key,
    required this.height,
    required this.snap,
    required this.grid,
    required this.lanes,
    required this.velocity,
    required this.trackColors,
    required this.activeTrack,
    required this.status,
    required this.formatTick,
    required this.onCollapsed,
    required this.onReload,
    required this.onVelocityCommit,
    required this.velocityDrawEnabled,
    required this.onResize,
    required this.onResetHeight,
  });

  final double height;
  final ValueNotifier<List<double>> snap;

  /// 拍号分段（与 PR 共用，保证小节/拍网格对齐）。
  final ValueNotifier<BarBeatGrid?> grid;
  final List<AutomationLaneData> lanes;
  final List<(double, double, double, int)> velocity;
  final List<Color> trackColors;
  final int activeTrack;

  /// 顶部栏临时读数（力度 / 自动化 / 框选）。
  final ValueNotifier<EditStatus?> status;

  /// tick → 顶部栏位置文本（跟随时间码的小节/秒模式）。
  final String Function(double tick) formatTick;
  final ValueChanged<bool> onCollapsed;
  final VoidCallback onReload;

  /// 提交一笔力度绘制点 `(tick, velocity 1..127)`。
  final void Function(List<(double, double)> points) onVelocityCommit;

  /// 铅笔工具下才允许在力度栏落笔，避免手形/刷子误改力度。
  final bool velocityDrawEnabled;

  /// 拖动顶部手柄：`dy` 为本次垂直位移（向上为负 = 变高）。
  final ValueChanged<double> onResize;

  /// 双击顶部手柄恢复默认高度。
  final VoidCallback onResetHeight;

  /// 最小高度：保留旧版 81px 绘制区 + 顶部手柄。
  static const double minHeight = 97.0;

  /// 首次展开的默认高度：120px 绘制区，比旧版高一些方便手指绘制。
  static const double defaultHeight = 136.0;

  /// 顶部拖拽手柄占用的高度。
  static const double handleHeight = 16.0;

  /// 绘制区实际可用高度（面板高度扣除拖拽手柄）。
  double get canvasHeight =>
      (height - handleHeight).clamp(1.0, double.infinity);

  @override
  State<AutomationPanel> createState() => _AutomationPanelState();
}

class _AutomationPanelState extends State<AutomationPanel> {
  AppLocalizations get l => AppLocalizations.of(context);

  int? _selectedLane; // null = Velocity
  (int, int)? _pendingTarget; // ensureAutomationLane 后等待 lanes 刷新
  final ValueNotifier<List<(double, double, double, int)>> _velocityPreview =
      ValueNotifier(const []);
  bool _velocityStrokeActive = false;
  final List<(double, double)> _velocityPoints = [];
  final Map<(int, int), (double, double, double, int)> _velocityTouched = {};

  /// AM 锚点选择（按 tick）。
  final Set<int> _selectedTicks = {};

  /// 单个锚点点选：可同时改 tick + value；否则批量只改 tick。
  bool _singleSelection = false;

  /// 拖动/框选状态（绘制区局部坐标）。
  double? _dragAnchorTick;
  double? _dragStartSnapTick;
  double? _dragStartValue;
  bool _marqueeActive = false;
  Offset? _marqueeStart;
  Offset? _marqueeCurrent;

  /// old tick -> (new tick, new value)，拖动中的预览。
  Map<int, (int, double)> _movePreview = {};

  @override
  void didUpdateWidget(covariant AutomationPanel oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.activeTrack != widget.activeTrack) {
      _selectedLane = null;
      _pendingTarget = null;
      _clearVelocityStroke();
      _clearSelection();
    } else if (_selectedLane != null) {
      if (_selectedLane! >= widget.lanes.length) {
        _selectedLane = null;
        _clearSelection();
      } else if (_canonicalLabel(widget.lanes[_selectedLane!]) == null) {
        // 旧列表里不支持的 CC（如 CC1/CC91）不再出现在 AM 下拉中。
        _selectedLane = null;
        _clearSelection();
      }
    }
    final pending = _pendingTarget;
    if (pending != null) {
      final idx = _findLane(pending.$1, pending.$2);
      if (idx >= 0) {
        _selectedLane = idx;
        _pendingTarget = null;
      }
    }
  }

  /// 当前选中的 lane 下标（null = 力度）。
  int? get _laneIdx =>
      _selectedLane != null && _selectedLane! < widget.lanes.length
      ? _selectedLane
      : null;

  /// 按插入自动化的 target 语义查找 lane（PB/RPN 兼容 MIDI 导入的 Param 映射）。
  int _findLane(int kind, int number) {
    for (var i = 0; i < widget.lanes.length; i++) {
      if (_matchesTarget(widget.lanes[i], kind, number)) {
        return i;
      }
    }
    return -1;
  }

  bool _matchesTarget(AutomationLaneData lane, int kind, int number) {
    if (lane.kind != kind) {
      return false;
    }
    // PB：Param{ChannelInstrument, PITCH_BEND} 也映射为 kind 0。
    if (kind == 1) {
      return true;
    }
    if (lane.number == number) {
      return true;
    }
    // 导入的 RPN 可能是 Param lane（number=-1），按名称兜底匹配。
    if (kind == 2 && lane.number < 0) {
      final name = MidiNames.instance.rpn[number];
      return name != null && name.isNotEmpty && lane.name.contains(name);
    }
    return false;
  }

  @override
  void dispose() {
    _velocityPreview.dispose();
    super.dispose();
  }

  void _clearVelocityStroke() {
    _velocityStrokeActive = false;
    _velocityPoints.clear();
    _velocityTouched.clear();
    _velocityPreview.value = const [];
  }

  void _clearSelection() {
    _selectedTicks.clear();
    _singleSelection = false;
    _dragAnchorTick = null;
    _movePreview = {};
    _marqueeActive = false;
    _marqueeStart = null;
    _marqueeCurrent = null;
  }

  double _tickAt(double x) {
    final s = widget.snap.value;
    final ppu = s[1];
    final scrollX = s[2];
    final kbW = s[5];
    if (ppu <= 0) {
      return 0.0;
    }
    final px = x - (kbW - AutomationPainter.comboWidth);
    final tick = (px + scrollX) / ppu;
    return tick < 0.0 ? 0.0 : tick;
  }

  /// 与 PR 共用量化档位：`tickAtX` 需要画布内容坐标（AM 的 combo 列宽即键盘宽）。
  double _snapTickAt(double localX) =>
      rust.tickAtX(localX + AutomationPainter.comboWidth);

  double _xOfTick(double tick) {
    final s = widget.snap.value;
    return tick * s[1] - s[2] + (s[5] - AutomationPainter.comboWidth);
  }

  double _yOfValue(double value) =>
      widget.canvasHeight * (1.0 - value.clamp(0.0, 1.0));

  double _valueAtY(double y) =>
      (1.0 - (y / widget.canvasHeight)).clamp(0.0, 1.0);

  double _velocityAt(double y) {
    final v = 1.0 + 126.0 * (1.0 - (y / widget.canvasHeight).clamp(0.0, 1.0));
    return v.clamp(1.0, 127.0);
  }

  /// 命中锚点（屏幕欧氏距离 16px 内）。
  double? _hitAnchor(AutomationLaneData lane, Offset p) {
    double? best;
    var bestDist = 16.0 * 16.0;
    for (final q in lane.points) {
      final dx = _xOfTick(q.dx) - p.dx;
      final dy = _yOfValue(q.dy) - p.dy;
      final dist = dx * dx + dy * dy;
      if (dist < bestDist) {
        bestDist = dist;
        best = q.dx;
      }
    }
    return best;
  }

  int _velocityLowerBound(double tick) {
    var lo = 0;
    var hi = widget.velocity.length;
    while (lo < hi) {
      final mid = (lo + hi) >> 1;
      if (widget.velocity[mid].$1 < tick) {
        lo = mid + 1;
      } else {
        hi = mid;
      }
    }
    return lo;
  }

  /// 把一个线段扫过的力度条写入预览。正式修改由 Rust 在松手时按原音符重算。
  void _hitVelocitySegment((double, double) a, (double, double) b) {
    final s = widget.snap.value;
    final ppu = s[1];
    if (ppu <= 0 || widget.activeTrack < 0) {
      return;
    }
    final hit = 2.0 / ppu;
    final lo = (a.$1 < b.$1 ? a.$1 : b.$1) - hit;
    final hi = (a.$1 > b.$1 ? a.$1 : b.$1) + hit;
    final denom = b.$1 - a.$1;
    for (var i = _velocityLowerBound(lo); i < widget.velocity.length; i++) {
      final (tick, len, _, track) = widget.velocity[i];
      if (tick > hi) {
        break;
      }
      if (tick < lo || track != widget.activeTrack) {
        continue;
      }
      final f = denom.abs() < 1e-9 ? 0.0 : (tick - a.$1) / denom;
      final velocity = (a.$2 + (b.$2 - a.$2) * f).clamp(1.0, 127.0);
      _velocityTouched[(tick.round(), track)] = (
        tick,
        len,
        (velocity - 1.0) / 126.0,
        track,
      );
    }
    _velocityPreview.value = _velocityTouched.values.toList(growable: false);
  }

  void _setVelocityStatus((double, double) p) {
    widget.status.value = EditStatus(
      position: widget.formatTick(p.$1),
      primary: p.$2.round().toString(),
    );
  }

  bool _startVelocityStroke(Offset p) {
    if (!widget.velocityDrawEnabled || _laneIdx != null) {
      return false;
    }
    final point = (_tickAt(p.dx), _velocityAt(p.dy));
    _velocityStrokeActive = true;
    _velocityPoints
      ..clear()
      ..add(point);
    _velocityTouched.clear();
    _hitVelocitySegment(point, point);
    _setVelocityStatus(point);
    return true;
  }

  void _handleVelocityTap(Offset p) {
    if (!widget.velocityDrawEnabled || _laneIdx != null) {
      return;
    }
    final point = (_tickAt(p.dx), _velocityAt(p.dy));
    _setVelocityStatus(point);
    widget.onVelocityCommit([point]);
    widget.status.value = null;
  }

  void _handleVelocityUpdate(Offset p) {
    if (!_velocityStrokeActive) {
      return;
    }
    final point = (_tickAt(p.dx), _velocityAt(p.dy));
    final last = _velocityPoints.last;
    _velocityPoints.add(point);
    _hitVelocitySegment(last, point);
    _setVelocityStatus(point);
  }

  void _finishVelocityStroke() {
    if (!_velocityStrokeActive) {
      return;
    }
    final points = List<(double, double)>.unmodifiable(_velocityPoints);
    _clearVelocityStroke();
    widget.status.value = null;
    if (points.isNotEmpty) {
      widget.onVelocityCommit(points);
    }
  }

  /// 单点锚点编辑 / 框选：
  /// - 按在锚点上：选中（未选中则替换选择），拖动可改 tick+value；
  /// - 按在空白：拉时间框选（全高），多选拖动只改 tick。
  void _handlePanStart(DragStartDetails d) {
    if (_laneIdx == null) {
      _startVelocityStroke(d.localPosition);
      return;
    }
    final laneIdx = _laneIdx;
    if (laneIdx == null || laneIdx >= widget.lanes.length) {
      return;
    }
    final s = widget.snap.value;
    if (s[1] <= 0) {
      return;
    }
    final lane = widget.lanes[laneIdx];
    widget.status.value = null;
    final hit = _hitAnchor(lane, d.localPosition);
    if (hit != null) {
      final hitTick = hit.round();
      if (!_selectedTicks.contains(hitTick)) {
        _selectedTicks
          ..clear()
          ..add(hitTick);
      }
      _singleSelection = _selectedTicks.length == 1;
      _dragAnchorTick = hit;
      _dragStartSnapTick = _snapTickAt(d.localPosition.dx);
      _dragStartValue = _valueAtY(d.localPosition.dy);
      _movePreview = {};
      setState(() {});
      return;
    }
    _selectedTicks.clear();
    _singleSelection = false;
    _dragAnchorTick = null;
    _movePreview = {};
    _marqueeActive = true;
    _marqueeStart = d.localPosition;
    _marqueeCurrent = d.localPosition;
    setState(() {});
  }

  void _handlePanUpdate(DragUpdateDetails d) {
    if (_velocityStrokeActive) {
      _handleVelocityUpdate(d.localPosition);
      return;
    }
    if (_marqueeActive) {
      setState(() => _marqueeCurrent = d.localPosition);
      return;
    }
    if (_dragAnchorTick == null ||
        _dragStartSnapTick == null ||
        _dragStartValue == null) {
      return;
    }
    final laneIdx = _laneIdx;
    if (laneIdx == null || laneIdx >= widget.lanes.length) {
      return;
    }
    final lane = widget.lanes[laneIdx];
    final dt = _snapTickAt(d.localPosition.dx) - _dragStartSnapTick!;
    final dv = _valueAtY(d.localPosition.dy) - _dragStartValue!;
    // 批量移动保持统一位移：最左锚点碰到 0 就整体停住，读数与实际落点一致。
    final minSel = _selectedTicks.isEmpty ? 0 : _selectedTicks.reduce(math.min);
    final effectiveDt = math.max(dt, -minSel.toDouble());
    final preview = <int, (int, double)>{};
    for (final p in lane.points) {
      final oldTick = p.dx.round();
      if (!_selectedTicks.contains(oldTick)) {
        continue;
      }
      final newTick = math.max(0, (p.dx + effectiveDt).round());
      final newValue = _singleSelection ? (p.dy + dv).clamp(0.0, 1.0) : p.dy;
      preview[oldTick] = (newTick, newValue);
    }
    setState(() => _movePreview = preview);
    if (_singleSelection) {
      final moved = preview[_dragAnchorTick!.round()];
      if (moved != null) {
        widget.status.value = EditStatus(
          position: widget.formatTick(moved.$1.toDouble()),
          primary: lane.formatValue(moved.$2),
        );
      }
    } else {
      final dti = effectiveDt.round();
      widget.status.value = EditStatus(
        hidePosition: true,
        primary: '${dti >= 0 ? '+' : ''}$dti',
        secondary: '×${_selectedTicks.length}',
      );
    }
  }

  void _handlePanEnd() {
    if (_velocityStrokeActive) {
      _finishVelocityStroke();
      return;
    }
    if (_marqueeActive) {
      _finishMarquee();
      return;
    }
    final laneIdx = _laneIdx;
    final moves = <(int, int, double)>[];
    if (laneIdx != null) {
      for (final p in widget.lanes[laneIdx].points) {
        final oldTick = p.dx.round();
        final moved = _movePreview[oldTick];
        if (moved == null) {
          continue;
        }
        if (moved.$1 == oldTick && (moved.$2 - p.dy).abs() < 1e-6) {
          continue;
        }
        moves.add((oldTick, moved.$1, moved.$2));
      }
    }
    final oldSelected = Set<int>.from(_selectedTicks);
    final wasSingle = _singleSelection;
    _dragAnchorTick = null;
    _dragStartSnapTick = null;
    _dragStartValue = null;
    _movePreview = {};
    widget.status.value = null;
    setState(() {});
    if (moves.isEmpty) {
      return;
    }
    final ok = rust.moveAutomationAnchors(
      widget.activeTrack,
      laneIdx! + 1,
      moves,
    );
    if (ok != 1) {
      _selectedTicks
        ..clear()
        ..addAll(oldSelected);
      _singleSelection = wasSingle;
      setState(() {});
      return;
    }
    _selectedTicks
      ..clear()
      ..addAll(moves.map((m) => m.$2));
    _singleSelection = wasSingle;
    // 立即回读：旧实现延迟 250ms 会让锚点先弹回旧位置再跳回。
    widget.onReload();
    setState(() {});
  }

  void _finishMarquee() {
    final start = _marqueeStart;
    final end = _marqueeCurrent;
    final laneIdx = _laneIdx;
    _marqueeActive = false;
    _marqueeStart = null;
    _marqueeCurrent = null;
    _selectedTicks.clear();
    _singleSelection = false;
    if (start != null &&
        end != null &&
        (end.dx - start.dx).abs() >= 3 &&
        laneIdx != null) {
      final lo = math.min(start.dx, end.dx);
      final hi = math.max(start.dx, end.dx);
      for (final p in widget.lanes[laneIdx].points) {
        final x = _xOfTick(p.dx);
        if (x >= lo && x <= hi) {
          _selectedTicks.add(p.dx.round());
        }
      }
    }
    setState(() {});
  }

  void _handleLaneTap(Offset p) {
    final laneIdx = _laneIdx;
    if (laneIdx == null || laneIdx >= widget.lanes.length) {
      return;
    }
    final hit = _hitAnchor(widget.lanes[laneIdx], p);
    _selectedTicks.clear();
    _singleSelection = false;
    if (hit != null) {
      _selectedTicks.add(hit.round());
      _singleSelection = true;
    }
    widget.status.value = null;
    setState(() {});
  }

  void _handlePanCancel() {
    if (_velocityStrokeActive) {
      _clearVelocityStroke();
    }
    if (_marqueeActive) {
      _selectedTicks.clear();
      _singleSelection = false;
    }
    _marqueeActive = false;
    _marqueeStart = null;
    _marqueeCurrent = null;
    _dragAnchorTick = null;
    _dragStartSnapTick = null;
    _dragStartValue = null;
    _movePreview = {};
    widget.status.value = null;
    setState(() {});
  }

  Color get _trackColor {
    if (widget.trackColors.isEmpty) {
      return YinheTheme.accent;
    }
    final i = widget.activeTrack.clamp(0, widget.trackColors.length - 1);
    return widget.trackColors[i];
  }

  /// 与「插入自动化」菜单一致的候选 target（kind, number）。
  static const List<(int, int)> _amTargets = <(int, int)>[
    (1, 0), // Pitch Bend
    (2, 7),
    (2, 10),
    (2, 11),
    (2, 64),
    (2, 71),
    (2, 72),
    (2, 73),
    (2, 74),
    (3, 0),
    (3, 1),
    (3, 2),
  ];

  String _laneLabel(AutomationLaneData lane) {
    if (lane.kind == 1 && lane.number >= 0) {
      return MidiNames.instance.ccLabel(lane.number);
    }
    if (lane.kind == 2 && lane.number >= 0) {
      return MidiNames.instance.rpnLabel(lane.number);
    }
    return lane.name;
  }

  /// 规范化 lane 对应的插入菜单文案；非 AM 支持项返回 null。
  String? _canonicalLabel(AutomationLaneData lane) {
    for (final (kind, number) in _amTargets) {
      if (_matchesTarget(lane, kind, number)) {
        return _targetLabel(kind, number);
      }
    }
    return null;
  }

  String _targetLabel(int kind, int number) {
    if (kind == 1) {
      return 'Pitch Bend';
    }
    if (kind == 2) {
      return MidiNames.instance.ccLabel(number);
    }
    return MidiNames.instance.rpnLabel(number);
  }

  @override
  Widget build(BuildContext context) {
    final lane = _laneIdx == null ? null : widget.lanes[_laneIdx!];
    // 只列「插入自动化」支持的标准目标；旧的任意 CC（CC1/CC91…）不再展示。
    final entries = <(String, int)>[('Velocity', 0)];
    for (final (kind, number) in _amTargets) {
      final idx = widget.lanes.indexWhere(
        (ln) => _matchesTarget(ln, kind, number),
      );
      final label = _targetLabel(kind, number);
      if (idx >= 0) {
        entries.add((label, idx + 1));
      } else {
        entries.add((label, -(kind * 100000 + number + 1)));
      }
    }
    Rect? marquee;
    if (_marqueeActive && _marqueeStart != null && _marqueeCurrent != null) {
      final a = _marqueeStart!;
      final b = _marqueeCurrent!;
      marquee = Rect.fromLTRB(
        math.min(a.dx, b.dx),
        0,
        math.max(a.dx, b.dx),
        widget.canvasHeight,
      );
    }
    return Container(
      height: widget.height,
      decoration: BoxDecoration(
        color: YinheTheme.bg,
        border: Border(top: BorderSide(color: YinheTheme.lineFg)),
      ),
      child: Column(
        children: [
          // 顶部拖拽手柄：向上拖变高、向上拖到极限后 PR 只会被压缩不会覆盖。
          GestureDetector(
            behavior: HitTestBehavior.opaque,
            onVerticalDragUpdate: (d) => widget.onResize(d.delta.dy),
            onDoubleTap: widget.onResetHeight,
            child: SizedBox(
              height: AutomationPanel.handleHeight,
              width: double.infinity,
              child: Center(
                child: Container(
                  width: 36,
                  height: 4,
                  decoration: BoxDecoration(
                    color: YinheTheme.textMuted.withValues(alpha: 0.55),
                    borderRadius: BorderRadius.circular(2),
                  ),
                ),
              ),
            ),
          ),
          Expanded(
            child: Row(
              children: [
                SizedBox(
                  width: AutomationPainter.comboWidth,
                  child: Column(
                    children: [
                      InkWell(
                        onTap: () => widget.onCollapsed(true),
                        child: SizedBox(
                          height: 22,
                          width: double.infinity,
                          child: Icon(
                            Icons.keyboard_arrow_down,
                            size: 16,
                            color: YinheTheme.textMuted,
                          ),
                        ),
                      ),
                      Expanded(
                        child: PopupMenuButton<int>(
                          tooltip: l.automationType,
                          padding: EdgeInsets.zero,
                          itemBuilder: (_) => [
                            for (final (label, value) in entries)
                              PopupMenuItem(value: value, child: Text(label)),
                          ],
                          onSelected: (v) {
                            _clearVelocityStroke();
                            _clearSelection();
                            widget.status.value = null;
                            if (v >= 0) {
                              setState(
                                () => _selectedLane = v == 0 ? null : v - 1,
                              );
                              return;
                            }
                            final code = -v - 1;
                            final kind = code ~/ 100000;
                            final number = code % 100000;
                            if (rust.ensureAutomationLane(
                                  widget.activeTrack,
                                  kind,
                                  number,
                                ) ==
                                1) {
                              _pendingTarget = (kind, number);
                              widget.onReload();
                            }
                          },
                          child: Center(
                            child: Text(
                              lane == null
                                  ? 'Velocity'
                                  : _canonicalLabel(lane) ?? _laneLabel(lane),
                              textAlign: TextAlign.center,
                              maxLines: 2,
                              overflow: TextOverflow.ellipsis,
                              style: TextStyle(
                                color: YinheTheme.textLabel,
                                fontSize: 10,
                              ),
                            ),
                          ),
                        ),
                      ),
                    ],
                  ),
                ),
                Expanded(
                  child: GestureDetector(
                    behavior: HitTestBehavior.opaque,
                    onTapUp: _laneIdx == null
                        ? (widget.velocityDrawEnabled
                              ? (d) => _handleVelocityTap(d.localPosition)
                              : null)
                        : (d) => _handleLaneTap(d.localPosition),
                    onPanStart: _handlePanStart,
                    onPanUpdate: _handlePanUpdate,
                    onPanEnd: (_) => _handlePanEnd(),
                    onPanCancel: _handlePanCancel,
                    child: SizedBox.expand(
                      child: CustomPaint(
                        painter: AutomationPainter(
                          lane: lane,
                          velocity: widget.velocity,
                          trackColors: widget.trackColors,
                          color: _trackColor,
                          snap: widget.snap,
                          grid: widget.grid,
                          selectedTicks: _selectedTicks,
                          movePreview: _movePreview,
                          marquee: marquee,
                          velocityPreview: _velocityPreview,
                        ),
                      ),
                    ),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// 自动化栏收起后的左下角升起按钮。
class AutoExpandButton extends StatelessWidget {
  const AutoExpandButton({super.key, required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return Positioned(
      left: 8,
      bottom: 8,
      child: Material(
        color: YinheTheme.controlBg,
        borderRadius: BorderRadius.circular(6),
        elevation: 1,
        child: InkWell(
          borderRadius: BorderRadius.circular(6),
          onTap: onTap,
          child: Padding(
            padding: const EdgeInsets.all(5),
            child: Icon(
              Icons.keyboard_arrow_up,
              size: 18,
              color: YinheTheme.textSecondary,
            ),
          ),
        ),
      ),
    );
  }
}
