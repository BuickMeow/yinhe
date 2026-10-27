part of 'studio_shell.dart';

mixin _ShellMenus on _ShellBase {
  /// 长按完成但未移动：选区操作网格（PR/AR 音符）。
  @override
  void _showSelectionMenu() {
    _resetInteraction();
    unawaited(_applySelectionAction(0));
  }

  /// 弹选区操作网格并执行；`scope` 0=音符，1=AM 锚点。
  Future<void> _applySelectionAction(int scope) async {
    final id = await showSelectionActionPicker(context, automation: scope == 1);
    if (!mounted || id == null) {
      return;
    }
    // 复制：写入应用内剪贴板，不产生 undo。
    if (id == 1) {
      if (rust.copySelection() == 1) {
        showSnack(context, l.copied, duration: const Duration(seconds: 1));
      }
      return;
    }
    // 剪切 = 复制 + 删除（删除本身一步 undo）。
    if (id == 2) {
      if (rust.copySelection() == 1) {
        if (rust.selectionAction(scope, 11) == 1) {
          _refreshDocumentMeta();
        }
      }
      return;
    }
    // 人性化：幅度由对话框决定，不再固定 8/8/8。
    if (id == 16) {
      final spec = await showHumanizeDialog(context);
      if (!mounted || spec == null) {
        return;
      }
      if (rust.humanize(spec.timing, spec.velocity, spec.gate) == 1) {
        _refreshDocumentMeta();
      }
      return;
    }
    // 批量处理：音符或 AM 锚点，字段固定设值，一次 undo。
    if (id == 24) {
      final automation = scope == 1;
      final spec = await showBatchEditDialog(
        context,
        automation: automation,
        selectionRange: automation ? null : rust.selectionRange(),
        formatTick: _formatEditTick,
      );
      if (!mounted || spec == null) {
        return;
      }
      final ok = automation
          ? rust.automationBatchEdit(spec.value, spec.tick)
          : rust.batchEdit(
              spec.value,
              spec.gate,
              spec.key,
              spec.tick,
              spec.stretch,
            );
      if (ok == 1) {
        _refreshDocumentMeta();
        if (automation) {
          _loadAutomation();
        }
      }
      return;
    }
    if (rust.selectionAction(scope, id) == 1) {
      _refreshDocumentMeta();
      if (scope == 1) {
        _loadAutomation();
      }
    }
  }

  /// 空白处长按完成：粘贴/插入菜单（都在长按位置操作）。
  @override
  void _showPasteMenu(Offset? pos, LongPressContext? ctx) {
    if (pos == null) {
      return;
    }
    _resetInteraction();
    final token = ++_lpToken;
    _lpIndicator.value = ctx == null
        ? null
        : LongPressIndicator(tick: ctx.tick, track: ctx.track, pr: ctx.pr);
    if (ctx != null) {
      // 菜单/后续插入对话框期间，顶部栏保持蓝色插入位置。
      _setInsertStatus(ctx.tick);
    }
    var inserting = false;
    showEmptyLongPressMenu(
      context,
      canPaste: rust.clipboardEmpty() == 0,
      onSelectAll: () {
        if (_viewMode == kViewPianoroll) {
          final track =
              ctx?.track ??
              (_displayTrack >= 0 ? _displayTrack : (_activeTrack ?? -1));
          _selectAll(0, track);
        } else {
          // AR 空白长按：全工程所有轨道、所有长度。
          _selectAll(1, -1);
        }
      },
      onPaste: () {
        if (rust.pasteSelection(pos.dx) == 1) {
          _refreshDocumentMeta();
        }
      },
      onInsert: ctx == null
          ? null
          : () {
              inserting = true;
              _runInsertFlow(ctx);
            },
    ).whenComplete(() {
      if (!inserting) {
        _clearInsertStatus();
      }
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
          ? [(l.off, 0), (l.on, 127)]
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
      // 插入流程（含目标/值/自定义号对话框）结束后才恢复普通时间显示。
      _clearInsertStatus();
    }
  }

  /// 列表末尾占位行：添加音轨。
  @override
  void _showAddTrackDialog() {
    final rows = _tracks.value;
    final last = rows.isNotEmpty ? rows.last : null;
    _resetInteraction();
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

  /// AR 长按音轨（未拖动）：音轨信息菜单（含删除）。
  @override
  void _showTrackMenu(int track) {
    final rows = _tracks.value;
    final master = track < 0 || track >= rows.length || rows[track].master;
    _resetInteraction();
    showTrackMenu(
      context,
      onSelectAll: () => _selectAll(1, track),
      onEdit: () => _editTrackInfo(track),
      onDelete: master ? null : () => _deleteTrack(track),
      onEvents: () => _showEventList(track),
    );
  }

  /// 全选：`scope` 0 = PR，1 = AR；`track < 0` = 全工程。
  void _selectAll(int scope, int track) {
    if (rust.selectAll(scope, track) == 1) {
      _refreshDocumentMeta();
    }
  }

  /// 打开事件列表（音符/自动化/全局事件穿插）。
  void _showEventList(int track) {
    _resetInteraction();
    showEventListDialog(
      context,
      grid: _grid,
      tracks: _tracks.value,
      initialTrack: track,
      onJump: _jumpToEvent,
      onChanged: _refreshDocumentMeta,
    );
  }

  /// 事件列表「跳转」：选中该轨、进入 PR、光标与视口中心落在事件起点。
  void _jumpToEvent(int track, double tick) {
    _showAllTracks = false;
    _selectEditTrack(track);
    _setView(kViewPianoroll);
    rust.jumpToEvent(tick);
  }

  /// 删除音轨（连同音符；一步一 undo）。已经过二次确认。
  Future<void> _deleteTrack(int track) async {
    final ok = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text('${l.deleteTrack}?'),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(ctx).pop(false),
            child: Text(l.cancel),
          ),
          FilledButton(
            onPressed: () => Navigator.of(ctx).pop(true),
            child: Text(l.ok),
          ),
        ],
      ),
    );
    if (ok != true || !mounted) {
      return;
    }
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
    _resetInteraction();
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
}
