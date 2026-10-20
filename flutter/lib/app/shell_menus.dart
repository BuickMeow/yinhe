part of 'studio_shell.dart';

mixin _ShellMenus on _ShellBase {
  /// 长按完成但未移动：选区操作菜单。
  @override
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
  @override
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
    }
  }

  /// 列表末尾占位行：添加音轨。
  @override
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

  /// AR 长按音轨（未拖动）：音轨信息菜单（含删除）。
  @override
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
}
