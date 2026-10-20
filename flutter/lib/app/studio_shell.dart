import 'dart:async';
import 'dart:convert';
import 'dart:ffi' as ffi;

import 'package:ffi/ffi.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';

import '../core/ffi.dart' as rust;
import '../core/snack.dart';
import '../dialogs/add_tracks_dialog.dart';
import '../widgets/automation_panel.dart';
import '../menus/canvas_menus.dart';
import '../widgets/home_view.dart';
import '../dialogs/insert_flow.dart';
import '../l10n.dart';
import '../l10n/app_localizations.dart';
import '../painters/long_press_indicator_painter.dart';
import '../core/models.dart';
import '../widgets/ruler_markers.dart';
import '../dialogs/archive_dialogs.dart';
import '../dialogs/event_list_dialog.dart';
import '../dialogs/export_dialogs.dart';
import '../core/bar_beat.dart';
import '../dialogs/project_defaults_dialog.dart';
import '../dialogs/project_dialog.dart';
import '../painters/painters.dart';
import '../widgets/settings_view.dart';
import '../core/theme.dart';
import '../core/tools.dart';
import '../dialogs/track_info_dialog.dart';
import '../widgets/top_bar.dart';

part 'shell_core.dart';
part 'shell_document.dart';
part 'shell_transport.dart';
part 'shell_input.dart';
part 'shell_menus.dart';
part 'shell_build.dart';

const MethodChannel _channel = MethodChannel('yinhe/surface');

const int kViewArrangement = 0;
const int kViewPianoroll = 1;

/// 工作台壳：画布常驻，主页为覆盖浮层，上侧栏为走带控制。
class StudioShell extends StatefulWidget {
  const StudioShell({super.key});

  @override
  State<StudioShell> createState() => _StudioShellState();
}

abstract class _ShellBase extends State<StudioShell>
    with SingleTickerProviderStateMixin, WidgetsBindingObserver {
  AppLocalizations get l => AppLocalizations.of(context);

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

  // ── 跨模块方法（由各 mixin 实现） ──
  void _beginRightPan(PointerDownEvent e);
  void _cancelLongPressWatch();
  int _denPow(int den);
  Future<void> _editDefaultBpm();
  Future<void> _editDefaultTimeSig();
  Future<void> _ensureSurface();
  Future<void> _exportAudio();
  Future<void> _exportMidi();
  void _flashSaved();
  String _fmtBpm(double v);
  void _handleCanvasTap(Offset p);
  void _handleLongPressEnd();
  void _handleLongPressMove(Offset p);
  void _handleLongPressStart(Offset p);
  void _handlePanZoomUpdate(PointerPanZoomUpdateEvent e);
  void _handlePointerSignal(PointerSignalEvent e);
  bool _isArchivePath(String path);
  bool _isRightMouse(PointerEvent e);
  void _loadArAutomation();
  void _loadAutomation();
  void _loadProjectDefaults();
  void _loadSongTitle();
  void _loadTimeSigSegments();
  void _loadVelocity();
  void _newProject();
  void _openProjectSettings();
  void _openRecent(RecentItem item);
  void _openSettings();
  Future<void> _pickMidi();
  Future<void> _pickSoundFont();
  Future<void> _pickSfzFolder();
  double get _ppq;
  void _pushQuantize(String label);
  void _refreshDocumentMeta();
  void _rememberProject(String name);
  Future<void> _saveProject({bool saveAs});
  void _selectEditTrack(int track);
  void _selectTool(int tool);
  void _sendPanTouch(int kind, Offset p, int pointer);
  void _sendTouch(int kind, Offset p, int pointer);
  void _setAutoCollapsed(bool collapsed);
  void _setView(int mode);
  void _showAddTrackDialog();
  void _showPasteMenu(Offset? pos, LongPressContext? ctx);
  void _showSelectionMenu();
  void _showTrackMenu(int track);
  void _stopPlayback();
  void _togglePlay();
  int get _tool;
  void _updateTimecode();
}

class _StudioShellState extends _ShellBase
    with
        _ShellCore,
        _ShellDocument,
        _ShellTransport,
        _ShellInput,
        _ShellMenus,
        _ShellBuild {
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
}
