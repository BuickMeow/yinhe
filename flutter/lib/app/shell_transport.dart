part of 'studio_shell.dart';

mixin _ShellTransport on _ShellBase {
  /// 读取当前轨道的自动化 lane（仅 PB/CC/RPN）。
  @override
  void _loadAutomation() {
    final rows = _tracks.value;
    if (rows.isEmpty) {
      setState(() => _autoLanes = const []);
      return;
    }
    final track = (_activeTrack ?? 0).clamp(0, rows.length - 1);
    final ptr = rust.automationJson(track);
    var lanes = const <AutomationLaneData>[];
    if (ptr != ffi.nullptr) {
      try {
        final list = jsonDecode(ptr.toDartString()) as List<dynamic>;
        lanes = [
          for (final e in list)
            AutomationLaneData.fromJson(e as Map<String, dynamic>),
        ];
      } catch (_) {
        lanes = const [];
      } finally {
        rust.freeString(ptr);
      }
    }
    if (!mounted) {
      return;
    }
    setState(() => _autoLanes = lanes);
  }

  /// 可见窗口变化后延迟重载力度条，避免滚动时每帧扫音符。
  void _scheduleVelocityReload() {
    if (_viewMode != kViewPianoroll || _showHome || _autoCollapsed) {
      return;
    }
    final s = _snap.value;
    if (s.length < 3) {
      return;
    }
    final ppu = s[1];
    final scroll = s[2];
    if (_velocityViewPpu == ppu && _velocityViewScroll == scroll) {
      return;
    }
    _velocityViewPpu = ppu;
    _velocityViewScroll = scroll;
    // 节流而不是防抖：持续滚动时也保持力度条更新，但最多约 12Hz。
    if (_velocityReloadTimer != null) {
      return;
    }
    _velocityReloadTimer = Timer(const Duration(milliseconds: 80), () {
      _velocityReloadTimer = null;
      if (!mounted ||
          _viewMode != kViewPianoroll ||
          _showHome ||
          _autoCollapsed) {
        return;
      }
      _loadVelocity();
    });
  }

  /// 读取当前轨道的力度条数据（每音符一条）。
  @override
  void _loadVelocity() {
    final rows = _tracks.value;
    if (rows.isEmpty) {
      setState(() => _velocityBars = const []);
      return;
    }
    final lenPtr = calloc<ffi.Uint32>();
    var bars = const <(double, double, double, int)>[];
    try {
      final ptr = rust.velocityBars(lenPtr);
      final len = lenPtr.value;
      if (ptr != ffi.nullptr && len >= 4) {
        final data = ptr.asTypedList(len);
        bars = [
          for (var i = 0; i + 3 < len; i += 4)
            (data[i], data[i + 1], data[i + 2], data[i + 3].toInt()),
        ];
      }
    } finally {
      calloc.free(lenPtr);
    }
    if (!mounted) {
      return;
    }
    final s = _snap.value;
    if (s.length > 2) {
      _velocityViewPpu = s[1];
      _velocityViewScroll = s[2];
    }
    setState(() => _velocityBars = bars);
  }

  /// 提交 PR 力度笔划：Rust 侧按原音符重算并合成一个 undo。
  @override
  void _commitVelocityStroke(List<(double, double)> points) {
    if (points.isEmpty) {
      return;
    }
    final ptr = calloc<ffi.Double>(points.length * 2);
    var changed = 0;
    try {
      for (var i = 0; i < points.length; i++) {
        ptr[i * 2] = points[i].$1;
        ptr[i * 2 + 1] = points[i].$2;
      }
      changed = rust.velocityStroke(ptr, points.length);
    } finally {
      calloc.free(ptr);
    }
    if (changed == 1) {
      _readDocState();
      _loadVelocity();
    }
  }

  @override
  void _setAutoCollapsed(bool collapsed) {
    setState(() => _autoCollapsed = collapsed);
    if (!collapsed && _viewMode == kViewPianoroll) {
      _loadVelocity();
    }
  }

  /// 拖动 AM 栏顶部手柄：向上（dy < 0）变高，最大占画布 60%。
  @override
  void _resizeAutoPanel(double dy) {
    if (_autoCollapsed) {
      return;
    }
    final maxH = _autoPanelMaxHeight;
    setState(() {
      _autoPanelHeight = (_autoPanelHeight - dy).clamp(
        AutomationPanel.minHeight,
        maxH < AutomationPanel.minHeight ? AutomationPanel.minHeight : maxH,
      );
    });
  }

  /// 双击手柄恢复默认高度。
  @override
  void _resetAutoPanelHeight() {
    setState(() => _autoPanelHeight = AutomationPanel.defaultHeight);
  }
  // ── 走带 ──

  @override
  void _togglePlay() {
    final next = !_playing;
    setState(() => _playing = next);
    rust.transport(next ? 0 : 1);
    _updateTimecode();
  }

  @override
  void _stopPlayback() {
    rust.transport(2);
    _updateTimecode();
    setState(() => _playing = false);
  }

  /// 每帧刷新时间码文本（只触发文本重建，不动整棵布局）。
  @override
  void _updateTimecode() {
    if (!mounted) {
      return;
    }
    // 长按微调时时间码临时显示手指位置（松手/取消后恢复播放光标）。
    final preview = _lpPreviewTick;
    final text = _timecodeBar
        ? _fmtBarBeat(preview ?? rust.playheadTick(), _ppq)
        : _fmtTime(
            preview != null
                ? rust.secondsAtTick(preview)
                : rust.playbackSeconds(),
          );
    if (_timecodeText.value != text) {
      _timecodeText.value = text;
    }
    // 顶栏 BPM / 拍号：跟随播放位置的实时值（快照 10/11/12）。
    final s = _snap.value;
    if (s.length > 12) {
      final bpmText = _fmtBpm(s[12]);
      final sigNum = s[10] > 0 ? s[10].round() : 4;
      final sigDen = 1 << (s[11] >= 0 ? s[11].round().clamp(0, 8) : 2);
      final sigText = '$sigNum/$sigDen';
      if (bpmText != _liveBpmText || sigText != _liveSigText) {
        setState(() {
          _liveBpmText = bpmText;
          _liveSigText = sigText;
        });
      }
    }
    final playing = rust.isPlaying() == 1;
    if (playing != _playing) {
      setState(() => _playing = playing);
    }
  }

  /// 秒数（`m:ss.mmm`）。
  String _fmtTime(double seconds) {
    final total = seconds.floor();
    final m = total ~/ 60;
    final s = total % 60;
    final ms = ((seconds - total) * 1000).floor();
    return '$m:${s.toString().padLeft(2, '0')}.${ms.toString().padLeft(3, '0')}';
  }

  /// 小节.拍.tick（1.0.000 风格：bar 1 起、beat 0 起）。
  ///
  /// 用拍号分段换算（拍号变化时 bar 连续计数；原生事件优先）。
  String _fmtBarBeat(double tick, double ppq) {
    final grid = _grid.value;
    if (grid != null) {
      return grid.format(tick);
    }
    if (ppq <= 0) {
      return '1.0.000';
    }
    return '1.0.000';
  }

  /// 读取拍号分段（文档变化时刷新）。
  @override
  void _loadTimeSigSegments() {
    final ptr = rust.timeSigSegmentsJson();
    if (ptr == ffi.nullptr) {
      return;
    }
    try {
      final list = jsonDecode(ptr.toDartString()) as List<dynamic>;
      final flat = <double>[];
      for (final seg in list) {
        if (seg is List && seg.length >= 3) {
          flat
            ..add((seg[0] as num).toDouble())
            ..add((seg[1] as num).toDouble())
            ..add((seg[2] as num).toDouble());
        }
      }
      _grid.value = BarBeatGrid.fromFlat(_ppq, flat);
    } catch (_) {
      // 保持旧分段。
    } finally {
      rust.freeString(ptr);
    }
  }

  @override
  double get _ppq {
    final v = _snap.value[8];
    return v > 0 ? v : 480;
  }
}
