part of 'studio_shell.dart';

mixin _ShellDocument on _ShellBase {
  /// 打开工程设置对话框（歌名/艺术家/PPQ/简介）。
  @override
  void _openProjectSettings() {
    final ptr = rust.projectMetaJson();
    Map<String, dynamic> meta = const {};
    if (ptr != ffi.nullptr) {
      try {
        meta = jsonDecode(ptr.toDartString()) as Map<String, dynamic>;
      } catch (_) {
        meta = const {};
      } finally {
        rust.freeString(ptr);
      }
    }
    if (meta.isEmpty) {
      showSnack(context, l.openFirst);
      return;
    }
    _resetInteraction();
    showDialog<void>(
      context: context,
      builder: (_) => ProjectDialog(
        noteCount: _noteCount,
        initialName: meta['name'] as String? ?? '',
        initialArtist: meta['artist'] as String? ?? '',
        initialDescription: meta['description'] as String? ?? '',
        initialPpq: (meta['ppq'] as num?)?.toInt() ?? 480,
        onSave: (name, artist, desc, ppq) {
          final n = name.toNativeUtf8();
          final a = artist.toNativeUtf8();
          final d = desc.toNativeUtf8();
          try {
            if (rust.setProjectMeta(n, a, d, ppq) == 1) {
              _refreshDocumentMeta();
            }
          } finally {
            calloc.free(n);
            calloc.free(a);
            calloc.free(d);
          }
        },
      ),
    );
  }

  String _exportBaseName() {
    final raw = _songTitle.isNotEmpty ? _songTitle : (_fileName ?? 'Untitled');
    return raw.replaceAll(RegExp(r'\.[^.]+$'), '');
  }

  Future<String?> _exportTempPath(String ext) {
    return _channel.invokeMethod<String>('exportTempPath', {'ext': ext});
  }

  /// 主页：导出 WAV（位深可选；采样率跟随设备）。
  @override
  Future<void> _exportAudio() async {
    if (_tracks.value.isEmpty) {
      showSnack(context, l.openFirst);
      return;
    }
    if (_playing) {
      _stopPlayback();
    }
    final depth = await showAudioExportOptions(context);
    if (!mounted || depth == null) {
      return;
    }
    final tmp = await _exportTempPath('wav');
    if (!mounted || tmp == null) {
      return;
    }
    final p = tmp.toNativeUtf8();
    var ok = 0;
    try {
      ok = rust.exportAudioStart(p, depth);
    } finally {
      calloc.free(p);
    }
    if (!mounted) {
      return;
    }
    if (ok != 1) {
      showSnack(context, l.exportFailedShort);
      return;
    }
    final done = await showTaskProgressDialog(
      context,
      poll: () => rust.takeString(rust.exportAudioProgressJson()),
      onCancel: rust.exportAudioCancel,
    );
    if (!mounted || done != true) {
      return;
    }
    await _channel.invokeMethod<void>('saveExport', {
      'name': '${_exportBaseName()}.wav',
      'mime': 'audio/wav',
      'ext': 'wav',
    });
  }

  /// 主页：导出 MIDI（保留 0 BPM 跳转标记与完整时间轴）。
  @override
  Future<void> _exportMidi() async {
    if (_tracks.value.isEmpty) {
      showSnack(context, l.openFirst);
      return;
    }
    if (_playing) {
      _stopPlayback();
    }
    final tmp = await _exportTempPath('mid');
    if (!mounted || tmp == null) {
      return;
    }
    const opts =
        '{"encoding":"utf8","rpn_full":true,"curve_interpolate":false,'
        '"strip_empty_tracks":true,"dedup_overlaps":false}';
    final p = tmp.toNativeUtf8();
    final o = opts.toNativeUtf8();
    var ok = 0;
    try {
      ok = rust.exportMidiStart(p, o);
    } finally {
      calloc.free(p);
      calloc.free(o);
    }
    if (!mounted) {
      return;
    }
    if (ok != 1) {
      showSnack(context, l.exportFailedShort);
      return;
    }
    final done = await showTaskProgressDialog(
      context,
      poll: () => rust.takeString(rust.exportMidiStatusJson()),
      determinate: false,
    );
    if (!mounted || done != true) {
      return;
    }
    await _channel.invokeMethod<void>('saveExport', {
      'name': '${_exportBaseName()}.mid',
      'mime': 'audio/midi',
      'ext': 'mid',
    });
  }

  /// 是否压缩包扩展名（唯一一份列表在 `core/platform_files.dart`）。
  bool _isArchivePath(String path) => isArchivePath(path);

  /// 轮询压缩包状态：1=打开中（显示转圈）2=就绪（选条目）3=需要密码 4=失败。
  void _pollArchive() {
    if (!_archivePolling || !mounted) {
      return;
    }
    switch (rust.archiveStatus()) {
      case 1:
        if (!_archiveBusy) {
          _archiveBusy = true;
          showDialog<void>(
            context: context,
            barrierDismissible: false,
            builder: (_) => const Center(child: CircularProgressIndicator()),
          ).whenComplete(() => _archiveBusy = false);
        }
      case 2:
        _archiveBusy = false;
        _archivePolling = false;
        _closeArchiveBusy();
        _showArchiveEntries();
      case 3:
        _archiveBusy = false;
        _closeArchiveBusy();
        _askArchivePassword();
      case 4:
        _archiveBusy = false;
        _archivePolling = false;
        _closeArchiveBusy();
        final err = _readArchiveError();
        showSnack(context, err.isEmpty ? l.archiveOpenFailed : err);
      default:
        break;
    }
  }

  void _closeArchiveBusy() {
    if (_archiveBusy && mounted) {
      _archiveBusy = false;
      Navigator.of(context, rootNavigator: true).pop();
    }
  }

  Map<String, dynamic> _readArchiveJson() {
    final ptr = rust.archiveJson();
    if (ptr == ffi.nullptr) {
      return const {};
    }
    try {
      return jsonDecode(ptr.toDartString()) as Map<String, dynamic>;
    } catch (_) {
      return const {};
    } finally {
      rust.freeString(ptr);
    }
  }

  String _readArchiveError() {
    final json = _readArchiveJson();
    return json['error'] as String? ?? '';
  }

  Future<void> _showArchiveEntries() async {
    final json = _readArchiveJson();
    final entries = parseArchiveEntries(json);
    if (entries.isEmpty) {
      showSnack(context, l.noMidiInArchive);
      return;
    }
    final path = json['path'] as String? ?? '';
    final index = await showArchiveEntriesDialog(
      context,
      path: path,
      entries: entries,
    );
    if (index == null || !mounted) {
      return;
    }
    final target = '${json['path'] ?? ''}#${entries[index].name}';
    if (rust.takeString(rust.currentPath()) == target) {
      _returnToWorkspace();
      return;
    }
    if (!await _confirmReplace()) {
      return;
    }
    if (!mounted) {
      return;
    }
    if (rust.archiveOpenEntry(index) == 1) {
      unawaited(_showLoadProgress());
      setState(() => _fileName = entries[index].name);
      rust.setViewMode(kViewArrangement);
      rust.setDisplayTrack(-1);
      rust.setActiveTrack(-1);
      setState(() {
        _viewMode = kViewArrangement;
        _activeTrack = null;
        _displayTrack = -1;
        _showAllTracks = true;
      });
    }
  }

  Future<void> _askArchivePassword() async {
    final json = _readArchiveJson();
    final path = json['path'] as String? ?? '';
    final password = await showArchivePasswordDialog(context, path);
    if (password == null || !mounted) {
      return;
    }
    final p = path.toNativeUtf8();
    final w = password.toNativeUtf8();
    try {
      rust.archiveOpenPassword(p, w);
    } finally {
      calloc.free(p);
      calloc.free(w);
    }
    _archivePolling = true;
  }

  /// 保存成功后 2 秒内把顶栏保存按钮显示为对勾。
  @override
  void _flashSaved() {
    _saveDoneTimer?.cancel();
    _savingTimer?.cancel();
    _savingTimer = null;
    setState(() {
      _saveDone = true;
      _saving = false;
    });
    _saveDoneTimer = Timer(const Duration(seconds: 2), () {
      _saveDoneTimer = null;
      if (mounted) {
        setState(() => _saveDone = false);
      }
    });
  }

  /// 替换当前文档前确认未保存修改。返回 true = 可以继续。
  @override
  Future<bool> _confirmReplace() async {
    _readDocState();
    if ((_docState.value & 4) == 0) {
      return true;
    }
    final action = await showUnsavedChangesDialog(context);
    if (!mounted) {
      return false;
    }
    switch (action) {
      case UnsavedAction.cancel:
        return false;
      case UnsavedAction.discard:
        return true;
      case UnsavedAction.save:
        return _saveProjectAndWait();
    }
  }

  /// 侧栏重新打开同一文件：文档还在内存里，直接回工作区，不重新解析。
  void _returnToWorkspace() {
    _scaffoldKey.currentState?.closeDrawer();
    _ensureSurface();
  }

  /// 真正开始替换文档（调用方已完成未保存确认）。
  void _startOpenPath(String path) {
    _archivePolling = _isArchivePath(path);
    setState(() {
      _fileName = path.split('/').last;
      _viewMode = kViewArrangement;
      _activeTrack = null;
      _displayTrack = -1;
      _showAllTracks = true;
      _toolAr = 0;
      _mainToolAr = 0;
      _quickToolAr = quickToolsAr.last;
      _quickSlotActiveAr = false;
    });
    rust.setViewMode(kViewArrangement);
    rust.setDisplayTrack(-1);
    rust.setActiveTrack(-1);
    rust.setTool(1);
    final p = path.toNativeUtf8();
    try {
      rust.openPath(p);
    } finally {
      calloc.free(p);
    }
    _ensureSurface();
    _rememberProject(path.split('/').last);
    if (!_archivePolling) {
      unawaited(_showLoadProgress());
    }
  }

  /// 文件选择回调：先判同一文件/未保存，再让 Rust 开始加载。
  @override
  Future<void> _handlePickedFile(String path) async {
    final current = rust.takeString(rust.currentPath());
    if (current.isNotEmpty && current == path) {
      _returnToWorkspace();
      return;
    }
    // 压缩包只是打开会话、不替换文档；等选中条目时再确认未保存。
    if (!_isArchivePath(path) && !await _confirmReplace()) {
      return;
    }
    if (!mounted) {
      return;
    }
    _startOpenPath(path);
  }

  @override
  Future<bool> _saveProject({bool saveAs = false}) {
    return _saveProjectImpl(saveAs: saveAs);
  }

  Future<bool> _saveProjectAndWait() {
    return _saveProjectImpl(waitForCompletion: true);
  }

  Future<bool> _saveProjectImpl({
    bool saveAs = false,
    bool waitForCompletion = false,
  }) async {
    if (_tracks.value.isEmpty) {
      showSnack(context, l.openFirst);
      return false;
    }
    if (_saveCompleter != null) {
      return false;
    }
    _savingTimer?.cancel();
    setState(() {
      _saving = true;
      _saveDone = false;
    });
    final name = _songTitle.isNotEmpty ? _songTitle : (_fileName ?? 'Untitled');
    final tmp = await _channel.invokeMethod<String>('exportTempPath');
    if (!mounted) {
      return false;
    }
    if (tmp == null) {
      setState(() => _saving = false);
      return false;
    }
    final p = tmp.toNativeUtf8();
    var ok = 0;
    try {
      ok = rust.saveProjectStart(p);
    } finally {
      calloc.free(p);
    }
    if (!mounted) {
      return false;
    }
    if (ok != 1) {
      setState(() => _saving = false);
      showSnack(context, l.saveFailed);
      return false;
    }
    final done = await showTaskProgressDialog(
      context,
      poll: () => rust.takeString(rust.saveProjectStatusJson()),
      determinate: true,
      title: l.savingProject,
      failureTitle: l.saveFailed,
    );
    if (!mounted) {
      return false;
    }
    if (done != true) {
      setState(() => _saving = false);
      return false;
    }
    // Rust 写临时文件完成；接下来交给 SAF 选择位置并复制。
    _savingTimer = Timer(const Duration(seconds: 20), () {
      _savingTimer = null;
      if (mounted) {
        setState(() => _saving = false);
      }
    });
    Completer<bool>? completer;
    if (waitForCompletion) {
      completer = Completer<bool>();
      _saveCompleter = completer;
    }
    try {
      await _channel.invokeMethod<void>('saveProject', {
        'name': name,
        'saveAs': saveAs,
      });
    } catch (_) {
      _savingTimer?.cancel();
      _savingTimer = null;
      if (mounted) {
        setState(() => _saving = false);
      }
      _finishSaveWait(false);
      return false;
    }
    if (completer == null) {
      return true;
    }
    return completer.future;
  }

  @override
  void _openRecent(RecentItem item) {
    unawaited(_openRecentAsync(item));
  }

  Future<void> _openRecentAsync(RecentItem item) async {
    final current = rust.takeString(rust.currentPath());
    if (current.isNotEmpty && current == item.path) {
      _returnToWorkspace();
      return;
    }
    if (!_isArchivePath(item.path)) {
      if (!await _confirmReplace()) {
        return;
      }
    }
    if (!mounted) {
      return;
    }
    _startOpenPath(item.path);
  }

  /// 按文件名恢复工程 SAF uri（最近列表打开 .yin 后可直接覆盖保存）。
  void _rememberProject(String name) {
    _channel.invokeMethod<void>('rememberProject', {'name': name});
  }

  /// 读取歌曲标题（meta.name）。
  @override
  void _loadSongTitle() {
    final ptr = rust.songTitle();
    if (ptr == ffi.nullptr) {
      return;
    }
    try {
      final title = ptr.toDartString();
      if (!mounted) {
        return;
      }
      setState(() => _songTitle = title);
    } finally {
      rust.freeString(ptr);
    }
  }

  /// 读取持久化的界面语言（system/zh/en）。
  void _loadLanguage() {
    final ptr = rust.uiLanguage();
    if (ptr == ffi.nullptr) {
      return;
    }
    try {
      final lang = ptr.toDartString();
      if (lang.isNotEmpty) {
        L10n.appLanguage.value = lang;
      }
    } finally {
      rust.freeString(ptr);
    }
  }

  /// 同步全曲音符数与本工程默认 BPM / 拍号（顶栏展示）。
  @override
  void _loadProjectDefaults() {
    if (!mounted) {
      return;
    }
    final count = rust.noteCount();
    final ptr = rust.projectDefaultsJson();
    var bpm = _projectBpm;
    var sigNum = _projectSigNum;
    var sigDen = _projectSigDen;
    if (ptr != ffi.nullptr) {
      try {
        final json = jsonDecode(ptr.toDartString()) as Map<String, dynamic>;
        bpm = (json['bpm'] as num?)?.toDouble() ?? bpm;
        sigNum = (json['num'] as num?)?.toInt() ?? sigNum;
        sigDen = (json['den'] as num?)?.toInt() ?? sigDen;
      } catch (_) {
        // 保持旧值。
      } finally {
        rust.freeString(ptr);
      }
    }
    setState(() {
      _noteCount = count;
      _projectBpm = bpm;
      _projectSigNum = sigNum;
      _projectSigDen = sigDen;
    });
  }

  @override
  String _fmtBpm(double v) =>
      v == v.roundToDouble() ? '${v.round()}' : v.toStringAsFixed(1);

  /// 编辑本工程默认 BPM（无原生 Tempo 事件时生效）。
  @override
  Future<void> _editDefaultBpm() async {
    if (_tracks.value.isEmpty) {
      return;
    }
    _resetInteraction();
    final bpm = await showDefaultBpmDialog(context, _projectBpm);
    if (bpm == null) {
      return;
    }
    if (rust.setProjectDefaults(bpm, _projectSigNum, _denPow(_projectSigDen)) ==
        1) {
      _loadProjectDefaults();
      _updateTimecode();
    }
  }

  /// 编辑本工程默认拍号（无原生拍号事件时生效）。
  @override
  Future<void> _editDefaultTimeSig() async {
    if (_tracks.value.isEmpty) {
      return;
    }
    _resetInteraction();
    final sig = await showDefaultTimeSigDialog(
      context,
      _projectSigNum,
      _projectSigDen,
    );
    if (sig == null) {
      return;
    }
    if (rust.setProjectDefaults(_projectBpm, sig.$1, _denPow(sig.$2)) == 1) {
      _loadProjectDefaults();
      _updateTimecode();
    }
  }

  /// 分母 → 2 次幂（4 → 2）。
  @override
  int _denPow(int den) {
    var pow = 0;
    var v = den;
    while (v > 1 && pow < 8) {
      v >>= 1;
      pow++;
    }
    return pow;
  }

  Future<int> _waitSoundfontArchiveStatus() async {
    for (var i = 0; i < 300; i++) {
      await Future<void>.delayed(const Duration(milliseconds: 100));
      if (!mounted) {
        return 4;
      }
      final st = rust.soundfontArchiveStatus();
      if (st != 1) {
        return st;
      }
    }
    return 4;
  }

  /// 音色库压缩包：列 SF2/SFZ → 选择 → 后台整包解压/import。
  @override
  Future<void> _openSoundfontArchive(String path) async {
    String? password;
    var opened = false;
    for (var attempt = 0; attempt < 5; attempt++) {
      final p = path.toNativeUtf8();
      var ok = 0;
      try {
        if (password == null) {
          ok = rust.soundfontArchiveOpen(p);
        } else {
          final pw = password.toNativeUtf8();
          try {
            ok = rust.soundfontArchiveOpenPassword(p, pw);
          } finally {
            calloc.free(pw);
          }
        }
      } finally {
        calloc.free(p);
      }
      if (ok != 1) {
        if (mounted) {
          showSnack(context, l.exportFailedShort);
        }
        return;
      }
      final status = await _waitSoundfontArchiveStatus();
      if (!mounted) {
        return;
      }
      if (status == 2) {
        opened = true;
        break;
      }
      if (status == 3) {
        final pw = await showArchivePasswordDialog(context, path);
        if (!mounted || pw == null) {
          return;
        }
        password = pw;
        continue;
      }
      showSnack(context, l.exportFailedShort);
      return;
    }
    if (!opened || !mounted) {
      return;
    }
    Map<String, dynamic> archiveJson = const {};
    try {
      archiveJson = jsonDecode(
        rust.takeString(rust.soundfontArchiveJson()),
      ) as Map<String, dynamic>;
    } catch (_) {
      archiveJson = const {};
    }
    final entries = parseArchiveEntries(archiveJson);
    if (entries.isEmpty) {
      showSnack(context, l.noSoundfontInArchive);
      return;
    }
    final index = await showArchiveEntriesDialog(
      context,
      path: path,
      entries: entries,
      countLabel: l.soundfonts,
    );
    if (!mounted || index == null) {
      return;
    }
    if (rust.soundfontArchiveImport(index) != 1) {
      showSnack(context, l.exportFailedShort);
      return;
    }
    final done = await showTaskProgressDialog(
      context,
      poll: () => rust.takeString(rust.soundfontArchiveImportJson()),
      determinate: true,
      title: l.importingSoundfont,
      failureTitle: l.exportFailedShort,
    );
    if (!mounted || done != true) {
      return;
    }
    _sfRevision.value++;
    showSnack(context, l.importSoundfontDone);
  }
}
