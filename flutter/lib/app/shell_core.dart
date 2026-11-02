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
  ///
  /// `renderFrame` 已同步等到 present 完成，这里读到的就是刚画进纹理的那份
  /// 状态，直接显示即可，不需要再押一帧。
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
  int get _mainTool => _viewMode == kViewPianoroll ? _mainToolPr : _mainToolAr;

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
    // 移动端只保留一套手势：Rust 固定手形，单击/长按由 Flutter 统一分发。
    rust.setTool(1);
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

  /// 量化档位同步给 Rust（只作用于当前视图：解析与写入都在 Rust）。
  @override
  void _pushQuantize(String label) {
    _applyQuantize(_viewMode, label);
  }

  /// 两个视图的量化档位都同步给 Rust（启动/加载文档/新建工程后）。
  void _pushAllQuantizes() {
    _applyQuantize(kViewArrangement, _quantizeAr);
    _applyQuantize(kViewPianoroll, _quantizePr);
  }

  void _applyQuantize(int view, String label) {
    final p = label.toNativeUtf8();
    try {
      rust.setQuantizeLabel(view, p);
    } finally {
      calloc.free(p);
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
    rust.setTool(1);
    _arrExpandedTracks.value = <String>{};
    _pushAllQuantizes();
    setState(() {
      _fileName = 'Untitled';
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
    // 吃掉当前 generation：菜单等主动刷新之后，ticker 不再重复刷一次。
    _docGeneration = rust.documentGeneration();
    setState(() {
      _recent = _readRecent();
      _tracks.value = _readTracks();
      _markers = _readMarkers();
    });
    // PR 专属面板（力度条/自动化条）不参与 AR 绘制。AR 下每次音符编辑
    // 都重扫全曲力度会卡住主线程，这里只在 PR 视图加载；AM 面板收起时
    // 面板不渲染，也没必要解析整条 lane。
    if (_viewMode == kViewPianoroll && !_autoCollapsed) {
      _loadAutomation();
      _loadVelocity();
    }
    _loadSongTitle();
    _loadProjectDefaults();
  }

  /// 加载 MIDI/工程：Rust 后台解析，Dart 延迟 120ms 再决定是否弹进度框，
  /// 避免小文件一闪而过。返回的 state：0=无 1=加载中 2=完成 3=失败。
  @override
  Future<void> _showLoadProgress() async {
    // 复制进度框可能还在关闭动画里：先等它彻底关掉，再开加载框。
    // 超时只是兜底（平台异常时进程已没有意义），避免永久等待。
    final copyClosed = _copyClosed;
    if (copyClosed != null) {
      await copyClosed.future.timeout(
        const Duration(minutes: 5),
        onTimeout: () {},
      );
    }
    if (!mounted) {
      return;
    }
    // 先占住“UI 未就绪”，避免加载恰好在 120ms 延迟内完成时，
    // generation 刷新已经跑过、标志却没置位，导致对话框永不关闭。
    _loadUiSettled = false;
    await Future<void>.delayed(const Duration(milliseconds: 120));
    if (!mounted) {
      return;
    }
    var state = _loadState();
    if (state != 1 && state != 3) {
      _loadUiSettled = true;
      if (state == 2) {
        // 新文档的量化默认值（AR 1/1 / PR 1/16）已由 Rust 建好；
        // 这里把 Flutter 侧记住的两套档位重新下发，避免 UI 与行为不一致。
        _pushAllQuantizes();
      }
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
    if (!mounted) {
      return;
    }
    if (_loadState() == 2) {
      _pushAllQuantizes();
    }
  }

  /// 当前加载状态：0=无 1=加载中 2=完成 3=失败。
  int _loadState() {
    try {
      final raw = rust.takeString(rust.loadStatusJson());
      return (jsonDecode(raw)['state'] as num?)?.toInt() ?? 0;
    } catch (_) {
      return 0;
    }
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
    // 大文件复制进度（Android/iOS/OHOS 统一协议）。
    if (call.method == 'onCopyStart') {
      final args = (call.arguments as Map?) ?? const {};
      _beginCopyProgress(
        kind: args['kind'] as String? ?? 'open',
        total: (args['total'] as num?)?.toInt() ?? -1,
      );
      return;
    }
    if (call.method == 'onCopyProgress') {
      final args = (call.arguments as Map?) ?? const {};
      _copyDone = (args['done'] as num?)?.toInt() ?? 0;
      _copyTotal = (args['total'] as num?)?.toInt() ?? _copyTotal;
      return;
    }
    // 以下都是复制/保存的终止事件：先收起复制进度框。
    _endCopyProgress();
    if (call.method == 'onFileRejected' ||
        call.method == 'onSoundfontRejected') {
      final name = call.arguments as String? ?? '';
      showSnack(
        context,
        name.isEmpty ? l.loadFailed : '${l.loadFailed}: $name',
      );
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
    switch (call.method) {
      case 'onFilePicked':
        await _handlePickedFile(path);
      case 'onSoundFontPicked':
        // 压缩包只打开会话列条目；裸 SF2/SFZ 才入表（含持久化）。
        if (isArchivePath(path)) {
          _openSoundfontArchive(path);
        } else {
          final p = path.toNativeUtf8();
          try {
            rust.addSoundfont(p);
          } finally {
            malloc.free(p);
          }
          _sfRevision.value++;
        }
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

  // ── 大文件复制进度（三端统一事件）──
  int _copyState = 0; // 0 = 无，1 = 进行中，2 = 已结束
  int _copyDone = 0;
  int _copyTotal = -1;
  int _copyToken = 0;

  /// 复制进度框真正关闭后完成；加载进度框要等它，避免两个路由叠在一起。
  Completer<void>? _copyClosed;

  void _beginCopyProgress({required String kind, required int total}) {
    _copyState = 1;
    _copyDone = 0;
    _copyTotal = total;
    final token = ++_copyToken;
    final closed = Completer<void>();
    _copyClosed = closed;
    final title = switch (kind) {
      'soundfont' => l.importingSoundfont,
      'saveProject' => l.savingProject,
      'saveExport' => l.exporting,
      _ => l.loadingFile,
    };
    // 小文件复制可能 100ms 内就完成：延迟弹框，避免闪一下。
    unawaited(Future<void>.delayed(const Duration(milliseconds: 150), () async {
      if (!mounted || token != _copyToken || _copyState != 1) {
        if (!closed.isCompleted) {
          closed.complete();
        }
        return;
      }
      await showTaskProgressDialog(
        context,
        poll: _copyProgressJson,
        determinate: _copyTotal > 0,
        title: title,
      );
      if (!closed.isCompleted) {
        closed.complete();
      }
    }));
  }

  void _endCopyProgress() {
    if (_copyState == 0) {
      return;
    }
    _copyState = 2;
    _copyToken++;
  }

  String _copyProgressJson() {
    final ratio = _copyTotal > 0
        ? (_copyDone / _copyTotal).clamp(0.0, 1.0)
        : 0.0;
    return '{"state":${_copyState == 1 ? 1 : 2},"progress":$ratio,"status":""}';
  }

  /// 显示后台文档任务的进度框；完成/失败后刷新工程元数据。
  @override
  Future<bool> _runDocJob({
    required String title,
    bool determinate = false,
    bool cancelable = false,
  }) async {
    // 小操作通常 150ms 内完成：先等一下，已经结束就不弹框（撤销/重做不闪）。
    await Future<void>.delayed(const Duration(milliseconds: 150));
    if (!mounted) {
      return false;
    }
    final state = _docJobState();
    if (state == 0 || state == 2) {
      _refreshDocumentMeta();
      return true;
    }
    final done = await showTaskProgressDialog(
      context,
      poll: () => rust.takeString(rust.docJobStatusJson()),
      determinate: determinate,
      title: title,
      failureTitle: l.loadFailed,
      onCancel: cancelable ? () => rust.docJobCancel() : null,
    );
    if (!mounted) {
      return false;
    }
    if (done == true) {
      _refreshDocumentMeta();
    }
    return done == true;
  }

  int _docJobState() {
    try {
      final raw = rust.takeString(rust.docJobStatusJson());
      return (jsonDecode(raw)['state'] as num?)?.toInt() ?? 0;
    } catch (_) {
      return 0;
    }
  }

  Future<void> _init() async {
    final id = await _channel.invokeMethod<int>('getTextureId');
    final amId = await _channel.invokeMethod<int>('getAutomationTextureId');
    if (!mounted) {
      return;
    }
    setState(() {
      _textureId = id;
      _amTextureId = amId;
    });
    await _ensureSurface();
    // 启动时打开侧栏：替代旧主页，用户第一眼就能选文件/设置。
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) {
        _scaffoldKey.currentState?.openDrawer();
      }
    });
  }

  /// 确保 Rust 已拿到生产端 surface（部分引擎版本不触发 SurfaceProducer 回调）。
  @override
  Future<void> _ensureSurface() async {
    await _channel.invokeMethod<void>('ensureSurface');
  }

  @override
  Future<void> _pickMidi() async {
    // 扩展名策略在 Dart（`core/platform_files.dart`），壳只按它做校验。
    await _channel.invokeMethod<void>('openFilePicker', {
      'exts': projectExtensions,
    });
  }

  @override
  Future<void> _pickSoundFont() async {
    await _channel.invokeMethod<void>('openSoundFontPicker', {
      'exts': soundfontExtensions,
    });
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
    _pushQuantize(_quantizeLabel);
    _velocityViewPpu = null;
    _velocityViewScroll = null;
    if (mode == kViewPianoroll && !_autoCollapsed) {
      _loadAutomation();
      _loadVelocity();
    }
  }
}
