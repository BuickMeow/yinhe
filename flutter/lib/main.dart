import 'dart:async';
import 'dart:convert';
import 'dart:ffi' as ffi;

import 'package:ffi/ffi.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';

import 'core/ffi.dart' as rust;
import 'core/snack.dart';
import 'dialogs/add_tracks_dialog.dart';
import 'widgets/automation_panel.dart';
import 'menus/canvas_menus.dart';
import 'widgets/home_view.dart';
import 'dialogs/insert_flow.dart';
import 'l10n.dart';
import 'l10n/app_localizations.dart';
import 'painters/long_press_indicator_painter.dart';
import 'core/models.dart';
import 'widgets/ruler_markers.dart';
import 'dialogs/archive_dialogs.dart';
import 'dialogs/event_list_dialog.dart';
import 'core/bar_beat.dart';
import 'dialogs/project_defaults_dialog.dart';
import 'dialogs/project_dialog.dart';
import 'painters/painters.dart';
import 'widgets/settings_view.dart';
import 'core/theme.dart';
import 'core/tools.dart';
import 'dialogs/track_info_dialog.dart';
import 'widgets/top_bar.dart';

/// 视图模式（与 Rust 侧约定一致：0=AR 走带，1=PR 卷帘）。
const int kViewArrangement = 0;
const int kViewPianoroll = 1;

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  // 沉浸式全屏：隐藏顶部状态栏与底部小白条（滑动可临时唤出）。
  SystemChrome.setEnabledSystemUIMode(SystemUiMode.immersiveSticky);
  SystemChrome.setSystemUIOverlayStyle(
    const SystemUiOverlayStyle(
      statusBarColor: Colors.transparent,
      systemNavigationBarColor: Colors.transparent,
    ),
  );
  runApp(const YinheApp());
}

class YinheApp extends StatelessWidget {
  const YinheApp({super.key});

  @override
  Widget build(BuildContext context) {
    return ValueListenableBuilder<String>(
      valueListenable: L10n.appLanguage,
      builder: (context, _, _) => MaterialApp(
        debugShowCheckedModeBanner: false,
        theme: YinheTheme.data(),
        locale: L10n.locale,
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: const StudioShell(),
      ),
    );
  }
}

/// 工作台壳：画布常驻，主页为覆盖浮层，上侧栏为走带控制。
class StudioShell extends StatefulWidget {
  const StudioShell({super.key});

  @override
  State<StudioShell> createState() => _StudioShellState();
}

class _StudioShellState extends State<StudioShell>
    with SingleTickerProviderStateMixin, WidgetsBindingObserver {
  AppLocalizations get l => AppLocalizations.of(context);

  static const MethodChannel _channel = MethodChannel('yinhe/surface');

  int? _textureId;
  Size? _reportedPixelSize;

  bool _showHome = true;
  int _viewMode = kViewArrangement;
  String? _fileName;

  /// AR 中点击进入 PR 的轨道（自动化栏/聚焦用）。
  int? _activeTrack;

  bool _playing = false;

  /// 时间码显示模式：false=秒数，true=小节.拍.tick（默认小节格式）。
  bool _timecodeBar = true;

  /// 音色库表修订号（设置页监听：选择/增删后重读列表）。
  final ValueNotifier<int> _sfRevision = ValueNotifier(0);

  /// 全曲音符数（顶栏标题展示）。
  int _noteCount = 0;

  /// 拍号分段（标尺/模板行/时间码按分段排版；随文档刷新）。
  final ValueNotifier<BarBeatGrid?> _grid = ValueNotifier(null);

  /// 顶栏实时 BPM / 拍号文本（跟随播放位置，原生事件优先）。
  String _liveBpmText = '120';
  String _liveSigText = '4/4';

  /// 本工程默认 BPM / 拍号（无原生事件时的回退）。
  double _projectBpm = 120;
  int _projectSigNum = 4;
  int _projectSigDen = 4;

  /// 时间码文本（每帧刷新，只重建文本本身）。
  final ValueNotifier<String> _timecodeText = ValueNotifier('0:00.000');

  /// 设置页浮层（从主页进入）。
  bool _showSettings = false;

  List<RecentItem> _recent = const [];

  /// 自动化栏数据（当前轨道，仅 PB/CC/RPN）。
  List<AutomationLaneData> _autoLanes = const [];

  /// 力度条数据：(tick, length, velocity01, track)。
  List<(double, double, double, int)> _velocityBars = const [];

  /// 收起自动化栏（最大化画布）。
  bool _autoCollapsed = false;

  /// 当前工具（PR 5 种；AR 2 种；索引对应 `toolDefs`）。
  int _toolPr = 0;
  int _toolAr = 0;

  /// 量化档位显示（桌面：每文档瞬态，默认自动）。
  String _quantizeLabel = '自动';

  /// “显示轨道”过滤（-1 = 全部轨道）。
  int _displayTrack = -1;

  /// PR 顶部「全部轨道」开关：开 = 不按当前编辑轨过滤（只影响 PR）。
  bool _showAllTracks = true;

  /// 歌曲标题（meta.name；空 = 未设置）。
  String _songTitle = '';

  /// AR 折叠态自动化竖线。
  List<ArAutomationLane> _arAutomation = const [];

  /// 覆盖层（选框/预览）与修订号。
  final ValueNotifier<OverlayData> _overlay = ValueNotifier(OverlayData.empty);
  int _overlayRev = 0;

  /// 文档/选区世代号：变化时重新读取轨道/自动化/力度数据。
  int _docGeneration = 0;

  /// 标尺标签（marker）。
  List<RulerMarker> _markers = const [];

  /// 长按落点竖线（插入/粘贴位置）。
  final ValueNotifier<LongPressIndicator?> _lpIndicator = ValueNotifier(null);
  int _lpToken = 0;

  /// 工程状态位：bit0=可撤销，bit1=可重做，bit2=未保存。
  final ValueNotifier<int> _docState = ValueNotifier(0);
  late final ffi.Pointer<ffi.Int32> _statePtr;

  final ValueNotifier<double> _playhead = ValueNotifier<double>(-1);
  final ValueNotifier<List<double>> _snap = ValueNotifier<List<double>>(
    List<double>.filled(12, 0.0),
  );
  final ValueNotifier<List<TrackRow>> _tracks = ValueNotifier<List<TrackRow>>(
    const [],
  );
  final ValueNotifier<int> _dragFrom = ValueNotifier<int>(-1);
  final ValueNotifier<int> _dragTo = ValueNotifier<int>(-1);

  late final Ticker _ticker;
  late final ffi.Pointer<ffi.Float> _snapPtr;

  bool _dragging = false;

  /// AR 长按音轨期间手指是否离开过起始行（移动过又放回原位时不弹菜单）。
  bool _trackDragMoved = false;

  /// 鼠标右键拖拽的 pointer id（无论当前工具，都按手形平移）。
  int? _panPointer;

  /// PR 选框长按：圆环进度（500ms）；完成后进入移动态，未移动则弹菜单。
  Offset? _lpPos;
  DateTime? _lpStartTime;
  bool _lpMoved = false;

  /// 长按微调中的落点 tick（顶部时间码临时跟随手指）。
  double? _lpPreviewTick;

  /// 本次长按是 AR 音轨面板（Rust 不进入 PENDING，圆环由 Dart 自行计时）。
  bool _lpPanelRing = false;

  /// 刚保存成功：顶栏保存按钮临时显示对勾（2 秒）。
  bool _saveDone = false;
  Timer? _saveDoneTimer;

  /// 保存进行中：顶栏保存按钮显示加载圆环。
  bool _saving = false;
  Timer? _savingTimer;

  /// 正在等待压缩包打开结果（轮询 Rust 状态）。
  bool _archivePolling = false;
  bool _archiveBusy = false;
  final ValueNotifier<double> _lpProgress = ValueNotifier(0);
  final ValueNotifier<Offset?> _lpPosNotifier = ValueNotifier(null);
  final ValueNotifier<bool> _moveDashed = ValueNotifier(false);

  /// 触控板捏合的上一次累计缩放（用于换算增量）。
  double _panZoomScale = 1.0;

  /// 帧计数（时间码/工程状态降频）。
  int _frameCounter = 0;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _snapPtr = calloc<ffi.Float>(13);
    _statePtr = calloc<ffi.Int32>(3);
    _channel.setMethodCallHandler(_onNativeCall);
    _init();
    _ticker = createTicker((_) {
      // 帧驱动同步：先让 wgpu 渲染当前状态的一帧（同步等待完成），再用同一状态
      // 刷新浮层，保证纹理与 Flutter 浮层出自同一份场景数据、同一帧合成。
      if (!_showHome) {
        // 拖拽边缘自动滚动：先滚动视图，再渲染这一帧。
        rust.dragTick();
        rust.renderFrame();
      }
      _playhead.value = rust.playheadX();
      final snap = _readSnapshot();
      if (!_sameSnapshot(snap)) {
        _snap.value = snap;
      }
      _readOverlay();
      // 选框状态驱动（Rust 计时）：移动态画虚线；长按中更新圆环进度。
      final selState = rust.selectionState();
      if (_moveDashed.value != (selState == 2)) {
        _moveDashed.value = selState == 2;
      }
      if (selState == 3) {
        // 长按完成：指示线出现并跟随手指微调（松手前可移动位置）。
        if (_lpProgress.value != 0) {
          _lpProgress.value = 0;
        }
        final tick = rust.longPressTick();
        final track = rust.longPressTrack();
        _lpPreviewTick = tick;
        final cur = _lpIndicator.value;
        if (cur == null || cur.tick != tick || cur.track != track) {
          _lpIndicator.value = LongPressIndicator(
            tick: tick,
            track: track,
            pr: _viewMode == kViewPianoroll,
          );
        }
      } else if (selState == 2) {
        // 已转移动态：清掉圆环（虚线由 moving 分支绘制）。
        _lpPreviewTick = null;
        if (_lpProgress.value != 0) {
          _lpProgress.value = 0;
        }
      } else if (_lpStartTime != null && (selState == 1 || _lpPanelRing)) {
        // 前 200ms 不显示圆环（防普通点击遮挡内容），总时长仍为 500ms。
        final ms = DateTime.now().difference(_lpStartTime!).inMilliseconds;
        _lpProgress.value = ms <= 200 ? 0 : ((ms - 200) / 300).clamp(0.0, 1.0);
      }
      // 工程状态/时间码降频（每 ~66ms）：减少音频锁与历史栈分配开销。
      _frameCounter++;
      if (_frameCounter % 4 == 0) {
        final generation = rust.documentGeneration();
        if (generation != _docGeneration) {
          _docGeneration = generation;
          _refreshDocumentMeta();
        }
        _readDocState();
        _updateTimecode();
        _pollArchive();
      }
      // 保持连续帧：Rust 侧拖拽/边缘滚动/渲染都由帧驱动（手指静止时也要跑）。
      if (!_showHome) {
        SchedulerBinding.instance.scheduleFrame();
      }
    })..start();
    _recent = _readRecent();
    _loadLanguage();
    rust.setQuantize(1, 1, 1); // 默认自动（与桌面一致）
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _ticker.dispose();
    _timecodeText.dispose();
    _playhead.dispose();
    _snap.dispose();
    _tracks.dispose();
    _dragFrom.dispose();
    _dragTo.dispose();
    _lpProgress.dispose();
    _lpPosNotifier.dispose();
    _moveDashed.dispose();
    _lpIndicator.dispose();
    _overlay.dispose();
    _sfRevision.dispose();
    _grid.dispose();
    _saveDoneTimer?.cancel();
    _savingTimer?.cancel();
    _docState.dispose();
    calloc.free(_snapPtr);
    calloc.free(_statePtr);
    super.dispose();
  }

  /// 从桌面/后台返回：重新应用沉浸式全屏（系统可能已重置）。
  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) {
      SystemChrome.setEnabledSystemUIMode(SystemUiMode.immersiveSticky);
      // 从 SAF 选择器返回：保存流程要么已完成（onProjectSaved 会给对勾），
      // 要么被取消 —— 这里清掉"保存中"圆环，避免一直转。
      if (_saving) {
        setState(() => _saving = false);
      }
    } else {
      // 切后台/失去焦点时触点 up 可能收不到：清空，避免残留成“幽灵手指”。
      rust.resetTouch();
    }
  }

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
  int get _tool => _viewMode == kViewPianoroll ? _toolPr : _toolAr;

  /// 把当前视图的工具同步给 Rust。
  void _pushTool() {
    final list = _viewMode == kViewPianoroll ? toolPrAllowed : toolArAllowed;
    final t = list.contains(_tool) ? _tool : list.first;
    rust.setTool(t);
  }

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

  /// 打开工程设置对话框（歌名/艺术家/PPQ/简介）。
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

  /// 导出 .ying 工程：Rust 写临时文件 → Kotlin 复制到用户选择的位置。
  /// 是否压缩包扩展名（与 Rust/桌面同一份列表）。
  static bool _isArchivePath(String path) {
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

  /// 文档加载是后台线程：延迟刷新轨道/最近列表。
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
  Future<void> _ensureSurface() async {
    await _channel.invokeMethod<void>('ensureSurface');
  }

  Future<void> _pickMidi() async {
    await _channel.invokeMethod<void>('openFilePicker');
  }

  Future<void> _pickSoundFont() async {
    await _channel.invokeMethod<void>('openSoundFontPicker');
  }

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
  void _rememberProject(String name) {
    _channel.invokeMethod<void>('rememberProject', {'name': name});
  }

  void _openSettings() {
    setState(() => _showSettings = true);
  }

  /// 读取当前轨道的自动化 lane（仅 PB/CC/RPN）。
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

  /// 读取当前轨道的力度条数据（每音符一条）。
  void _loadVelocity() {
    final rows = _tracks.value;
    if (rows.isEmpty) {
      setState(() => _velocityBars = const []);
      return;
    }
    final ptr = rust.velocityJson();
    var bars = const <(double, double, double, int)>[];
    if (ptr != ffi.nullptr) {
      try {
        final list = jsonDecode(ptr.toDartString()) as List<dynamic>;
        bars = [
          for (final e in list)
            (
              ((e as List<dynamic>)[0] as num).toDouble(),
              (e[1] as num).toDouble(),
              (e[2] as num).toDouble(),
              (e[3] as num).toInt(),
            ),
        ];
      } catch (_) {
        bars = const [];
      } finally {
        rust.freeString(ptr);
      }
    }
    if (!mounted) {
      return;
    }
    setState(() => _velocityBars = bars);
  }

  /// 读取 AR 折叠态自动化竖线（所有可见轨）。
  void _loadArAutomation() {
    final ptr = rust.arAutomationJson();
    var lanes = const <ArAutomationLane>[];
    if (ptr != ffi.nullptr) {
      try {
        final list = jsonDecode(ptr.toDartString()) as List<dynamic>;
        lanes = [
          for (final e in list)
            ArAutomationLane.fromJson(e as Map<String, dynamic>),
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
    setState(() => _arAutomation = lanes);
  }

  /// 读取歌曲标题（meta.name）。
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

  static String _fmtBpm(double v) =>
      v == v.roundToDouble() ? '${v.round()}' : v.toStringAsFixed(1);

  /// 编辑本工程默认 BPM（无原生 Tempo 事件时生效）。
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
  static int _denPow(int den) {
    var pow = 0;
    var v = den;
    while (v > 1 && pow < 8) {
      v >>= 1;
      pow++;
    }
    return pow;
  }

  void _setAutoCollapsed(bool collapsed) {
    setState(() => _autoCollapsed = collapsed);
  }

  // ── 走带 ──

  void _togglePlay() {
    final next = !_playing;
    setState(() => _playing = next);
    rust.transport(next ? 0 : 1);
    _updateTimecode();
  }

  void _stopPlayback() {
    rust.transport(2);
    _updateTimecode();
    setState(() => _playing = false);
  }

  /// 每帧刷新时间码文本（只触发文本重建，不动整棵布局）。
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

  double get _ppq {
    final v = _snap.value[8];
    return v > 0 ? v : 480;
  }

  // ── 触摸 / 手势 ──

  bool _isRightMouse(PointerEvent e) =>
      e.kind == PointerDeviceKind.mouse &&
      (e.buttons & kSecondaryMouseButton) != 0;

  /// 右键按下：取消长按/编辑拖拽，强制进入平移（手形语义）。
  void _beginRightPan(PointerDownEvent e) {
    _panPointer = e.pointer;
    _cancelLongPressWatch();
    _sendPanTouch(3, e.localPosition, e.pointer);
  }

  /// 右键平移事件（3=down 4=move 5=up）。
  void _sendPanTouch(int kind, Offset p, int pointer) {
    rust.touch(kind, p.dx, p.dy, pointer);
    SchedulerBinding.instance.scheduleFrame();
  }

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

  /// 长按完成但未移动：选区操作菜单。
  void _showSelectionMenu() {
    rust.resetTouch();
    showSelectionMenu(
      context,
      onCopy: () {
        if (rust.copySelection() == 1) {
          showSnack(context, l.copied, duration: const Duration(seconds: 1));
        }
      },
    );
  }

  /// 空白处长按完成：粘贴/插入菜单（都在长按位置操作）。
  void _showPasteMenu(Offset? pos, LongPressContext? ctx) {
    if (pos == null) {
      return;
    }
    rust.resetTouch();
    final token = ++_lpToken;
    _lpIndicator.value = ctx == null
        ? null
        : LongPressIndicator(tick: ctx.tick, track: ctx.track, pr: ctx.pr);
    showEmptyLongPressMenu(
      context,
      canPaste: rust.clipboardEmpty() == 0,
      onPaste: () {
        if (rust.pasteSelection(pos.dx) == 1) {
          _refreshDocumentMeta();
        }
      },
      onInsert: ctx == null ? null : () => _runInsertFlow(ctx),
    ).whenComplete(() {
      if (token == _lpToken) {
        _lpIndicator.value = null;
      }
    });
  }

  /// 长按"插入"完整流程：选目标 →（自定义号）→ 值输入 → 插入。
  Future<void> _runInsertFlow(LongPressContext ctx) async {
    _lpToken++;
    try {
      final choice = await showInsertMenu(context, conductor: ctx.conductor);
      if (!mounted || choice == null) {
        return;
      }
      // Conductor 全局事件：拍号 / 调号 / 标签。
      if (choice.kind == 4) {
        final sig = await showInsertTimeSigDialog(context);
        if (!mounted || sig == null) {
          return;
        }
        if (rust.insertTimeSig(ctx.tick, sig.$1, _denPow(sig.$2)) == 1) {
          _refreshDocumentMeta();
        }
        return;
      }
      if (choice.kind == 5) {
        final key = await showInsertKeySigDialog(context);
        if (!mounted || key == null) {
          return;
        }
        if (rust.insertKeySig(ctx.tick, key.$1, key.$2) == 1) {
          _refreshDocumentMeta();
        }
        return;
      }
      if (choice.kind == 6) {
        final text = await showInsertTextDialog(
          context,
          title: l.marker,
          initial: 'Marker',
        );
        if (!mounted || text == null) {
          return;
        }
        final t = text.trim().toNativeUtf8();
        try {
          if (rust.markerInsert(ctx.tick, t) == 1) {
            _refreshDocumentMeta();
          }
        } finally {
          calloc.free(t);
        }
        return;
      }
      var kind = choice.kind;
      var number = choice.number;
      if (choice.custom) {
        final n = await showInsertNumberDialog(
          context,
          title: kind == 2 ? l.customCcTitle : l.customRpnTitle,
          min: 0,
          max: kind == 2 ? 127 : 16383,
          initial: kind == 2 ? 1 : 3,
        );
        if (!mounted || n == null) {
          return;
        }
        number = n;
      }
      final maxValue = insertDisplayMax(kind, number);
      final minValue = kind == 0 ? 1 : 0;
      final current = rust.automationDisplayValue(
        kind,
        number,
        ctx.tick,
        ctx.track,
      );
      final initial = current.isNaN
          ? insertDefaultValue(kind, number).clamp(minValue, maxValue)
          : current.round().clamp(minValue, maxValue);
      final quick = kind == 2 && number == 64
          ? const [('关闭', 0), ('开启', 127)]
          : const <(String, int)>[];
      final v = await showInsertValueDialog(
        context,
        title: insertValueTitle(l, kind, number),
        min: minValue,
        max: maxValue,
        initial: initial,
        quick: quick,
      );
      if (!mounted || v == null) {
        return;
      }
      if (rust.insertAutomation(
            kind,
            number,
            ctx.tick,
            ctx.track,
            v.toDouble(),
          ) ==
          1) {
        _refreshDocumentMeta();
      }
    } finally {
      _lpIndicator.value = null;
    }
  }

  /// 鼠标滚轮 / 触控板滚动：Ctrl（或 Cmd）+ 滚轮缩放（键盘区缩放音高轴，
  /// 其余缩放时间轴），普通滚轮平移时间轴。
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

  /// 列表末尾占位行：添加音轨。
  void _showAddTrackDialog() {
    final rows = _tracks.value;
    final last = rows.isNotEmpty ? rows.last : null;
    rust.resetTouch();
    showDialog<void>(
      context: context,
      builder: (_) => AddTracksDialog(
        initialPort: last?.port ?? 0,
        initialChannel: last?.channel ?? 0,
        onConfirm: (count, port, channel, cycle) {
          if (rust.addTracks(count, port, channel, cycle ? 1 : 0) == 1) {
            _refreshDocumentMeta();
          }
        },
      ),
    );
  }

  /// AR：长按拖动轨道排序。
  void _handleLongPressStart(Offset p) {
    if (_viewMode != kViewArrangement) {
      return;
    }
    // 长按成立：圆环完成，收起长按 UI（后续由轨道拖动/松手菜单接管）。
    _clearLongPressUi();
    final idx = _rowAt(p);
    if (idx == null || idx >= _tracks.value.length) {
      return;
    }
    _trackDragMoved = false;
    setState(() {
      _dragFrom.value = idx;
      _dragTo.value = idx;
      _dragging = true;
    });
  }

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

  /// AR 长按音轨（未拖动）：音轨信息菜单（含删除）。
  void _showTrackMenu(int track) {
    final rows = _tracks.value;
    final master = track < 0 || track >= rows.length || rows[track].master;
    rust.resetTouch();
    showTrackMenu(
      context,
      onEdit: () => _editTrackInfo(track),
      onDelete: master ? null : () => _deleteTrack(track),
      onEvents: () => _showEventList(track),
    );
  }

  /// 打开事件列表（音符/自动化/全局事件穿插）。
  void _showEventList(int track) {
    rust.resetTouch();
    showEventListDialog(
      context,
      grid: _grid,
      onJump: _jumpToEvent,
      onChanged: _refreshDocumentMeta,
    );
  }

  /// 事件列表「前往该地点」：选中该轨、光标跳到 tick、进入 PR。
  void _jumpToEvent(int track, double tick) {
    _showAllTracks = false;
    _selectEditTrack(track);
    rust.jumpToTick(tick);
    _setView(kViewPianoroll);
  }

  /// 删除音轨（连同音符；一步一 undo）。
  void _deleteTrack(int track) {
    if (rust.deleteTrack(track) != 1) {
      return;
    }
    setState(() {
      final active = _activeTrack;
      if (active != null) {
        if (active == track) {
          _activeTrack = null;
        } else if (active > track) {
          _activeTrack = active - 1;
        }
      }
      if (_displayTrack == track) {
        _displayTrack = _showAllTracks ? -1 : _activeTrack ?? -1;
      } else if (_displayTrack > track) {
        _displayTrack -= 1;
      }
      _dragFrom.value = -1;
      _dragTo.value = -1;
    });
    rust.setActiveTrack(_activeTrack ?? -1);
    rust.setDisplayTrack(_displayTrack);
    _refreshDocumentMeta();
    _loadAutomation();
    _loadVelocity();
  }

  /// 编辑音轨信息（名称/端口/通道/本通道乐器）。
  void _editTrackInfo(int track) {
    final rows = _tracks.value;
    if (track < 0 || track >= rows.length) {
      return;
    }
    rust.resetTouch();
    final row = rows[track];
    var instruments = const <(int, int)?>[];
    final instPtr = rust.channelInstrumentsJson();
    if (instPtr != ffi.nullptr) {
      try {
        final list = jsonDecode(instPtr.toDartString()) as List<dynamic>;
        instruments = [
          for (final e in list)
            e == null
                ? null
                : (
                    ((e as List<dynamic>)[0] as num).toInt(),
                    (e[1] as num).toInt(),
                  ),
        ];
      } catch (_) {
        instruments = const [];
      } finally {
        rust.freeString(instPtr);
      }
    }
    showDialog<void>(
      context: context,
      builder: (_) => TrackInfoDialog(
        initialName: row.name,
        initialPort: row.port,
        initialChannel: row.channel,
        isMaster: row.master,
        channelInstruments: instruments,
        onSave: (name, port, channel, bank, program) {
          final p = name.toNativeUtf8();
          var ok = 0;
          try {
            ok = rust.setTrackInfo(
              track,
              p,
              port,
              channel,
              bank ?? 0,
              program ?? 0,
              bank != null ? 1 : 0,
            );
          } finally {
            calloc.free(p);
          }
          if (ok == 1) {
            _refreshDocumentMeta();
          }
        },
      ),
    );
  }

  void _setView(int mode) {
    if (_viewMode == mode) {
      return;
    }
    rust.setViewMode(mode);
    setState(() => _viewMode = mode);
    _pushTool();
  }

  // ── 布局 ──

  @override
  Widget build(BuildContext context) {
    final id = _textureId;
    if (id == null) {
      return const Scaffold(
        backgroundColor: Color(0xFFF0F0F4),
        body: Center(child: CircularProgressIndicator()),
      );
    }
    return Scaffold(
      backgroundColor: YinheTheme.bg,
      body: SafeArea(
        child: Stack(
          children: [
            Column(
              children: [
                // 顶部栏占位（栏本体在外层 Stack 顶层，保证绘制优先级最高）。
                const SizedBox(height: 42),
                Expanded(
                  child: LayoutBuilder(
                    builder: (context, constraints) {
                      final size = constraints.biggest;
                      // 内容画布 = 画布区减去顶部标尺条（Rust 坐标系从标尺下方开始）。
                      final contentH = (size.height - RulerPainter.height)
                          .clamp(1.0, double.infinity);
                      final dpr = MediaQuery.devicePixelRatioOf(context);
                      final pixelW = (size.width * dpr).round();
                      final pixelH = (contentH * dpr).round();
                      if (_reportedPixelSize?.width != pixelW ||
                          _reportedPixelSize?.height != pixelH) {
                        _reportedPixelSize = Size(
                          pixelW.toDouble(),
                          pixelH.toDouble(),
                        );
                        _channel.invokeMethod<void>('setSize', {
                          'w': pixelW,
                          'h': pixelH,
                          'lw': size.width,
                          'lh': contentH,
                        });
                      }
                      return Stack(
                        children: [
                          Positioned(
                            left: 0,
                            right: 0,
                            top: RulerPainter.height,
                            bottom: 0,
                            child: GestureDetector(
                              behavior: HitTestBehavior.translucent,
                              onTapUp: (d) => _handleCanvasTap(d.localPosition),
                              onLongPressStart: (d) =>
                                  _handleLongPressStart(d.localPosition),
                              onLongPressMoveUpdate: (d) =>
                                  _handleLongPressMove(d.localPosition),
                              onLongPressEnd: (_) => _handleLongPressEnd(),
                              child: Listener(
                                behavior: HitTestBehavior.opaque,
                                onPointerDown: (e) {
                                  if (_isRightMouse(e)) {
                                    _beginRightPan(e);
                                  } else {
                                    _sendTouch(0, e.localPosition, e.pointer);
                                  }
                                },
                                onPointerMove: (e) {
                                  if (_panPointer == e.pointer) {
                                    _sendPanTouch(
                                      4,
                                      e.localPosition,
                                      e.pointer,
                                    );
                                  } else {
                                    _sendTouch(1, e.localPosition, e.pointer);
                                  }
                                },
                                onPointerUp: (e) {
                                  if (_panPointer == e.pointer) {
                                    _panPointer = null;
                                    _sendPanTouch(
                                      5,
                                      e.localPosition,
                                      e.pointer,
                                    );
                                  } else {
                                    _sendTouch(2, e.localPosition, e.pointer);
                                  }
                                },
                                onPointerCancel: (e) {
                                  if (_panPointer == e.pointer) {
                                    _panPointer = null;
                                    _sendPanTouch(
                                      5,
                                      e.localPosition,
                                      e.pointer,
                                    );
                                  } else {
                                    _sendTouch(2, e.localPosition, e.pointer);
                                  }
                                },
                                onPointerSignal: _handlePointerSignal,
                                onPointerPanZoomStart: (_) =>
                                    _panZoomScale = 1.0,
                                onPointerPanZoomUpdate: _handlePanZoomUpdate,
                                onPointerPanZoomEnd: (_) => _panZoomScale = 1.0,
                                child: Texture(textureId: id),
                              ),
                            ),
                          ),
                          // 层级：音符（Texture）→ 演奏指示线 → 音轨面板 → 时间标尺 → 自动化栏。
                          if (!_showHome)
                            Positioned.fill(
                              child: IgnorePointer(
                                child: CustomPaint(
                                  painter: PlayheadPainter(_playhead),
                                ),
                              ),
                            ),
                          if (!_showHome)
                            Positioned(
                              left: 0,
                              right: 0,
                              top: RulerPainter.height,
                              bottom: 0,
                              child: IgnorePointer(
                                child: CustomPaint(
                                  painter: LabelsPainter(
                                    snap: _snap,
                                    tracks: _tracks,
                                    dragFrom: _dragFrom,
                                    dragTo: _dragTo,
                                    arAutomation: _arAutomation,
                                  ),
                                ),
                              ),
                            ),
                          if (!_showHome)
                            Positioned(
                              left: 0,
                              right: 0,
                              top: RulerPainter.height,
                              bottom: 0,
                              child: IgnorePointer(
                                child: CustomPaint(
                                  painter: OverlayPainter(
                                    _overlay,
                                    _moveDashed,
                                  ),
                                ),
                              ),
                            ),
                          if (!_showHome)
                            Positioned(
                              left: 0,
                              right: 0,
                              top: 0,
                              height: RulerPainter.height,
                              child: IgnorePointer(
                                child: CustomPaint(
                                  painter: RulerPainter(_snap, _grid),
                                ),
                              ),
                            ),
                          if (!_showHome)
                            Positioned(
                              left: 0,
                              right: 0,
                              top: 0,
                              height: RulerPainter.height,
                              child: MarkerLayer(
                                snap: _snap,
                                grid: _grid,
                                markers: _markers,
                                onChanged: _refreshDocumentMeta,
                              ),
                            ),
                          if (!_showHome && _viewMode == kViewPianoroll)
                            if (!_autoCollapsed)
                              AutomationPanel(
                                snap: _snap,
                                lanes: _autoLanes,
                                velocity: _velocityBars,
                                trackColors: [
                                  for (final t in _tracks.value) t.color,
                                ],
                                activeTrack: _activeTrack ?? 0,
                                onCollapsed: _setAutoCollapsed,
                                onReload: _loadAutomation,
                              )
                            else
                              AutoExpandButton(
                                onTap: () => _setAutoCollapsed(false),
                              ),
                          // 长按圆环/落点竖线最后画：盖在音轨面板与自动化栏之上。
                          if (!_showHome)
                            Positioned(
                              left: 0,
                              right: 0,
                              top: RulerPainter.height,
                              bottom: 0,
                              child: IgnorePointer(
                                child: CustomPaint(
                                  painter: SelectionLongPressPainter(
                                    _lpProgress,
                                    _lpPosNotifier,
                                    _moveDashed,
                                    _overlay,
                                  ),
                                ),
                              ),
                            ),
                          if (!_showHome)
                            Positioned(
                              left: 0,
                              right: 0,
                              top: RulerPainter.height,
                              bottom: 0,
                              child: IgnorePointer(
                                child: CustomPaint(
                                  painter: LongPressIndicatorPainter(
                                    _snap,
                                    _lpIndicator,
                                  ),
                                ),
                              ),
                            ),
                        ],
                      );
                    },
                  ),
                ),
              ],
            ),
            if (!_showHome)
              Positioned(
                left: 0,
                right: 0,
                top: 0,
                height: 42,
                child: _buildTopBar(),
              ),
            if (_showHome)
              HomeView(
                recent: _recent,
                onPickMidi: _pickMidi,
                onNewProject: _newProject,
                onSaveProjectAs: () => _saveProject(saveAs: true),
                onOpenRecent: _openRecent,
                onOpenSettings: _openSettings,
                onBack: () => setState(() => _showHome = false),
              ),
            if (_showSettings)
              SettingsView(
                revision: _sfRevision,
                onPickSoundFont: _pickSoundFont,
                onBack: () => setState(() => _showSettings = false),
              ),
          ],
        ),
      ),
    );
  }

  /// 上侧栏（实现见 top_bar.dart）。
  Widget _buildTopBar() {
    return TopBar(
      isPr: _viewMode == kViewPianoroll,
      playing: _playing,
      timecodeText: _timecodeText,
      quantizeLabel: _quantizeLabel,
      ppq: _ppq,
      tool: _tool,
      tracks: _tracks.value,
      activeTrack: _activeTrack ?? -1,
      showAllTracks: _showAllTracks,
      songTitle: _songTitle,
      fileName: _fileName,
      docState: _docState,
      onHome: () => setState(() => _showHome = true),
      onBack: () => _setView(kViewArrangement),
      onTogglePlay: _togglePlay,
      onStop: _stopPlayback,
      onToggleTimecode: () {
        setState(() => _timecodeBar = !_timecodeBar);
        _updateTimecode();
      },
      onQuantize: (v) {
        setState(() => _quantizeLabel = v);
        _pushQuantize(v);
      },
      onTool: _selectTool,
      onUndo: _undo,
      onRedo: _redo,
      onSelectTrack: _selectEditTrack,
      onToggleAllTracks: _toggleAllTracks,
      onProjectSettings: _openProjectSettings,
      onSave: () => _saveProject(),
      saveDone: _saveDone,
      saving: _saving,
      noteCount: _noteCount,
      bpmText: _liveBpmText,
      sigText: _liveSigText,
      onEditBpm: _editDefaultBpm,
      onEditSig: _editDefaultTimeSig,
    );
  }

  /// 选择当前编辑音轨：PR 顶部显示它；「全部轨道」关闭时只绘制该轨。
  void _selectEditTrack(int track) {
    setState(() {
      _activeTrack = track;
      if (!_showAllTracks) {
        _displayTrack = track;
      }
    });
    rust.setActiveTrack(track);
    if (!_showAllTracks) {
      rust.setDisplayTrack(track);
    }
    _loadAutomation();
    _loadVelocity();
  }

  /// 「全部轨道」开关（仅影响 PR 绘制；AR 始终按自身显隐）。
  void _toggleAllTracks() {
    setState(() {
      _showAllTracks = !_showAllTracks;
      _displayTrack = _showAllTracks ? -1 : (_activeTrack ?? -1);
    });
    rust.setDisplayTrack(_displayTrack);
  }

  /// 撤销一步（含快照/音频刷新）。
  void _undo() {
    if (rust.undo() == 1) {
      _refreshDocumentMeta();
    }
  }

  /// 重做一步（含快照/音频刷新）。
  void _redo() {
    if (rust.redo() == 1) {
      _refreshDocumentMeta();
    }
  }
}
