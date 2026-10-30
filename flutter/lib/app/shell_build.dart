part of 'studio_shell.dart';

mixin _ShellBuild on _ShellBase {
  // ── 纹理尺寸上报 ──

  /// 布局阶段只登记画布物理尺寸，MethodChannel 上报留到帧末。
  ///
  /// 不能直接在 [LayoutBuilder.builder] 里 invokeMethod：那是 layout 阶段的
  /// 副作用，且一帧内可能多次 layout；帧末只发最终值。
  void _queueCanvasSize(int w, int h, double lw, double lh) {
    final pixel = Size(w.toDouble(), h.toDouble());
    if (_reportedPixelSize == pixel) {
      // 本帧又变回已上报尺寸：取消可能存在的待发送值。
      _pendingCanvasSize = null;
      return;
    }
    final next = (w: w, h: h, lw: lw, lh: lh);
    if (_pendingCanvasSize == next) {
      return;
    }
    _pendingCanvasSize = next;
    _scheduleSurfaceSizeFlush();
  }

  /// AM 面板同理：只有物理尺寸，没有逻辑视口。
  void _queueAutomationSize(int w, int h) {
    final pixel = Size(w.toDouble(), h.toDouble());
    if (_reportedAmSize == pixel) {
      _pendingAmSize = null;
      return;
    }
    final next = (w: w, h: h);
    if (_pendingAmSize == next) {
      return;
    }
    _pendingAmSize = next;
    _scheduleSurfaceSizeFlush();
  }

  void _scheduleSurfaceSizeFlush() {
    if (_surfaceSizeFlushScheduled) {
      return;
    }
    _surfaceSizeFlushScheduled = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _surfaceSizeFlushScheduled = false;
      if (!mounted) {
        return;
      }
      final canvas = _pendingCanvasSize;
      _pendingCanvasSize = null;
      if (canvas != null) {
        final pixel = Size(canvas.w.toDouble(), canvas.h.toDouble());
        if (_reportedPixelSize != pixel) {
          _reportedPixelSize = pixel;
          _channel.invokeMethod<void>('setSize', {
            'w': canvas.w,
            'h': canvas.h,
            'lw': canvas.lw,
            'lh': canvas.lh,
          });
        }
      }
      final am = _pendingAmSize;
      _pendingAmSize = null;
      if (am != null) {
        final pixel = Size(am.w.toDouble(), am.h.toDouble());
        if (_reportedAmSize != pixel) {
          _reportedAmSize = pixel;
          _channel.invokeMethod<void>('setAutomationSize', {
            'w': am.w,
            'h': am.h,
          });
        }
      }
    });
  }

  /// 打开主页：按设置决定是否暂停播放。
  void _openHome() {
    final pause = rust.pauseOnHome() == 1 && rust.isPlaying() == 1;
    if (pause) {
      rust.transport(1);
    }
    setState(() {
      _showHome = true;
      if (pause) {
        _playing = false;
      }
    });
    _updateTimecode();
  }

  // ── 布局 ──

  /// 量化菜单：'自定义' 打开分子/分母输入，其余直接切换当前视图档位。
  Future<void> _selectQuantize(String value) async {
    var label = value;
    if (value == '自定义') {
      final parsed = _parseQuantizeFraction(_quantizeLabel);
      final picked = await showCustomQuantizeDialog(
        context,
        ppq: _ppq.round(),
        numerator: parsed?.$1 ?? 1,
        denominator: parsed?.$2 ?? (_viewMode == kViewPianoroll ? 16 : 4),
      );
      if (picked == null || !mounted) {
        return;
      }
      label = '${picked.$1}/${picked.$2}';
    }
    if (!mounted) {
      return;
    }
    setState(() => _setQuantizeLabel(label));
    _pushQuantize(label);
  }

  @override
  Widget build(BuildContext context) {
    final id = _textureId;
    if (id == null) {
      return const Scaffold(
        backgroundColor: Color(0xFFF0F0F4),
        body: Center(child: CircularProgressIndicator()),
      );
    }
    // 竖屏顶栏拆两行（42 → 84），横屏保持单行。
    final compactTop = MediaQuery.sizeOf(context).width < 600;
    final topBarHeight = compactTop ? 84.0 : 42.0;
    // 竖屏显示系统栏时给状态栏/导航栏让位；必须用 viewPadding（不随键盘变化），
    // 否则键盘弹出时 padding.bottom 变成 0，会让画布逐帧改尺寸、重配交换链。
    final barsVisible = SystemBarsMode.visible(context);
    final viewPadding = MediaQuery.viewPaddingOf(context);
    return PopScope(
      // 拦截系统返回：设置页 → 关闭；PR → AR；AR → 主页；主页才真正退出。
      canPop: false,
      onPopInvokedWithResult: (didPop, _) {
        if (didPop) {
          return;
        }
        if (_showSettings) {
          setState(() => _showSettings = false);
        } else if (!_showHome && _viewMode == kViewPianoroll) {
          _setView(kViewArrangement);
        } else if (!_showHome) {
          _openHome();
        } else {
          SystemNavigator.pop();
        }
      },
      child: Scaffold(
        backgroundColor: YinheTheme.bg,
        // 键盘只移动对话框，不缩放画布/wgpu 纹理（adjustNothing）。
        resizeToAvoidBottomInset: false,
        // 画布不吃 top/bottom 安全区：键盘弹出/收起时 Android 会带着导航栏
        // 做 inset 动画，MediaQuery.padding.bottom 在过渡期间反复变化；
        // 若这里消费 padding，LayoutBuilder 会逐帧改尺寸并重配 wgpu 交换链。
        // 沉浸式全屏下画布本来就铺满窗口，左右仍保留刘海避让。
        body: SafeArea(
          top: false,
          bottom: false,
          child: Padding(
            padding: EdgeInsets.only(
              top: barsVisible ? viewPadding.top : 0,
              bottom: barsVisible ? viewPadding.bottom : 0,
            ),
            child: Stack(
              children: [
                Column(
                  children: [
                    // 顶部栏占位（栏本体在外层 Stack 顶层，保证绘制优先级最高）。
                    SizedBox(height: topBarHeight),
                    Expanded(
                      child: LayoutBuilder(
                        builder: (context, constraints) {
                          final size = constraints.biggest;
                          // AM 面板可见时压缩画布高度：PR 与 AM 上下排列、互不覆盖。
                          final amVisible =
                              !_showHome &&
                              _viewMode == kViewPianoroll &&
                              !_autoCollapsed;
                          final maxAmH =
                              ((size.height - RulerPainter.height) * 0.6).clamp(
                                AutomationPanel.minHeight,
                                double.infinity,
                              );
                          _autoPanelMaxHeight = maxAmH;
                          final amH = amVisible
                              ? _autoPanelHeight.clamp(
                                  AutomationPanel.minHeight,
                                  maxAmH,
                                )
                              : 0.0;
                          final canvasH = (size.height - amH).clamp(
                            1.0,
                            double.infinity,
                          );
                          // 内容画布 = 画布区减去顶部标尺条（Rust 坐标系从标尺下方开始）。
                          final contentH = (canvasH - RulerPainter.height)
                              .clamp(1.0, double.infinity);
                          final dpr = MediaQuery.devicePixelRatioOf(context);
                          final pixelW = (size.width * dpr).round();
                          final pixelH = (contentH * dpr).round();
                          _queueCanvasSize(
                            pixelW,
                            pixelH,
                            size.width,
                            contentH,
                          );
                          // AM 与 PR 一样在布局层确认视口：物理尺寸才决定
                          // SurfaceProducer 的交换链大小，不能等子组件回调。
                          if (amVisible) {
                            final amW =
                                (size.width - AutomationPainter.comboWidth)
                                    .clamp(1.0, double.infinity);
                            final amH2 = (amH - AutomationPanel.handleHeight)
                                .clamp(1.0, double.infinity);
                            final amPw = (amW * dpr).round();
                            final amPh = (amH2 * dpr).round();
                            _queueAutomationSize(amPw, amPh);
                          }
                          return RawGestureDetector(
                            behavior: HitTestBehavior.translucent,
                            gestures: <Type, GestureRecognizerFactory>{
                              MultiFingerGestureRecognizer:
                                  GestureRecognizerFactoryWithHandlers<
                                    MultiFingerGestureRecognizer
                                  >(
                                    () => MultiFingerGestureRecognizer(),
                                    (r) => r
                                      ..onTwoFingerTap = _undoFromGesture
                                      ..onThreeFingerTap = _redoFromGesture
                                      ..onSwipeLeft = _undoFromGesture
                                      ..onSwipeRight = _redoFromGesture,
                                  ),
                            },
                            child: Stack(
                              children: [
                                Positioned(
                                  left: 0,
                                  right: 0,
                                  top: 0,
                                  height: canvasH,
                                  child: Stack(
                                    children: [
                                      // PR 背景/网格由 Flutter 画（wgpu 只留音符/键盘）：
                                      // 与选框同一帧，缩放/平移不会错位。
                                      if (!_showHome &&
                                          _viewMode == kViewArrangement)
                                        Positioned(
                                          left: 0,
                                          right: 0,
                                          top: RulerPainter.height,
                                          bottom: 0,
                                          child: IgnorePointer(
                                            child: CustomPaint(
                                              painter: ArrangementGridPainter(
                                                _snap,
                                                _gridLines,
                                                _tracks,
                                              ),
                                            ),
                                          ),
                                        ),
                                      if (!_showHome &&
                                          _viewMode == kViewPianoroll)
                                        Positioned(
                                          left: 0,
                                          right: 0,
                                          top: RulerPainter.height,
                                          bottom: 0,
                                          child: IgnorePointer(
                                            child: CustomPaint(
                                              painter: PianorollGridPainter(
                                                _snap,
                                                _gridLines,
                                              ),
                                            ),
                                          ),
                                        ),
                                      Positioned(
                                        left: 0,
                                        right: 0,
                                        top: RulerPainter.height,
                                        bottom: 0,
                                        child: GestureDetector(
                                          behavior: HitTestBehavior.opaque,
                                          onTapUp: _handleTapUp,
                                          onLongPressStart:
                                              _handleLongPressStart,
                                          onLongPressMoveUpdate:
                                              _handleLongPressMove,
                                          onLongPressEnd: _handleLongPressEnd,
                                          onScaleStart: _handleScaleStart,
                                          onScaleUpdate: _handleScaleUpdate,
                                          onScaleEnd: _handleScaleEnd,
                                          child: Listener(
                                            behavior: HitTestBehavior.opaque,
                                            onPointerDown: _handlePointerDown,
                                            onPointerMove: _handlePointerMove,
                                            onPointerUp: _handlePointerUp,
                                            onPointerCancel:
                                                _handlePointerCancel,
                                            onPointerSignal:
                                                _handlePointerSignal,
                                            child: Texture(textureId: id),
                                          ),
                                        ),
                                      ),
                                      // 层级：音符（Texture）→ 演奏指示线 → 音轨面板 → 时间标尺 → 自动化栏。
                                      if (!_showHome)
                                        Positioned.fill(
                                          child: IgnorePointer(
                                            child: CustomPaint(
                                              painter: PlayheadPainter(
                                                _playhead,
                                                _snap,
                                              ),
                                            ),
                                          ),
                                        ),
                                      // 卷帘背景 C 音名：设置「显示键盘」开启时显示。
                                      if (!_showHome &&
                                          _viewMode == kViewPianoroll)
                                        Positioned(
                                          left: 0,
                                          right: 0,
                                          top: RulerPainter.height,
                                          bottom: 0,
                                          child: IgnorePointer(
                                            child: CustomPaint(
                                              painter: KeyLabelPainter(
                                                _snap,
                                                showKeyboardLabels,
                                              ),
                                            ),
                                          ),
                                        ),
                                      // 删除残影：在音符纹理之上、选框/锚点之下。
                                      if (!_showHome)
                                        Positioned(
                                          left: 0,
                                          right: 0,
                                          top: RulerPainter.height,
                                          bottom: 0,
                                          child: IgnorePointer(
                                            child: CustomPaint(
                                              painter: NoteDeleteFadePainter(
                                                _noteFadeTick,
                                                _noteFades,
                                              ),
                                            ),
                                          ),
                                        ),
                                      // 选框必须在音轨面板/键盘下方：面板不透明底会盖住越界部分。
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
                                                _snap,
                                                _tool == 0,
                                                _handlesScale,
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
                                              painter: LabelsPainter(
                                                snap: _snap,
                                                tracks: _tracks,
                                                dragFrom: _dragFrom,
                                                dragTo: _dragTo,
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
                                              painter: RulerPainter(
                                                _rulerLabels,
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
                                              painter:
                                                  LongPressIndicatorPainter(
                                                    _snap,
                                                    _lpIndicator,
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
                                          child: MarkerLayer(
                                            snap: _snap,
                                            grid: _grid,
                                            markers: _markers,
                                            onChanged: _refreshDocumentMeta,
                                            onInsertStart:
                                                _handleRulerInsertStart,
                                            onInsertMove:
                                                _handleRulerInsertMove,
                                            onInsertEnd: _handleRulerInsertEnd,
                                          ),
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
                                              painter:
                                                  SelectionLongPressPainter(
                                                    _lpProgress,
                                                    _lpPosNotifier,
                                                    _moveDashed,
                                                    _overlay,
                                                    _snap,
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
                                              painter: MarqueeHintPainter(
                                                _marqueeHintPos,
                                                _marqueeHint,
                                              ),
                                            ),
                                          ),
                                        ),
                                    ],
                                  ),
                                ),
                                if (amVisible)
                                  Positioned(
                                    left: 0,
                                    right: 0,
                                    bottom: 0,
                                    height: amH,
                                    child: AutomationPanel(
                                      height: amH,
                                      snap: _snap,
                                      gridLines: _gridLines,
                                      lanes: _autoLanes,
                                      velocity: _velocityBars,
                                      trackColors: [
                                        for (final t in _tracks.value) t.color,
                                      ],
                                      activeTrack: _activeTrack ?? 0,
                                      isConductor:
                                          _activeTrack != null &&
                                          _activeTrack! >= 0 &&
                                          _activeTrack! <
                                              _tracks.value.length &&
                                          _tracks.value[_activeTrack!].master,
                                      status: _editStatus,
                                      formatTick: _formatEditTick,
                                      onCollapsed: _setAutoCollapsed,
                                      onReload: _loadAutomation,
                                      textureId: _amTextureId,
                                      pencilActive: _tool == 2 || _tool == 5,
                                      deselectTick: _amDeselectTick,
                                      onVelocityCommit: _commitVelocityStroke,
                                      velocityDrawEnabled:
                                          _tool == 2 || _tool == 5,
                                      lineTool: _tool == 5,
                                      marqueeEnabled: _tool == 0,
                                      onInsertAtTick: (tick) {
                                        final track = _activeTrack ?? 0;
                                        final conductor =
                                            track >= 0 &&
                                            track < _tracks.value.length &&
                                            _tracks.value[track].master;
                                        _runInsertFlow(
                                          LongPressContext(
                                            tick: tick,
                                            track: track,
                                            conductor: conductor,
                                            pr: true,
                                          ),
                                        );
                                      },
                                      onResize: _resizeAutoPanel,
                                      onResetHeight: _resetAutoPanelHeight,
                                    ),
                                  ),
                                if (!_showHome &&
                                    _viewMode == kViewPianoroll &&
                                    _autoCollapsed)
                                  AutoExpandButton(
                                    onTap: () => _setAutoCollapsed(false),
                                  ),
                              ],
                            ),
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
                    height: topBarHeight,
                    child: _buildTopBar(
                      compact: compactTop,
                      height: topBarHeight,
                    ),
                  ),
                if (_showHome)
                  HomeView(
                    recent: _recent,
                    onPickMidi: _pickMidi,
                    onNewProject: _newProject,
                    onSaveProjectAs: () => _saveProject(saveAs: true),
                    onOpenRecent: _openRecent,
                    onOpenSettings: _openSettings,
                    onExportAudio: _exportAudio,
                    onExportMidi: _exportMidi,
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
        ),
      ),
    );
  }

  /// 上侧栏（实现见 top_bar.dart）。
  Widget _buildTopBar({required bool compact, required double height}) {
    return TopBar(
      height: height,
      compact: compact,
      isPr: _viewMode == kViewPianoroll,
      playing: _playing,
      timecodeText: _timecodeText,
      editStatus: _editStatus,
      quantizeLabel: _quantizeLabel,
      ppq: _ppq,
      mainTool: _mainTool,
      quickTool: _quickTool,
      quickSlotActive: _quickSlotActive,
      tracks: _tracks.value,
      activeTrack: _activeTrack ?? -1,
      showAllTracks: _showAllTracks,
      songTitle: _songTitle,
      fileName: _fileName,
      docState: _docState,
      onHome: _openHome,
      onBack: () => _setView(kViewArrangement),
      onTogglePlay: _togglePlay,
      onStop: _stopPlayback,
      onToggleTimecode: () {
        setState(() => _timecodeBar = !_timecodeBar);
        _updateTimecode();
      },
      onQuantize: (v) => unawaited(_selectQuantize(v)),
      onTool: _selectTool,
      onQuickTool: _selectQuickTool,
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
      onDeselect: () {
        final had = rust.clearSelection() == 1;
        setState(() => _amDeselectTick++);
        if (had) {
          _refreshDocumentMeta();
        }
      },
    );
  }

  /// 选择当前编辑音轨：PR 顶部显示它；「全部轨道」关闭时只绘制该轨。
  @override
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
  @override
  void _undo() {
    if (rust.undo() == 1) {
      _refreshDocumentMeta();
    }
  }

  /// 重做一步（含快照/音频刷新）。
  @override
  void _redo() {
    if (rust.redo() == 1) {
      _refreshDocumentMeta();
    }
  }
}
