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
import '../core/edit_status.dart';
import '../core/multi_finger_gesture.dart';
import '../core/pinch_zoom.dart';
import '../dialogs/add_tracks_dialog.dart';
import '../dialogs/batch_edit_dialog.dart';
import '../dialogs/inputs.dart';
import '../widgets/automation_panel.dart';
import '../menus/canvas_menus.dart';
import '../widgets/home_view.dart';
import '../dialogs/insert_flow.dart';
import '../l10n.dart';
import '../l10n/app_localizations.dart';
import '../painters/long_press_indicator_painter.dart';
import '../core/models.dart';
import 'system_bars.dart';
import '../core/ui_settings.dart';
import '../widgets/ruler_markers.dart';
import '../dialogs/archive_dialogs.dart';
import '../dialogs/event_list_dialog.dart';
import '../dialogs/export_dialogs.dart';
import '../dialogs/unsaved_dialog.dart';
import '../dialogs/progress_dialog.dart';
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

  /// AM 面板独立 wgpu 纹理。
  int? _amTextureId;
  Size? _reportedPixelSize;

  /// AM 独立纹理的物理尺寸（布局层上报，和 PR 的 setSize 同一策略）。
  Size? _reportedAmSize;

  /// 本帧待上报的纹理尺寸：布局阶段只登记，帧末统一发送。
  ({int w, int h, double lw, double lh})? _pendingCanvasSize;
  ({int w, int h})? _pendingAmSize;
  bool _surfaceSizeFlushScheduled = false;

  bool _showHome = true;
  int _viewMode = kViewArrangement;
  String? _fileName;

  /// 顶栏取消选区信号：AM 面板据此清空锚点选择。
  int _amDeselectTick = 0;

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

  /// 顶部栏临时读数（力度/自动化/选框拖动）；null = 常规显示。
  final ValueNotifier<EditStatus?> _editStatus = ValueNotifier(null);

  /// 当前是否显示“插入位置”读数（长按/插入菜单期间保持蓝色）。
  bool _insertStatusActive = false;

  /// 长按插入点/菜单打开期间：顶部栏位置显示插入 tick，强调色。
  void _setInsertStatus(double tick) {
    final pos = _formatEditTick(tick);
    if (_insertStatusActive && _editStatus.value?.position == pos) {
      return;
    }
    _insertStatusActive = true;
    _editStatus.value = EditStatus(position: pos);
  }

  void _clearInsertStatus() {
    if (!_insertStatusActive) {
      return;
    }
    _insertStatusActive = false;
    final cur = _editStatus.value;
    if (cur != null && cur.primary == null && cur.secondary == null) {
      _editStatus.value = null;
      // 立即恢复普通时间码，避免状态清掉后还残留插入位置文字。
      _updateTimecode();
    }
  }

  /// 目前顶部栏读数是否由 PR/AR 选框移动驱动。
  bool _selectionStatusActive = false;

  /// 设置页浮层（从主页进入）。
  bool _showSettings = false;

  List<RecentItem> _recent = const [];

  /// 自动化栏数据（当前轨道，仅 PB/CC/RPN）。
  List<AutomationLaneData> _autoLanes = const [];

  /// 力度条数据：(tick, length, velocity01, track)。
  List<(double, double, double, int)> _velocityBars = const [];

  /// 收起自动化栏（最大化画布）。手机默认收起，进入 PR 先看音符。
  bool _autoCollapsed = true;

  /// 自动化栏高度（含顶部拖拽手柄）。
  double _autoPanelHeight = AutomationPanel.defaultHeight;

  /// 当前允许的最大高度（由 LayoutBuilder 每帧写入，避免溢出屏幕）。
  double _autoPanelMaxHeight = AutomationPanel.defaultHeight;

  /// 当前工具（PR 5 种；AR 2 种；索引对应 `toolDefs`）。
  int _toolPr = 0;
  int _toolAr = 0;

  /// 两个工具按钮各自记住的槽位工具。点击某个槽位会把它设为当前工具，
  /// 但不会改写另一个槽位，因此两个工具按钮互相独立。
  int _mainToolPr = 0;
  int _mainToolAr = 0;

  /// 初始备用工具：PR 铅笔、AR 手型；主槽位都是选框。
  int _quickToolPr = defaultQuickToolPr;
  int _quickToolAr = quickToolsAr.last;

  /// 当前高亮的是快捷槽还是主槽。仅最近一次操作的槽位高亮；
  /// 即使两个槽位工具相同，也只有一个按钮是强调色。
  bool _quickSlotActivePr = false;
  bool _quickSlotActiveAr = false;

  /// PR/AR 各自的量化档位（'关闭'/'自动'/'1/4'/'1 tick'…）。
  ///
  /// 默认 AR = 1/1、PR = 1/16；切换视图/加载文档后各自恢复。
  String _quantizeAr = '1/1';
  String _quantizePr = '1/16';

  String get _quantizeLabel =>
      _viewMode == kViewPianoroll ? _quantizePr : _quantizeAr;

  void _setQuantizeLabel(String label) {
    if (_viewMode == kViewPianoroll) {
      _quantizePr = label;
    } else {
      _quantizeAr = label;
    }
  }

  /// “显示轨道”过滤（-1 = 全部轨道）。
  int _displayTrack = -1;

  /// PR 顶部「全部轨道」开关：开 = 不按当前编辑轨过滤（只影响 PR）。
  bool _showAllTracks = true;

  /// 歌曲标题（meta.name；空 = 未设置）。
  String _songTitle = '';

  /// AR 折叠态自动化竖线。

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
  late final ffi.Pointer<ffi.Int64> _dragDtPtr;
  late final ffi.Pointer<ffi.Int32> _dragDkPtr;
  late final ffi.Pointer<ffi.Int32> _dragDtrPtr;

  bool _dragging = false;

  /// AR 长按音轨期间手指是否离开过起始行（移动过又放回原位时不弹菜单）。
  bool _trackDragMoved = false;

  /// AR 内容区双击进入轨道（不用 GestureDetector 双指识别，避免普通 tap 延迟）。
  DateTime? _arrLastTap;
  Offset? _arrLastTapPos;

  /// PR 选框长按：圆环进度（500ms）；完成后进入移动态，未移动则弹菜单。
  Offset? _lpPos;
  Offset? _lpGlobalPos;
  DateTime? _lpStartTime;
  bool _lpMoved = false;

  /// 长按后还没拖动：一拖就转成选框。
  bool _lpMarqueeArmed = false;

  /// 长按微调中的落点 tick（顶部时间码临时跟随手指）。
  double? _lpPreviewTick;

  /// 长按圆环走完后的“可框选”小方框提示。
  final ValueNotifier<Offset?> _marqueeHintPos = ValueNotifier(null);
  final ValueNotifier<double> _marqueeHint = ValueNotifier(0);
  DateTime? _marqueeHintStart;
  bool _marqueeHintFading = false;
  DateTime? _marqueeHintFadeStart;

  /// 选框锚点出现动画（0=隐藏，1=正常大小，允许轻微回弹）。
  final ValueNotifier<double> _handlesScale = ValueNotifier(0);
  bool _handlesShown = false;
  DateTime? _handlesStart;

  /// 删除音符的残影：立即删除模型，只画一小段选中色淡出。
  final List<NoteDeleteFade> _noteFades = [];
  final ValueNotifier<int> _noteFadeTick = ValueNotifier(0);

  /// 刚保存成功：顶栏保存按钮临时显示对勾（2 秒）。
  bool _saveDone = false;
  Timer? _saveDoneTimer;

  /// PR 力度条可见窗口重载：滚动/缩放后延迟一次，避免逐帧扫音符。
  Timer? _velocityReloadTimer;
  double? _velocityViewPpu;
  double? _velocityViewScroll;

  /// Flutter 浮层相对 wgpu Texture 的延迟帧数。
  ///
  /// 所有视图统一延迟一帧：纹理先呈现当前帧，snapshot / overlay / playhead
  /// 晚一帧刷新，保证标尺、键盘标签、选框和播放头与纹理同帧。
  int get _overlayFrameDelay => 1;
  final List<List<double>> _snapDelay = <List<double>>[];
  final List<double> _playheadDelay = <double>[];
  OverlayData? _overlayNext;

  /// 保存进行中：顶栏保存按钮显示加载圆环。
  bool _saving = false;
  Timer? _savingTimer;

  /// “保存并继续”等待 SAF/复制完成；普通保存为 null。
  Completer<bool>? _saveCompleter;

  /// 正在等待压缩包打开结果（轮询 Rust 状态）。
  bool _archivePolling = false;
  bool _archiveBusy = false;

  /// 加载进度框是否已等到 Flutter 首次刷新/首帧渲染；未完成时先不关闭，
  /// 避免“进度条消失后画布还空白、轨道/自动化逐个出现”。
  bool _loadUiSettled = true;
  final ValueNotifier<double> _lpProgress = ValueNotifier(0);
  final ValueNotifier<Offset?> _lpPosNotifier = ValueNotifier(null);
  final ValueNotifier<bool> _moveDashed = ValueNotifier(false);

  /// 帧计数（时间码/工程状态降频）。
  int _frameCounter = 0;

  /// 完成/取消正在等待的保存（“保存并继续”流程）。
  void _finishSaveWait(bool ok) {
    final c = _saveCompleter;
    _saveCompleter = null;
    if (c != null && !c.isCompleted) {
      c.complete(ok);
    }
  }

  // ── 跨模块方法（由各 mixin 实现） ──
  int _denPow(int den);
  Future<void> _editDefaultBpm();
  Future<void> _editDefaultTimeSig();
  Future<void> _ensureSurface();
  Future<void> _exportAudio();
  Future<void> _exportMidi();
  void _flashSaved();
  String _fmtBpm(double v);
  String _formatEditTick(double tick);
  void _handlePointerDown(PointerDownEvent e);
  void _handlePointerMove(PointerMoveEvent e);
  void _handlePointerUp(PointerUpEvent e);
  void _handlePointerCancel(PointerCancelEvent e);
  void _handlePointerSignal(PointerSignalEvent e);
  void _handleTapUp(TapUpDetails d);
  void _handleScaleStart(ScaleStartDetails d);
  void _handleScaleUpdate(ScaleUpdateDetails d);
  void _handleScaleEnd(ScaleEndDetails d);
  void _handleLongPressStart(LongPressStartDetails d);
  void _handleLongPressMove(LongPressMoveUpdateDetails d);
  void _handleLongPressEnd(LongPressEndDetails d);
  void _loadAutomation();
  void _loadProjectDefaults();
  void _loadSongTitle();
  void _loadTimeSigSegments();
  void _loadVelocity();
  void _commitVelocityStroke(List<(double, double)> points);
  void _newProject();
  void _openProjectSettings();
  void _openRecent(RecentItem item);
  void _openSettings();
  Future<void> _pickMidi();
  Future<void> _pickSoundFont();
  Future<void> _openSoundfontArchive(String path);
  double get _ppq;
  void _pushQuantize(String label);
  (int, int)? _parseQuantizeFraction(String label);
  void _readDocState();
  void _refreshDocumentMeta();
  void _resetInteraction();
  void _undo();
  void _redo();
  void _undoFromGesture();
  void _redoFromGesture();
  Future<bool> _saveProject({bool saveAs});
  Future<void> _showLoadProgress();
  Future<bool> _confirmReplace();
  Future<void> _handlePickedFile(String path);
  void _selectEditTrack(int track);
  void _selectTool(int tool);
  void _selectQuickTool(int tool);
  int get _mainTool;
  int get _quickTool;
  bool get _quickSlotActive;
  void _setAutoCollapsed(bool collapsed);
  void _resizeAutoPanel(double dy);
  void _resetAutoPanelHeight();
  void _setView(int mode);
  void _showAddTrackDialog();
  void _showPasteMenu(Offset? pos, LongPressContext? ctx);
  void _handleRulerInsertStart(Offset local, Offset global);
  void _handleRulerInsertMove(Offset local);
  void _handleRulerInsertEnd(Offset local, Offset global);
  void _showPasteMenuAt(
    Offset local,
    Offset global,
    LongPressContext? ctx, {
    bool fromRuler = false,
  });
  void _showSelectionMenu();
  Future<void> _runInsertFlow(LongPressContext ctx);
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
    _dragDtPtr = calloc<ffi.Int64>();
    _dragDkPtr = calloc<ffi.Int32>();
    _dragDtrPtr = calloc<ffi.Int32>();
    _channel.setMethodCallHandler(_onNativeCall);
    showKeyboardLabels.value = rust.showKeyboardLabels() == 1;
    _init().then((_) {
      if (mounted) {
        showKeyboardLabels.value = rust.showKeyboardLabels() == 1;
      }
    });
    _ticker = createTicker((_) {
      // 模态路由（各类对话框）在上层时编辑器被遮罩：跳过 wgpu 帧提交与视图同步。
      // `renderFrame` 会同步等待渲染线程完成 present，键盘弹出/收起与对话框
      // 动画期间继续每帧等待会占住 UI 线程，输入法动画直接掉帧。
      // 帧循环仍保持，对话框关闭后的下一帧自动恢复。
      final coveredByRoute = !(ModalRoute.of(context)?.isCurrent ?? true);
      if (!coveredByRoute) {
        // 帧驱动同步：wgpu 先呈现当前帧，所有视图的 Flutter 浮层统一延迟一帧。
        if (!_showHome) {
          // 拖拽边缘自动滚动：先滚动视图，再渲染这一帧。
          rust.dragTick();
          rust.renderFrame();
        }
        final delay = _overlayFrameDelay;
        final px = rust.playheadX();
        _playheadDelay.add(px);
        final shownPlayhead = _playheadDelay.length > delay
            ? _playheadDelay.removeAt(0)
            : px;
        _playhead.value = shownPlayhead;
        final snap = _readSnapshot();
        _snapDelay.add(snap);
        final shownSnap = _snapDelay.length > delay
            ? _snapDelay.removeAt(0)
            : snap;
        if (!_sameSnapshot(shownSnap)) {
          _snap.value = shownSnap;
        }
        _scheduleVelocityReload();
        _readOverlay();
        // 选框状态驱动：移动态画虚线；长按中更新圆环进度（Flutter long press 推进状态）。
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
          _setInsertStatus(tick);
          // 插入竖线只在时间标尺上显示；主视口长按不再画线。
          if (rust.insertFromRuler() == 1) {
            final cur = _lpIndicator.value;
            if (cur == null || cur.tick != tick || cur.track != track) {
              _lpIndicator.value = LongPressIndicator(
                tick: tick,
                track: track,
                pr: _viewMode == kViewPianoroll,
              );
            }
          } else if (_lpIndicator.value != null) {
            _lpIndicator.value = null;
          }
        } else if (selState == 2) {
          // 已转移动态：清掉圆环（虚线由 moving 分支绘制），顶部栏显示相对增量。
          _lpPreviewTick = null;
          if (_lpProgress.value != 0) {
            _lpProgress.value = 0;
          }
          _updateSelectionStatus();
        } else if (_lpStartTime != null) {
          // 前 200ms 不显示圆环（防普通点击遮挡内容），总时长仍为 500ms。
          final ms = DateTime.now().difference(_lpStartTime!).inMilliseconds;
          _lpProgress.value = ms <= 200
              ? 0
              : ((ms - 200) / 300).clamp(0.0, 1.0);
        }
        _tickMarqueeHint();
        _tickHandlesAppear();
        _tickNoteFades();
        if (selState != 2 && _selectionStatusActive) {
          _selectionStatusActive = false;
          _editStatus.value = null;
        }
      }
      // 时间码每帧更新（60Hz），避免毫秒位按 4 帧（~67ms）跳动。
      _updateTimecode();
      // 工程状态降频（每 ~66ms）：减少音频锁与历史栈分配开销。
      // 对话框打开时也要跑：_pollArchive 靠它关闭加载进度框。
      _frameCounter++;
      if (_frameCounter % 4 == 0) {
        final generation = rust.documentGeneration();
        if (generation != _docGeneration) {
          _docGeneration = generation;
          _refreshDocumentMeta();
          if (!_loadUiSettled) {
            // 先让这一帧把轨道/自动化/纹理画出来，再放进度框关闭。
            WidgetsBinding.instance.addPostFrameCallback((_) {
              if (mounted) {
                _loadUiSettled = true;
              }
            });
          }
        }
        _readDocState();
        _pollArchive();
      }
      // 保持连续帧：Rust 侧拖拽/边缘滚动/渲染都由帧驱动（手指静止时也要跑）。
      if (!_showHome) {
        SchedulerBinding.instance.scheduleFrame();
      }
    })..start();
    _recent = _readRecent();
    _loadLanguage();
    // 默认 AR 1/1、PR 1/16（文档可能是空白占位，先显式下发一次）。
    _pushAllQuantizes();
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
    _marqueeHintPos.dispose();
    _marqueeHint.dispose();
    _handlesScale.dispose();
    _noteFadeTick.dispose();
    _lpPosNotifier.dispose();
    _moveDashed.dispose();
    _lpIndicator.dispose();
    _overlay.dispose();
    _sfRevision.dispose();
    _grid.dispose();
    _saveDoneTimer?.cancel();
    _savingTimer?.cancel();
    _velocityReloadTimer?.cancel();
    _docState.dispose();
    _editStatus.dispose();
    calloc.free(_snapPtr);
    calloc.free(_statePtr);
    calloc.free(_dragDtPtr);
    calloc.free(_dragDkPtr);
    calloc.free(_dragDtrPtr);
    super.dispose();
  }

  /// PR/AR 选框移动态：顶部栏显示相对 tick / key 或轨道差。
  void _updateSelectionStatus() {
    if (rust.dragDelta(_dragDtPtr, _dragDkPtr, _dragDtrPtr) != 1) {
      if (_selectionStatusActive) {
        _selectionStatusActive = false;
        _editStatus.value = null;
      }
      return;
    }
    final dt = _dragDtPtr.value;
    final dk = _dragDkPtr.value;
    final dtr = _dragDtrPtr.value;
    _editStatus.value = EditStatus(
      hidePosition: true,
      primary: '${dt >= 0 ? '+' : ''}$dt tick',
      secondary: _viewMode == kViewPianoroll
          ? '${dk >= 0 ? '+' : ''}$dk key'
          : '${dtr >= 0 ? '+' : ''}$dtr track',
    );
    _selectionStatusActive = true;
  }

  /// 从桌面/后台返回：保存状态复位（系统栏由 [SystemBarsMode] 负责恢复）。
  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) {
      // 从 SAF 选择器返回：保存流程要么已完成（onProjectSaved 会给对勾），
      // 要么被取消 —— 这里清掉"保存中"圆环，避免一直转。
      if (_saving) {
        _savingTimer?.cancel();
        _savingTimer = null;
        setState(() => _saving = false);
      }
    } else {
      // 退出应用（切后台）按设置暂停播放。
      if (state == AppLifecycleState.paused &&
          rust.pauseOnExit() == 1 &&
          rust.isPlaying() == 1) {
        rust.transport(1);
        if (mounted) {
          setState(() => _playing = false);
        }
      }
      // 切后台/失去焦点时触点 up 可能收不到：清空，避免残留成“幽灵手指”。
      _resetInteraction();
    }
  }
}
