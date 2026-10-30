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
    final ppu = s[SnapField.ppu];
    final scroll = s[SnapField.scrollX];
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
      _velocityViewPpu = s[SnapField.ppu];
      _velocityViewScroll = s[SnapField.scrollX];
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
      // FFI 的 len 是扁平 f64 元素数（tick/velocity 各一个），不是点对数。
      changed = rust.velocityStroke(ptr, points.length * 2);
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
      final bpmText = _fmtBpm(s[SnapField.bpm]);
      final sigNum = s[SnapField.sigNum] > 0 ? s[SnapField.sigNum].round() : 4;
      final sigDen =
          1 <<
          (s[SnapField.sigDen] >= 0
              ? s[SnapField.sigDen].round().clamp(0, 8)
              : 2);
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
  /// 拍号分段换算统一由 Rust 完成（与桌面 `format_tick_bar_beat_with_time_sig`
  /// 同一实现），Dart 不再自算，避免两边 bar 号漂移。
  String _fmtBarBeat(double tick, double ppq) {
    final ptr = rust.barBeatText(tick);
    if (ptr == ffi.nullptr) {
      return '1.0.000';
    }
    try {
      final text = ptr.toDartString();
      return text.isEmpty ? '1.0.000' : text;
    } finally {
      rust.freeString(ptr);
    }
  }

  /// 顶部栏临时读数的位置文本：跟随时间码当前的小节/秒模式。
  @override
  String _formatEditTick(double tick) {
    if (_timecodeBar) {
      return _fmtBarBeat(tick, _ppq);
    }
    return _fmtTime(rust.secondsAtTick(tick));
  }

  @override
  double get _ppq {
    final v = _snap.value[8];
    return v > 0 ? v : 480;
  }
}
