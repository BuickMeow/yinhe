part of 'studio_shell.dart';

// 视口手势模式（Rust `yinhe_viewport_gesture` 的 mode）。
const int _gesturePan = 0;
const int _gesturePrTime = 1;
const int _gesturePrPitch = 2;
const int _gestureArTime = 3;
const int _gestureArLane = 4;

mixin _ShellInput on _ShellBase {
  // ── 触摸 / 手势 ──

  final Set<int> _activePointers = <int>{};
  final Map<int, Offset> _pointerPositions = <int, Offset>{};

  /// 本手势序列出现过的最大手指数（sticky 到下一次第一根手指按下）。
  /// 用于在多指轻点/长按时压掉单指 Tap/LongPress 的副作用。
  int _maxActivePointers = 0;
  Offset? _gestureStart;
  Offset _lastFocal = Offset.zero;

  /// 双指分轴缩放：绝对间距起手 + 逐帧跟手。
  final PinchZoom _pinchZoom = PinchZoom();

  /// 指针数量变化后跳过一帧缩放，避免基准跳变。
  bool _pinchRebase = false;
  bool _gestureEditing = false;
  bool _gestureWasEditing = false;
  bool _gestureZoomed = false;
  bool _gestureMoved = false;

  /// 标尺拖动定位（playhead scrub）状态：手指离开标尺也不断，靠 ticker
  /// 在视口边缘继续平移并重算时间。
  bool _rulerScrubbing = false;
  double _rulerScrubX = 0;

  /// 标尺自动滚动触发区（逻辑像素）：比选框更宽，避免拖到边缘还进不去。
  static const double _rulerScrubZone = 48.0;

  /// 第一根手指按下时立即 begin，保证铅笔/选框没有 100ms 点击延迟。
  /// 后续的 tap / long press / scale 全部交给 Flutter gesture arena。
  @override
  void _handlePointerDown(PointerDownEvent e) {
    final emptyBefore = _activePointers.isEmpty;
    _activePointers.add(e.pointer);
    _pointerPositions[e.pointer] = e.localPosition;
    if (emptyBefore) {
      _maxActivePointers = 1;
    } else if (_activePointers.length > _maxActivePointers) {
      _maxActivePointers = _activePointers.length;
    }
    if (!emptyBefore) {
      // 第二根手指：取消编辑，交给 Flutter 双指手势；下一帧只平移不缩放。
      rust.editCancel();
      _gestureEditing = false;
      _gestureZoomed = false;
      _pinchRebase = true;
      _clearLongPressUi();
      return;
    }

    _gestureStart = e.localPosition;
    _lastFocal = e.localPosition;
    _pinchRebase = false;
    _pinchZoom.rebase(_pointerPositions.values.toList(growable: false));
    _gestureEditing = false;
    _gestureWasEditing = false;
    _gestureZoomed = false;
    _gestureMoved = false;
    rust.stopFling();
    _gestureEditing =
        rust.editBegin(e.localPosition.dx, e.localPosition.dy) == 1;
    _gestureWasEditing = _gestureEditing;
    // 单一交互：任何单指按下都开始长按计时（圆环 200ms 后才显示）。
    _beginLongPressWatch(e.localPosition);
  }

  @override
  void _handlePointerMove(PointerMoveEvent e) {
    if (_pointerPositions.containsKey(e.pointer)) {
      _pointerPositions[e.pointer] = e.localPosition;
    }
  }

  @override
  void _handlePointerUp(PointerUpEvent e) {
    _activePointers.remove(e.pointer);
    _pointerPositions.remove(e.pointer);
    if (_activePointers.isNotEmpty) {
      // 指针数量变化：下一帧只平移不缩放，重新取基准。
      _pinchRebase = true;
      return;
    }
    _lastFocal = e.localPosition;
    _pinchRebase = false;
    _pinchZoom.rebase(_pointerPositions.values.toList(growable: false));
    // PointerUp 带精确落点；编辑在最后一根手指离开时立即提交，避免只靠
    // ScaleEnd 的“最后一次 move”导致笔尖/音符结束位置偏几像素。
    if (_gestureEditing) {
      _finishEditGesture(e.localPosition, moved: _gestureMoved || _lpMoved);
    }
  }

  @override
  void _handlePointerCancel(PointerCancelEvent e) {
    _activePointers.remove(e.pointer);
    _pointerPositions.remove(e.pointer);
    if (_activePointers.isNotEmpty) {
      _pinchRebase = true;
      return;
    }
    rust.editCancel();
    _gestureEditing = false;
    _gestureWasEditing = false;
    _gestureZoomed = false;
    _gestureMoved = false;
    _pinchRebase = false;
    _pinchZoom.rebase(_pointerPositions.values.toList(growable: false));
    _clearLongPressUi();
  }

  @override
  void _handleTapUp(TapUpDetails d) {
    // 多指轻点由 MultiFingerGestureRecognizer 处理，不能再走单指画布点击
    // （双指 Tap 会在第一根手指抬起时触发一次 onTapUp）。
    if (_maxActivePointers >= 2) {
      _gestureWasEditing = false;
      _clearLongPressUi();
      return;
    }
    final p = d.localPosition;
    if (_gestureEditing) {
      _finishEditGesture(p, moved: _gestureMoved);
    } else if (rust.clearSelection() == 1) {
      // 有选框：这一次点击只负责清除选框，不落音/删除/切轨。
    } else if (_viewMode == kViewPianoroll) {
      // 没有选框才允许写音符：点空白落音、点音符删除。
      final deleted = _noteRectAt(p);
      if (rust.tapNote(p.dx, p.dy) == 1) {
        _refreshDocumentMeta();
        if (deleted != null) {
          _startNoteFade(deleted);
        }
      }
    } else {
      _handleCanvasTap(p);
    }
    _gestureWasEditing = false;
    _clearLongPressUi();
  }

  // ── 双指 / 单指平移缩放 ──

  @override
  void _handleScaleStart(ScaleStartDetails d) {
    rust.stopFling();
    _clearLongPressUi();
    _lastFocal = d.localFocalPoint;
    _gestureZoomed = false;
    _gestureMoved = false;
    final downPos = _gestureStart;
    _gestureStart = d.localFocalPoint;
    _pinchRebase = false;
    _pinchZoom.rebase(_pointerPositions.values.toList(growable: false));
    _gestureEditing = false;
    if (d.pointerCount == 1) {
      // 用按下点判定（scale 起手点已经被第一次移动带偏）。
      // 优先级：选框锚点 → 选框内部 → 音符右端改长度 → 音符移动。
      final hit = downPos ?? d.localFocalPoint;
      _gestureEditing =
          rust.beginSelectionHandle(hit.dx, hit.dy) == 1 ||
          rust.beginSelectionGate(hit.dx, hit.dy) == 1 ||
          rust.beginSelectionMove(hit.dx, hit.dy) == 1 ||
          rust.beginNoteResize(hit.dx, hit.dy) == 1 ||
          rust.beginNoteMove(hit.dx, hit.dy) == 1;
      _gestureWasEditing = _gestureEditing;
    }
  }

  @override
  void _handleScaleUpdate(ScaleUpdateDetails d) {
    final focal = d.localFocalPoint;
    if (d.pointerCount >= 2) {
      if (_gestureEditing) {
        rust.editCancel();
        _gestureEditing = false;
      }
      final pan = focal - _lastFocal;
      _lastFocal = focal;
      if (_pinchRebase) {
        _pinchRebase = false;
        _pinchZoom.rebase(_pointerPositions.values.toList(growable: false));
        return;
      }
      final (hDelta, vDelta) = _pinchZoom.update(
        _pointerPositions.values.toList(growable: false),
      );
      final timeMode = _viewMode == kViewPianoroll
          ? _gesturePrTime
          : _gestureArTime;
      final crossMode = _viewMode == kViewPianoroll
          ? _gesturePrPitch
          : _gestureArLane;
      // 平移 + 左右缩放；纵向/斜向再叠加副轴缩放。
      rust.viewportGesture(
        timeMode,
        pan.dx,
        pan.dy,
        hDelta,
        focal.dx,
        focal.dy,
      );
      if ((vDelta - 1.0).abs() > 1e-6) {
        rust.viewportGesture(crossMode, 0.0, 0.0, vDelta, focal.dx, focal.dy);
      }
      if ((hDelta - 1.0).abs() > 1e-6 || (vDelta - 1.0).abs() > 1e-6) {
        _gestureZoomed = true;
      }
      return;
    }

    final pan = focal - _lastFocal;
    if (_gestureEditing) {
      final active = rust.editUpdate(focal.dx, focal.dy) == 1;
      if (!active) {
        // 已选框长按待定被拖动取消：从这次位移开始转为平移。
        _gestureEditing = false;
        rust.viewportGesture(
          _gesturePan,
          pan.dx,
          pan.dy,
          1.0,
          focal.dx,
          focal.dy,
        );
      }
    } else {
      rust.viewportGesture(
        _gesturePan,
        pan.dx,
        pan.dy,
        1.0,
        focal.dx,
        focal.dy,
      );
    }
    if (_gestureStart != null && (focal - _gestureStart!).distance > 3.0) {
      _gestureMoved = true;
    }
    _lastFocal = focal;
  }

  @override
  void _handleScaleEnd(ScaleEndDetails d) {
    final wasEditing = _gestureEditing || _gestureWasEditing;
    if (_gestureEditing) {
      _finishEditGesture(_lastFocal, moved: _gestureMoved || _lpMoved);
    } else if (!_gestureZoomed && !wasEditing) {
      final v = d.velocity.pixelsPerSecond;
      if (v.distance > 0) {
        rust.startFling(v.dx, v.dy);
      }
    }
    _gestureEditing = false;
    _gestureWasEditing = false;
    _gestureZoomed = false;
    _gestureMoved = false;
    _pinchRebase = false;
    _pinchZoom.rebase(_pointerPositions.values.toList(growable: false));
    _clearLongPressUi();
  }

  // ── 长按（Flutter LongPressGestureRecognizer）──

  @override
  void _handleLongPressStart(LongPressStartDetails d) {
    // 多指按住不应触发 AR 拖轨/选区菜单等单指长按行为。
    if (_maxActivePointers >= 2) {
      _clearLongPressUi();
      return;
    }
    final p = d.localPosition;
    if (_viewMode == kViewArrangement) {
      final idx = _rowAt(p);
      if (idx != null && idx < _tracks.value.length) {
        // AR 音轨面板：长按拖动排序。
        _clearLongPressUi();
        _trackDragMoved = false;
        setState(() {
          _dragFrom.value = idx;
          _dragTo.value = idx;
          _dragging = true;
        });
        return;
      }
      if (idx != null && idx >= _tracks.value.length) {
        // 列表末尾是“添加音轨”占位行，由普通 tap 处理。
        _clearLongPressUi();
        return;
      }
    }
    // 长按已有选框：优先进入选框编辑（拖 = 移动、松手 = 选区菜单）。
    if (rust.selectionBoxAt(p.dx, p.dy) == 1) {
      _gestureEditing = rust.beginMarquee(p.dx, p.dy) == 1;
      if (_gestureEditing) {
        rust.editLongPress();
        _markLongPressRecognized();
        _lpMoved = false;
      }
      return;
    }
    // PR/AR 内容：长按后拖动 = 新建选框；不拖 = 松开时弹浮动菜单。
    _lpGlobalPos = d.globalPosition;
    _lpMarqueeArmed = true;
    _markLongPressRecognized();
    _lpMoved = false;
    _startMarqueeHint(p);
  }

  void _startMarqueeHint(Offset p) {
    _marqueeHintPos.value = p;
    _marqueeHint.value = 0;
    _marqueeHintStart = DateTime.now();
    _marqueeHintFading = false;
    _marqueeHintFadeStart = null;
  }

  void _tickMarqueeHint() {
    if (_marqueeHintFading) {
      final start = _marqueeHintFadeStart;
      if (start == null) {
        return;
      }
      final ms = DateTime.now().difference(start).inMilliseconds;
      final t = 1.0 + (ms / 120).clamp(0.0, 1.0);
      _marqueeHint.value = t;
      if (t >= 2) {
        _stopMarqueeHint();
      }
      return;
    }
    final start = _marqueeHintStart;
    if (start == null) {
      return;
    }
    final ms = DateTime.now().difference(start).inMilliseconds;
    // 先放大，随后保持到松手；开始拖框时才淡出。
    _marqueeHint.value = (ms / 180).clamp(0.0, 1.0);
  }

  /// 开始拖框：小方框淡出，真实选框接手。
  void _fadeOutMarqueeHint() {
    if (_marqueeHintPos.value == null) {
      return;
    }
    _marqueeHintFading = true;
    _marqueeHintFadeStart = DateTime.now();
  }

  void _stopMarqueeHint() {
    _marqueeHintStart = null;
    _marqueeHintFading = false;
    _marqueeHintFadeStart = null;
    _marqueeHintPos.value = null;
    _marqueeHint.value = 0;
  }

  /// 读取命中音符的像素矩形；无命中返回 null。
  (Rect, int)? _noteRectAt(Offset p) {
    final ptr = rust.noteRectAt(p.dx, p.dy);
    if (ptr == ffi.nullptr) {
      return null;
    }
    try {
      final decoded = jsonDecode(ptr.toDartString());
      if (decoded is Map<String, dynamic>) {
        final x0 = (decoded['x0'] as num?)?.toDouble();
        final y0 = (decoded['y0'] as num?)?.toDouble();
        final x1 = (decoded['x1'] as num?)?.toDouble();
        final y1 = (decoded['y1'] as num?)?.toDouble();
        final track = (decoded['track'] as num?)?.toInt();
        if (x0 == null ||
            y0 == null ||
            x1 == null ||
            y1 == null ||
            track == null) {
          return null;
        }
        return (Rect.fromLTRB(x0, y0, x1, y1), track);
      }
    } catch (_) {
      return null;
    } finally {
      rust.freeString(ptr);
    }
    return null;
  }

  /// 开始一个删除残影：选中色保持一小段后快速淡出。
  void _startNoteFade((Rect, int)? hit) {
    if (hit == null) {
      return;
    }
    final (rect, track) = hit;
    final rows = _tracks.value;
    final base = track >= 0 && track < rows.length
        ? rows[track].color
        : YinheTheme.accent;
    final target = Theme.of(context).brightness == Brightness.dark
        ? Colors.white
        : Colors.black;
    _noteFades.add(
      NoteDeleteFade(rect, Color.lerp(base, target, 0.7)!, DateTime.now()),
    );
    _noteFadeTick.value++;
  }

  void _tickNoteFades() {
    if (_noteFades.isEmpty) {
      return;
    }
    final now = DateTime.now();
    _noteFades.removeWhere(
      (f) => now.difference(f.start) >= NoteDeleteFade.duration,
    );
    _noteFadeTick.value++;
  }

  /// 读取 Rust 长按上下文（插入/粘贴菜单用）。
  LongPressContext? _readLongPressCtx() {
    final ptr = rust.longPressContextJson();
    if (ptr == ffi.nullptr) {
      return null;
    }
    try {
      final decoded = jsonDecode(ptr.toDartString());
      if (decoded is Map<String, dynamic>) {
        return LongPressContext.fromJson(decoded);
      }
    } catch (_) {
      return null;
    } finally {
      rust.freeString(ptr);
    }
    return null;
  }

  // ── 时间标尺长按：插入落点微调 ──

  /// 选框锚点出现：新选区从 0 放大到正式大小。
  void _tickHandlesAppear() {
    final o = _overlay.value;
    // 移动选框时锚点不显示，但让缩放动画继续跑完；松手后不再重新弹一次。
    final show = _tool == 0 && o.box == null && o.sel.length == 1;
    if (!show) {
      if (_handlesShown) {
        _handlesShown = false;
        _handlesStart = null;
        _handlesScale.value = 0;
      }
      return;
    }
    if (!_handlesShown) {
      _handlesShown = true;
      _handlesStart = DateTime.now();
      _handlesScale.value = 0;
    }
    final start = _handlesStart;
    if (start == null) {
      return;
    }
    final t = (DateTime.now().difference(start).inMilliseconds / 180).clamp(
      0.0,
      1.0,
    );
    _handlesScale.value = t >= 1 ? 1.0 : Curves.easeOutBack.transform(t);
    if (t >= 1) {
      _handlesStart = null;
    }
  }

  /// 未进入长按插入：水平拖动时间标尺切换播放位置，顶部栏蓝字显示目标时间。
  @override
  void _handleRulerScrub(Offset local) {
    _rulerScrubbing = true;
    _rulerScrubX = local.dx;
    _applyRulerScrub();
  }

  @override
  void _handleRulerScrubEnd() {
    _rulerScrubbing = false;
    _clearInsertStatus();
  }

  void _applyRulerScrub() {
    final tick = rust.tickAtX(_rulerScrubX);
    rust.jumpToTick(tick);
    _setInsertStatus(tick);
  }

  /// 每帧推进：手指进入视口左右边缘区时继续平移，并持续更新播放位置。
  void _tickRulerScrub() {
    if (!_rulerScrubbing) {
      return;
    }
    final width = MediaQuery.sizeOf(context).width;
    final s = _snap.value;
    final panel = s.length > SnapField.leftPanelW
        ? s[SnapField.leftPanelW]
        : 0.0;
    final left = panel + _rulerScrubZone;
    var vx = 0.0;
    if (_rulerScrubX < left) {
      vx = ((left - _rulerScrubX) * 0.9).clamp(0.0, 30.0);
    } else if (_rulerScrubX > width - _rulerScrubZone) {
      vx = -((_rulerScrubX - (width - _rulerScrubZone)) * 0.9).clamp(0.0, 30.0);
    }
    if (vx == 0.0) {
      return;
    }
    rust.panBy(vx, 0);
    _applyRulerScrub();
  }

  @override
  void _handleRulerInsertStart(Offset local, Offset global) {
    rust.editCancel();
    _lpToken++;
    _clearLongPressUi();
    if (rust.beginRulerInsert(local.dx, local.dy) != 1) {
      return;
    }
    rust.editLongPress();
    _lpPos = local;
    _lpGlobalPos = global;
  }

  @override
  void _handleRulerInsertMove(Offset local) {
    rust.editUpdate(local.dx, local.dy);
    _lpPos = local;
  }

  @override
  void _handleRulerInsertEnd(Offset local, Offset global) {
    final ctx = _readLongPressCtx();
    rust.editEnd(local.dx, local.dy);
    _readDocState();
    _clearLongPressUi();
    _showPasteMenuAt(local, global, ctx, fromRuler: true);
  }

  void _markLongPressRecognized() {
    _lpStartTime = null;
    _lpProgress.value = 0;
    // 保留 _lpPos：空白处长按松手时用它定位粘贴菜单。
  }

  @override
  void _handleLongPressMove(LongPressMoveUpdateDetails d) {
    final p = d.localPosition;
    if (_dragging) {
      // Master（Conductor）固定在 0 号位，拖动不改目标。
      if (_dragFrom.value <= 0) {
        return;
      }
      final idx = _rowAt(p);
      if (idx == null) {
        return;
      }
      if (idx != _dragFrom.value) {
        _trackDragMoved = true;
      }
      if (idx != _dragTo.value) {
        setState(() => _dragTo.value = idx);
      }
      return;
    }
    if (_lpMarqueeArmed) {
      if (_lpPos != null && (p - _lpPos!).distance > 12.0) {
        _lpMarqueeArmed = false;
        _startMarquee(_lpPos!);
        rust.editUpdate(p.dx, p.dy);
      }
      return;
    }
    if (!_gestureEditing) {
      return;
    }
    final active = rust.editUpdate(p.dx, p.dy) == 1;
    if (!active) {
      _gestureEditing = false;
      return;
    }
    if (_lpPos != null && (p - _lpPos!).distance > 12.0) {
      _lpMoved = true;
    }
    if (_gestureStart != null && (p - _gestureStart!).distance > 3.0) {
      _gestureMoved = true;
    }
  }

  @override
  void _handleLongPressEnd(LongPressEndDetails d) {
    if (_dragging) {
      _finishTrackDrag();
      return;
    }
    if (_gestureEditing) {
      // 选框提交；选框编辑未移动时由 _finishEditGesture 弹选区菜单。
      _finishEditGesture(d.localPosition, moved: _lpMoved);
      return;
    }
    if (_lpMarqueeArmed) {
      _lpMarqueeArmed = false;
      final pos = _lpPos;
      final global = _lpGlobalPos;
      _clearLongPressUi();
      if (pos != null && global != null) {
        _showFloatingLongPressMenu(pos, global);
      }
      return;
    }
    _clearLongPressUi();
  }

  /// 长按拖动开始：临时切到选框工具，从按下点拉框。
  void _startMarquee(Offset press) {
    _fadeOutMarqueeHint();
    rust.clearSelection();
    _gestureEditing = rust.beginMarquee(press.dx, press.dy) == 1;
  }

  /// 长按未拖动：在手指位置弹浮动菜单（粘贴/全选）。
  void _showFloatingLongPressMenu(Offset local, Offset global) {
    final ptr = rust.longPressContextAt(local.dx, local.dy);
    LongPressContext? ctx;
    if (ptr != ffi.nullptr) {
      try {
        final decoded = jsonDecode(ptr.toDartString());
        if (decoded is Map<String, dynamic>) {
          ctx = LongPressContext.fromJson(decoded);
        }
      } catch (_) {
        ctx = null;
      } finally {
        rust.freeString(ptr);
      }
    }
    _showPasteMenuAt(local, global, ctx);
  }

  void _finishTrackDrag() {
    final from = _dragFrom.value;
    final to = _dragTo.value;
    setState(() {
      _dragFrom.value = -1;
      _dragTo.value = -1;
      _dragging = false;
    });
    if (from < 0) {
      return;
    }
    if (from == 0) {
      // Master 轨只弹信息菜单，不参与排序。
      _showTrackMenu(0);
      return;
    }
    if (to < 0 || (from == to && !_trackDragMoved)) {
      // 长按未拖动：弹出音轨信息菜单。
      _showTrackMenu(from);
      return;
    }
    if (from == to) {
      // 移动过又放回原位：不排序、不弹菜单。
      return;
    }
    if (rust.moveTrack(from, to) == 1) {
      _refreshDocumentMeta();
    }
  }

  /// 统一的编辑收尾：提交 Rust 编辑，再按长按时的状态弹选区/粘贴菜单。
  void _finishEditGesture(Offset p, {required bool moved}) {
    final state = rust.selectionState();
    LongPressContext? ctx;
    if (state == 3) {
      final ptr = rust.longPressContextJson();
      if (ptr != ffi.nullptr) {
        try {
          final decoded = jsonDecode(ptr.toDartString());
          if (decoded is Map<String, dynamic>) {
            ctx = LongPressContext.fromJson(decoded);
          }
        } catch (_) {
          ctx = null;
        } finally {
          rust.freeString(ptr);
        }
      }
    }
    final pos = _lpPos ?? p;
    rust.editEnd(p.dx, p.dy);
    _gestureEditing = false;
    // 编辑提交后立刻刷新撤销/重做图标，不等下一次 66ms 状态轮询。
    _readDocState();
    _clearLongPressUi();
    if (state == 2 && !moved) {
      _showSelectionMenu();
    } else if (state == 3) {
      _showPasteMenu(pos, ctx);
    }
  }

  /// 按下：开始长按计时（圆环转满后由 Flutter long press 接手）。
  void _beginLongPressWatch(Offset p) {
    _lpPos = p;
    _lpPosNotifier.value = p;
    _lpStartTime = DateTime.now();
    _lpMoved = false;
    _lpProgress.value = 0;
    _moveDashed.value = false;
  }

  /// 清理长按 UI 状态（不改 Rust 拖拽状态）。
  void _clearLongPressUi() {
    _stopMarqueeHint();
    _lpStartTime = null;
    _lpPos = null;
    _lpGlobalPos = null;
    _lpPosNotifier.value = null;
    _lpProgress.value = 0;
    _lpMoved = false;
    _lpMarqueeArmed = false;
    _lpPreviewTick = null;
  }

  /// 全量清空 Flutter 手势状态，并让 Rust 取消编辑/惯性。
  @override
  void _resetInteraction() {
    rust.resetInteraction();
    _activePointers.clear();
    _pointerPositions.clear();
    _pinchRebase = false;
    _pinchZoom.rebase(_pointerPositions.values.toList(growable: false));
    _gestureEditing = false;
    _gestureWasEditing = false;
    _gestureZoomed = false;
    _gestureMoved = false;
    _clearLongPressUi();
    _clearInsertStatus();
  }

  /// 多指手势：双指轻点撤销。
  @override
  void _undoFromGesture() => _runHistoryGesture(_undo);

  /// 多指手势：三指轻点/右滑重做。
  @override
  void _redoFromGesture() => _runHistoryGesture(_redo);

  /// 手势触发撤销/重做前先丢弃未提交编辑与惯性，避免和 Rust 拖拽状态打架。
  /// 不清 `_activePointers`：动作在最后一根手指抬起后触发，指针集合由 up 自己维护。
  void _runHistoryGesture(void Function() action) {
    if (!mounted) {
      return;
    }
    rust.editCancel();
    rust.stopFling();
    _gestureEditing = false;
    _gestureWasEditing = false;
    _gestureZoomed = false;
    _gestureMoved = false;
    _pinchRebase = false;
    _pinchZoom.rebase(_pointerPositions.values.toList(growable: false));
    _clearLongPressUi();
    action();
    _readDocState();
  }

  /// 鼠标滚轮 / 触控板滚动：Ctrl（或 Cmd）+ 滚轮缩放（键盘区缩放音高轴，
  /// 其余缩放时间轴），普通滚轮平移时间轴。
  @override
  void _handlePointerSignal(PointerSignalEvent e) {
    if (e is! PointerScrollEvent) {
      return;
    }
    final ctrl =
        HardwareKeyboard.instance.isControlPressed ||
        HardwareKeyboard.instance.isMetaPressed;
    rust.scrollAt(
      e.localPosition.dx,
      e.localPosition.dy,
      e.scrollDelta.dx,
      e.scrollDelta.dy,
      ctrl ? 1 : 0,
    );
  }

  int? _rowAt(Offset p) {
    final s = _snap.value;
    final panel = s[SnapField.leftPanelW];
    final laneH = s[SnapField.laneHeight];
    final scrollY = s[SnapField.scrollY];
    if (laneH <= 0 || p.dx >= panel) {
      return null;
    }
    final idx = ((p.dy + scrollY) / laneH).floor();
    final rows = _tracks.value;
    // idx == rows.length 是列表末尾的"添加音轨"占位行。
    if (idx < 0 || idx > rows.length) {
      return null;
    }
    return idx;
  }

  bool _inMsZone(Offset p) {
    final s = _snap.value;
    final panel = s[SnapField.leftPanelW];
    final laneH = s[SnapField.laneHeight];
    if (laneH < 34) {
      return false;
    }
    final btn = (laneH * 0.44).clamp(18.0, 26.0).toDouble();
    final zoneLeft = panel - 6.0 - btn * 2 - 4.0 - 4.0;
    return p.dx >= zoneLeft;
  }

  /// AR：点击轨道面板。M/S 按钮切换；末行 + 打开添加音轨；其余进入 PR。
  void _handleCanvasTap(Offset p) {
    if (_viewMode != kViewArrangement) {
      return;
    }
    final rows = _tracks.value;
    final s = _snap.value;
    final panel = s[SnapField.leftPanelW];
    final laneH = s[SnapField.laneHeight];
    final scrollY = s[SnapField.scrollY];
    // AR 内容区：双击按 y 找行并进入对应轨道（单击不动，避免误进）。
    if (laneH > 0 && p.dx >= panel) {
      final idx = ((p.dy + scrollY) / laneH).floor();
      if (idx < 0 || idx >= rows.length) {
        return;
      }
      final now = DateTime.now();
      final last = _arrLastTap;
      final lastPos = _arrLastTapPos;
      if (last != null &&
          lastPos != null &&
          now.difference(last).inMilliseconds <= 300 &&
          (lastPos - p).distance <= 30) {
        _arrLastTap = null;
        _arrLastTapPos = null;
        _selectEditTrack(idx);
        _setView(kViewPianoroll);
      } else {
        _arrLastTap = now;
        _arrLastTapPos = p;
        // 单击（不拖动）把播放头跳到对应时间；拖动仍由视口手势处理。
        rust.jumpToTick(rust.tickAtX(p.dx));
      }
      return;
    }
    _arrLastTap = null;
    _arrLastTapPos = null;
    final idx = _rowAt(p);
    if (idx == null) {
      return;
    }
    if (idx == rows.length) {
      _showAddTrackDialog();
      return;
    }
    if (!rows[idx].master && _inMsZone(p)) {
      _toggleMs(p, idx);
      return;
    }
    // 进入 PR：AR 点的那条轨就是当前编辑轨；「全部轨道」开关保持不变。
    _selectEditTrack(idx);
    _setView(kViewPianoroll);
  }

  /// 命中 M/S 按钮：M 在左（警告色），S 在右（危险色）。
  void _toggleMs(Offset p, int idx) {
    final s = _snap.value;
    final laneH = s[SnapField.laneHeight];
    final panelW = s[SnapField.leftPanelW];
    final btn = (laneH * 0.44).clamp(18.0, 26.0).toDouble();
    const gap = 4.0;
    final right = panelW - 6.0;
    final mLeft = right - btn * 2 - gap;
    final sLeft = right - btn;
    if (p.dx >= mLeft && p.dx <= mLeft + btn) {
      if (rust.toggleTrackMute(idx) == 1) {
        _refreshDocumentMeta();
      }
    } else if (p.dx >= sLeft && p.dx <= sLeft + btn) {
      if (rust.toggleTrackSolo(idx) == 1) {
        _refreshDocumentMeta();
      }
    }
  }
}
