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
    rust.resetTouch();
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
    final done = await showExportProgressDialog(
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
    final done = await showExportProgressDialog(
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

  /// 导出 .ying 工程：Rust 写临时文件 → Kotlin 复制到用户选择的位置。
  /// 是否压缩包扩展名（与 Rust/桌面同一份列表）。
  @override
  bool _isArchivePath(String path) {
    final lower = path.toLowerCase();
    const exts = [
      '.zip',
      '.7z',
      '.rar',
      '.lzh',
      '.lha',
      '.tar',
      '.gz',
      '.xz',
      '.tgz',
      '.txz',
      '.tbz',
      '.bz2',
    ];
    return exts.any(lower.endsWith);
  }

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
    if (rust.archiveOpenEntry(index) == 1) {
      setState(() => _fileName = entries[index].name);
      rust.setViewMode(kViewArrangement);
      rust.setDisplayTrack(-1);
      rust.setActiveTrack(-1);
      setState(() {
        _showHome = false;
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

  @override
  Future<void> _saveProject({bool saveAs = false}) async {
    if (_tracks.value.isEmpty) {
      showSnack(context, l.openFirst);
      return;
    }
    // 顶栏保存按钮转圈；导出+写盘期间保持，完成/取消后由超时兜底复位。
    _savingTimer?.cancel();
    setState(() {
      _saving = true;
      _saveDone = false;
    });
    _savingTimer = Timer(const Duration(seconds: 20), () {
      _savingTimer = null;
      if (mounted) {
        setState(() => _saving = false);
      }
    });
    final name = _songTitle.isNotEmpty ? _songTitle : (_fileName ?? 'Untitled');
    final tmp = await _channel.invokeMethod<String>('exportTempPath');
    if (tmp == null || !mounted) {
      return;
    }
    final p = tmp.toNativeUtf8();
    var ok = 0;
    try {
      ok = rust.saveYing(p);
    } finally {
      calloc.free(p);
    }
    if (ok != 1) {
      _savingTimer?.cancel();
      _savingTimer = null;
      if (mounted) {
        setState(() => _saving = false);
        showSnack(context, l.exportFailed);
      }
      return;
    }
    await _channel.invokeMethod<void>('saveProject', {
      'name': name,
      'saveAs': saveAs,
    });
  }

  @override
  void _openRecent(RecentItem item) {
    _archivePolling = _isArchivePath(item.path);
    final p = item.path.toNativeUtf8();
    try {
      rust.openPath(p);
    } finally {
      calloc.free(p);
    }
    setState(() {
      _fileName = item.name;
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
    _rememberProject(item.name);
  }

  /// 按文件名恢复工程 SAF uri（最近列表打开 .ying 后可直接覆盖保存）。
  @override
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
    rust.resetTouch();
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
    rust.resetTouch();
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

  /// 音色库压缩包：列 SF2/SF3/SFZ → 选择 → 后台整包解压/import。
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
    final done = await showExportProgressDialog(
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
