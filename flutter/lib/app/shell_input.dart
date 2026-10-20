part of 'studio_shell.dart';

mixin _ShellInput on _ShellBase {
  // ── 触摸 / 手势 ──

  @override
  bool _isRightMouse(PointerEvent e) =>
      e.kind == PointerDeviceKind.mouse &&
      (e.buttons & kSecondaryMouseButton) != 0;

  /// 右键按下：取消长按/编辑拖拽，强制进入平移（手形语义）。
  @override
  void _beginRightPan(PointerDownEvent e) {
    _panPointer = e.pointer;
    _cancelLongPressWatch();
    _sendPanTouch(3, e.localPosition, e.pointer);
  }

  /// 右键平移事件（3=down 4=move 5=up）。
  @override
  void _sendPanTouch(int kind, Offset p, int pointer) {
    rust.touch(kind, p.dx, p.dy, pointer);
    SchedulerBinding.instance.scheduleFrame();
  }

  @override
  void _sendTouch(int kind, Offset p, int pointer) {
    // AR 长按拖动音轨/自动化锚点时无需转发 move（避免误平移），但 up/cancel
    // 必须转发：否则 Rust 触点表永久残留这根手指，下一次单指会被判成双指。
    if (_dragging && kind != 2) {
      return;
    }
    // 只写入事件；纹理与浮层统一在下一帧同步刷新（帧驱动，输入侧已就地 clamp）。
    if (kind == 2) {
      // up 前读取状态与长按上下文（Rust 在 up 时会清理）。
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
      rust.touch(kind, p.dx, p.dy, pointer);
      _finishLongPressWatch(state, ctx);
    } else {
      rust.touch(kind, p.dx, p.dy, pointer);
      if (kind == 0) {
        // 选框内/空白处按下由 Rust 进入 PENDING（500ms 后自行转移动态或粘贴态）。
        if (rust.selectionState() == 1) {
          _beginLongPressWatch(p);
        } else if (_viewMode == kViewArrangement && _rowAt(p) != null) {
          // AR 音轨面板长按：Rust 不进入 PENDING，圆环由 Dart 计时。
          _beginLongPressWatch(p, panel: true);
        }
      } else if (kind == 1) {
        _updateLongPressWatch(p);
      }
    }
    // 触摸期间确保有下一帧（拖拽边缘自动滚动/预览依赖帧驱动）。
    SchedulerBinding.instance.scheduleFrame();
  }

  /// 按下：开始长按计时（圆环转满后进入移动态或粘贴态）。
  ///
  /// 判定由 Rust 的 PENDING 状态给出；500ms 计时也在 Rust 帧循环里推进。
  void _beginLongPressWatch(Offset p, {bool panel = false}) {
    _lpPos = p;
    _lpPosNotifier.value = p;
    _lpStartTime = DateTime.now();
    _lpMoved = false;
    _lpPanelRing = panel;
    _lpProgress.value = 0;
    _moveDashed.value = false;
  }

  void _updateLongPressWatch(Offset p) {
    if (_lpStartTime == null) {
      return;
    }
    if (_lpPanelRing) {
      // 面板长按：超过抖动阈值视为拖动（Flutter 长按手势会自行取消）。
      if (_lpPos != null && (p - _lpPos!).distance > 12) {
        _clearLongPressUi();
      }
      return;
    }
    final state = rust.selectionState();
    if (state == 1) {
      // 长按未完成就移动：取消（避免误拖）。
      if ((p - _lpPos!).distance > 12) {
        _cancelLongPressWatch();
      }
    } else if (state == 2) {
      // 已进入移动态：超过手指抖动阈值才算真的移动（否则松手仍弹复制菜单）。
      if (_lpPos != null && (p - _lpPos!).distance > 12) {
        _lpMoved = true;
      }
    } else if (state == 0) {
      // Rust 已把空白长按转成普通框选：只收掉长按 UI，不取消框选。
      _clearLongPressUi();
    }
  }

  /// 清理长按 UI 状态（不改 Rust 拖拽状态）。
  void _clearLongPressUi() {
    _lpStartTime = null;
    _lpPos = null;
    _lpPosNotifier.value = null;
    _lpProgress.value = 0;
    _lpMoved = false;
    _lpPanelRing = false;
    _lpPreviewTick = null;
  }

  @override
  void _cancelLongPressWatch() {
    if (_lpStartTime == null) {
      return;
    }
    _clearLongPressUi();
    _moveDashed.value = false;
    rust.cancelSelectionPending();
  }

  void _finishLongPressWatch(int state, LongPressContext? ctx) {
    if (_lpStartTime == null) {
      return;
    }
    final moved = _lpMoved;
    final pos = _lpPos;
    _clearLongPressUi();
    // 虚线由 Ticker 依 state 关闭（up 后 state=0）。
    if (state == 2 && !moved) {
      _showSelectionMenu();
    } else if (state == 3) {
      _showPasteMenu(pos, ctx);
    }
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

  /// 触控板捏合：按累计缩放的增量缩放（键盘区 → 音高轴）。
  @override
  void _handlePanZoomUpdate(PointerPanZoomUpdateEvent e) {
    final next = e.scale;
    final last = _panZoomScale;
    _panZoomScale = next;
    if (last <= 0 || next <= 0) {
      return;
    }
    final factor = next / last;
    if ((factor - 1.0).abs() > 0.002) {
      rust.zoomAt(e.localPosition.dx, e.localPosition.dy, factor);
    }
  }

  int? _rowAt(Offset p) {
    final s = _snap.value;
    final panel = s[5];
    final laneH = s[6];
    final scrollY = s[3];
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
    final panel = s[5];
    final laneH = s[6];
    if (laneH < 34) {
      return false;
    }
    final btn = (laneH * 0.44).clamp(18.0, 26.0).toDouble();
    final zoneLeft = panel - 6.0 - btn * 2 - 4.0 - 4.0;
    return p.dx >= zoneLeft;
  }

  /// AR：点击轨道面板。M/S 按钮切换；末行 + 打开添加音轨；其余进入 PR。
  @override
  void _handleCanvasTap(Offset p) {
    if (_viewMode != kViewArrangement) {
      return;
    }
    final idx = _rowAt(p);
    if (idx == null) {
      return;
    }
    final rows = _tracks.value;
    if (idx == rows.length) {
      _showAddTrackDialog();
      return;
    }
    if (!rows[idx].master && _inMsZone(p)) {
      _toggleMs(p, idx);
      return;
    }
    // 进入 PR：AR 点的那条轨就是当前编辑轨；「全部轨道」默认关闭。
    _showAllTracks = false;
    _selectEditTrack(idx);
    _setView(kViewPianoroll);
  }

  /// 命中 M/S 按钮：M 在左（警告色），S 在右（危险色）。
  void _toggleMs(Offset p, int idx) {
    final s = _snap.value;
    final laneH = s[6];
    final panelW = s[5];
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

  /// AR：长按拖动轨道排序。
  @override
  void _handleLongPressStart(Offset p) {
    if (_viewMode != kViewArrangement) {
      return;
    }
    final idx = _rowAt(p);
    if (idx == null || idx >= _tracks.value.length) {
      // 空白处：保留长按 UI，松手时由 _finishLongPressWatch 弹插入菜单。
      return;
    }
    // 长按成立：圆环完成，收起长按 UI（后续由轨道拖动/松手菜单接管）。
    _clearLongPressUi();
    _trackDragMoved = false;
    setState(() {
      _dragFrom.value = idx;
      _dragTo.value = idx;
      _dragging = true;
    });
  }

  @override
  void _handleLongPressMove(Offset p) {
    // Master（Conductor）固定在 0 号位，拖动不改目标。
    if (_dragFrom.value <= 0) {
      return;
    }
    final idx = _rowAt(p);
    if (idx == null) {
      return;
    }
    // 手指到过别的行就算"移动过"（即使最后回到原位，也不弹菜单）。
    if (idx != _dragFrom.value) {
      _trackDragMoved = true;
    }
    if (idx != _dragTo.value) {
      setState(() => _dragTo.value = idx);
    }
  }

  @override
  void _handleLongPressEnd() {
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
}
