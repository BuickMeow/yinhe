part of 'studio_shell.dart';

mixin _ShellCore on _ShellBase {
  Offset? _rightPanLast;

  // ── 数据读取 ──

  List<double> _readSnapshot() {
    if (rust.viewSnapshot(_snapPtr) == 0) {
      return _snap.value;
    }
    return List<double>.generate(13, (i) => _snapPtr[i]);
  }

  /// 视图快照是否与当前一致（避免无变化时触发整层重绘）。
  bool _sameSnapshot(List<double> next) {
    final cur = _snap.value;
    if (identical(cur, next)) {
      return true;
    }
    for (var i = 0; i < next.length; i++) {
      if (i >= cur.length || cur[i] != next[i]) {
        return false;
      }
    }
    return true;
  }

  /// 覆盖层：仅在修订号变化时重取 JSON。
  void _readOverlay() {
    final rev = rust.overlayRevision();
    if (rev == _overlayRev) {
      return;
    }
    _overlayRev = rev;
    final ptr = rust.overlayJson();
    if (ptr == ffi.nullptr) {
      return;
    }
    try {
      final json = jsonDecode(ptr.toDartString()) as Map<String, dynamic>;
      _overlay.value = OverlayData.fromJson(json);
    } catch (_) {
      _overlay.value = OverlayData.empty;
    } finally {
      rust.freeString(ptr);
    }
  }

  /// 工程状态（撤销/重做可用性、未保存标记）。
  @override
  void _readDocState() {
    if (rust.docState(_statePtr) != 1) {
      return;
    }
    final bits =
        (_statePtr[0] & 1) |
        ((_statePtr[1] & 1) << 1) |
        ((_statePtr[2] & 1) << 2);
    if (_docState.value != bits) {
      _docState.value = bits;
    }
  }

  /// 当前视图允许的工具。
  @override
  int get _tool => _viewMode == kViewPianoroll ? _toolPr : _toolAr;

  /// 主工具按钮当前记住的工具。
  @override
  int get _mainTool =>
      _viewMode == kViewPianoroll ? _mainToolPr : _mainToolAr;

  /// 快捷工具组当前记住的工具（与主工具按钮解耦）。
  @override
  int get _quickTool =>
      _viewMode == kViewPianoroll ? _quickToolPr : _quickToolAr;

  /// 最近一次操作的是否为快捷槽位；决定哪个按钮高亮。
  @override
  bool get _quickSlotActive =>
      _viewMode == kViewPianoroll ? _quickSlotActivePr : _quickSlotActiveAr;

  /// 把当前视图的工具同步给 Rust。
  void _pushTool() {
    final list = _viewMode == kViewPianoroll ? toolPrAllowed : toolArAllowed;
    final t = list.contains(_tool) ? _tool : list.first;
    rust.setTool(t);
  }

  @override
  void _selectTool(int tool) {
    setState(() {
      if (_viewMode == kViewPianoroll) {
        _mainToolPr = tool;
        _toolPr = tool;
        _quickSlotActivePr = false;
      } else {
        _mainToolAr = tool;
        _toolAr = tool;
        _quickSlotActiveAr = false;
      }
    });
    _pushTool();
  }

  /// 快捷工具组只更新自己的槽位，同时把该工具设为当前工具；
  /// 主工具 popup 改工具时不会回写快捷槽位，两个按钮因此互相独立。
  @override
  void _selectQuickTool(int tool) {
    setState(() {
      if (_viewMode == kViewPianoroll) {
        _quickToolPr = tool;
        _toolPr = tool;
        _quickSlotActivePr = true;
      } else {
        _quickToolAr = tool;
        _toolAr = tool;
        _quickSlotActiveAr = true;
      }
    });
    _pushTool();
  }

  /// 量化档位同步给 Rust（工具吸附用）。
  @override
  void _pushQuantize(String label) {
    if (label == '关闭') {
      rust.setQuantize(0, 1, 1);
    } else if (label.startsWith('1/')) {
      final den = int.tryParse(label.substring(2)) ?? 4;
      rust.setQuantize(2, 1, den);
    } else {
      rust.setQuantize(1, 1, 1);
    }
  }

  /// 新建空工程并进入工作区（先确认未保存修改）。
  @override
  void _newProject() {
    unawaited(_newProjectAfterConfirm());
  }

  Future<void> _newProjectAfterConfirm() async {
    if (!await _confirmReplace()) {
      return;
    }
    if (!mounted) {
      return;
    }
    rust.newProject();
    rust.setViewMode(kViewArrangement);
    rust.setDisplayTrack(-1);
    rust.setActiveTrack(-1);
    rust.setTool(toolArAllowed.first);
    setState(() {
      _fileName = 'Untitled';
      _showHome = false;
      _viewMode = kViewArrangement;
      _activeTrack = null;
      _displayTrack = -1;
      _showAllTracks = true;
      _toolAr = 0;
      _mainToolAr = 0;
      _quickToolAr = quickToolsAr.last;
      _quickSlotActiveAr = false;
      _songTitle = '';
      _playing = false;
    });
    _updateTimecode();
    _ensureSurface();
  }

  /// 文档加载是后台线程：延迟刷新轨道/最近列表。
  @override
  void _refreshDocumentMeta() {
    if (!mounted) {
      return;
    }
    setState(() {
      _recent = _readRecent();
      _tracks.value = _readTracks();
      _markers = _readMarkers();
    });
    // PR 专属面板（力度条/自动化条）不参与 AR 绘制。AR 下每次音符编辑
    // 都重扫全曲力度会卡住主线程，这里只在 PR 视图加载。
    if (_viewMode == kViewPianoroll) {
      _loadAutomation();
      _loadVelocity();
    }
    _loadSongTitle();
    _loadProjectDefaults();
    _loadTimeSigSegments();
  }

  /// 加载 MIDI/工程：Rust 后台解析，Dart 延迟 120ms 再决定是否弹进度框，
  /// 避免小文件一闪而过。返回的 state：0=无 1=加载中 2=完成 3=失败。
  @override
  Future<void> _showLoadProgress() async {
    // 先占住“UI 未就绪”，避免加载恰好在 120ms 延迟内完成时，
    // generation 刷新已经跑过、标志却没置位，导致对话框永不关闭。
    _loadUiSettled = false;
    await Future<void>.delayed(const Duration(milliseconds: 120));
    if (!mounted) {
      return;
    }
    var state = 0;
    try {
      final raw = rust.takeString(rust.loadStatusJson());
      state = (jsonDecode(raw)['state'] as num?)?.toInt() ?? 0;
    } catch (_) {
      state = 0;
    }
    if (state != 1 && state != 3) {
      _loadUiSettled = true;
      return;
    }
    if (state == 3) {
      // 失败直接显示错误，不需要再等 UI 刷新。
      _loadUiSettled = true;
    }
    await showTaskProgressDialog(
      context,
      poll: _loadProgressJson,
      determinate: true,
      title: l.loadingFile,
      failureTitle: l.loadFailed,
    );
  }

  /// 加载状态轮询：Rust 已到 2（完成）但 Flutter 还没刷新完时，暂时伪装成
  /// 仍在加载，避免进度框先关、画布后出现。
  String _loadProgressJson() {
    final raw = rust.takeString(rust.loadStatusJson());
    if (_loadUiSettled) {
      return raw;
    }
    try {
      final map = jsonDecode(raw) as Map<String, dynamic>;
      if ((map['state'] as num?)?.toInt() == 2) {
        return '{"state":1,"progress":1,"error":""}';
      }
    } catch (_) {
      // 保持原始 JSON，让对话框按错误/完成处理。
    }
    return raw;
  }

  Future<void> _onNativeCall(MethodCall call) async {
    if (!mounted) {
      return;
    }
    if (call.method == 'rightPan') {
      _onNativeRightPan(call.arguments as Map?);
      return;
    }
    if (call.method == 'onProjectSaveCancelled') {
      _savingTimer?.cancel();
      _savingTimer = null;
      if (_saving) {
        setState(() => _saving = false);
      }
      _finishSaveWait(false);
      return;
    }
    if (call.method == 'onProjectSaveFailed') {
      _savingTimer?.cancel();
      _savingTimer = null;
      if (_saving) {
        setState(() => _saving = false);
      }
      _finishSaveWait(false);
      showSnack(context, l.saveFailed);
      return;
    }
    final path = call.arguments as String?;
    if (path == null) {
      return;
    }
    if (call.method == 'onSoundFontArchivePicked') {
      _openSoundfontArchive(path);
      return;
    }
    switch (call.method) {
      case 'onFilePicked':
        await _handlePickedFile(path);
      case 'onSoundFontPicked':
        // Rust 已由 JNI 入口直接入表并持久化，这里只通知设置页刷新。
        _sfRevision.value++;
      case 'onProjectSaved':
        rust.markSaved();
        _loadSongTitle();
        _readDocState();
        _flashSaved();
        _finishSaveWait(true);
      case 'onExportSaved':
        showSnack(context, '${l.exportDone}：$path');
    }
  }

  /// 原生层识别到的鼠标右键（已消费，不会再走 Flutter 左键手势）。
  void _onNativeRightPan(Map? args) {
    if (_showHome || _showSettings || args == null) {
      return;
    }
    final kind = (args['kind'] as num?)?.toInt() ?? 2;
    final dpr = MediaQuery.devicePixelRatioOf(context);
    final padding = MediaQuery.paddingOf(context);
    final x = (args['x'] as num).toDouble() / dpr - padding.left;
    final y =
        (args['y'] as num).toDouble() / dpr -
        padding.top -
        42 -
        RulerPainter.height;
    final p = Offset(x, y);
    if (kind == 0) {
      // 右键按下：取消编辑/长按，强制进入手形平移。
      _resetInteraction();
      _rightPanLast = p;
    } else if (kind == 1) {
      final last = _rightPanLast;
      _rightPanLast = p;
      if (last != null) {
        rust.panBy(p.dx - last.dx, p.dy - last.dy);
      }
    } else {
      _rightPanLast = null;
    }
  }

  Future<void> _init() async {
    final id = await _channel.invokeMethod<int>('getTextureId');
    if (!mounted) {
      return;
    }
    setState(() => _textureId = id);
    await _ensureSurface();
  }

  /// 确保 Rust 已拿到生产端 surface（部分引擎版本不触发 SurfaceProducer 回调）。
  @override
  Future<void> _ensureSurface() async {
    await _channel.invokeMethod<void>('ensureSurface');
  }

  @override
  Future<void> _pickMidi() async {
    await _channel.invokeMethod<void>('openFilePicker');
  }

  @override
  Future<void> _pickSoundFont() async {
    await _channel.invokeMethod<void>('openSoundFontPicker');
  }

  @override
  void _openSettings() {
    setState(() => _showSettings = true);
  }

  List<RecentItem> _readRecent() {
    final ptr = rust.recentJson();
    if (ptr == ffi.nullptr) {
      return const [];
    }
    try {
      final list = jsonDecode(ptr.toDartString()) as List<dynamic>;
      return [
        for (final e in list) RecentItem.fromJson(e as Map<String, dynamic>),
      ];
    } catch (_) {
      return const [];
    } finally {
      rust.freeString(ptr);
    }
  }

  List<TrackRow> _readTracks() {
    final ptr = rust.tracksJson();
    if (ptr == ffi.nullptr) {
      return const [];
    }
    try {
      final list = jsonDecode(ptr.toDartString()) as List<dynamic>;
      return [
        for (final e in list) TrackRow.fromJson(e as Map<String, dynamic>),
      ];
    } catch (_) {
      return const [];
    } finally {
      rust.freeString(ptr);
    }
  }

  /// 标尺标签列表（marker_json）。
  List<RulerMarker> _readMarkers() {
    final ptr = rust.markersJson();
    if (ptr == ffi.nullptr) {
      return const [];
    }
    try {
      final list = jsonDecode(ptr.toDartString()) as List<dynamic>;
      return [
        for (final e in list) RulerMarker.fromJson(e as Map<String, dynamic>),
      ];
    } catch (_) {
      return const [];
    } finally {
      rust.freeString(ptr);
    }
  }

  @override
  void _setView(int mode) {
    if (_viewMode == mode) {
      return;
    }
    rust.setViewMode(mode);
    setState(() => _viewMode = mode);
    _pushTool();
    _velocityViewPpu = null;
    _velocityViewScroll = null;
    if (mode == kViewPianoroll) {
      _loadAutomation();
      _loadVelocity();
    }
  }
}
