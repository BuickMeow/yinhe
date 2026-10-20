part of 'studio_shell.dart';

mixin _ShellCore on _ShellBase {
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
        _toolPr = tool;
      } else {
        _toolAr = tool;
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

  /// 新建空工程并进入工作区。
  @override
  void _newProject() {
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
    _loadAutomation();
    _loadVelocity();
    _loadSongTitle();
    _loadProjectDefaults();
    _loadTimeSigSegments();
    _loadArAutomation();
  }

  Future<void> _onNativeCall(MethodCall call) async {
    if (!mounted) {
      return;
    }
    if (call.method == 'rightPan') {
      _onNativeRightPan(call.arguments as Map?);
      return;
    }
    if (call.method == 'onSoundFontFolderDone') {
      final count = (call.arguments as num?)?.toInt() ?? 0;
      if (count <= 0) {
        showSnack(context, l.sfzFolderEmpty);
      } else {
        _sfRevision.value++;
      }
      return;
    }
    final path = call.arguments as String?;
    if (path == null) {
      return;
    }
    switch (call.method) {
      case 'onFilePicked':
        _archivePolling = _isArchivePath(path);
        setState(() {
          _fileName = path.split('/').last;
          _showHome = false;
          _viewMode = kViewArrangement;
          _activeTrack = null;
          _displayTrack = -1;
          _showAllTracks = true;
        });
        rust.setViewMode(kViewArrangement);
        rust.setDisplayTrack(-1);
        rust.setActiveTrack(-1);
        rust.setTool(toolArAllowed.first);
        _ensureSurface();
        _rememberProject(path.split('/').last);
      case 'onSoundFontPicked':
        // Rust 已由 JNI 入口直接入表并持久化，这里只通知设置页刷新。
        _sfRevision.value++;
      case 'onProjectSaved':
        rust.markSaved();
        _loadSongTitle();
        _readDocState();
        _flashSaved();
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
    if (kind == 0) {
      _cancelLongPressWatch();
    }
    _sendPanTouch(kind == 0 ? 3 : (kind == 1 ? 4 : 5), Offset(x, y), -3);
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

  /// 导入 SFZ 所在目录（连同采样文件一起复制）。
  @override
  Future<void> _pickSfzFolder() async {
    await _channel.invokeMethod<void>('openSoundFontFolder');
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
  }
}
