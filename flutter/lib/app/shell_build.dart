part of 'studio_shell.dart';

mixin _ShellBuild on _ShellBase {
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
      // 键盘只移动对话框，不缩放画布/wgpu 纹理（adjustNothing）。
      resizeToAvoidBottomInset: false,
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
                              behavior: HitTestBehavior.opaque,
                              onTapUp: _handleTapUp,
                              onLongPressStart: _handleLongPressStart,
                              onLongPressMoveUpdate: _handleLongPressMove,
                              onLongPressEnd: _handleLongPressEnd,
                              onScaleStart: _handleScaleStart,
                              onScaleUpdate: _handleScaleUpdate,
                              onScaleEnd: _handleScaleEnd,
                              child: Listener(
                                behavior: HitTestBehavior.opaque,
                                onPointerDown: _handlePointerDown,
                                onPointerUp: _handlePointerUp,
                                onPointerCancel: _handlePointerCancel,
                                onPointerSignal: _handlePointerSignal,
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
