import 'dart:convert';
import 'dart:ffi' as ffi;

import 'package:ffi/ffi.dart';
import 'package:flutter/material.dart';

import '../core/bar_beat.dart';
import '../core/ffi.dart' as rust;
import '../core/midi_names.dart';
import '../core/models.dart';
import '../core/snack.dart';
import 'inputs.dart';
import 'insert_flow.dart';
import 'sheet.dart';
import '../core/theme.dart';
import '../l10n/app_localizations.dart';

/// 事件列表（移动端）：音符 + 自动化 + Tempo/标签穿插，按
/// 时间 → 音高/值 → 时长排序。
///
/// 长按一行：修改 / 删除 / 复制 / 粘贴（同时间位置）/ 跳转。
Future<void> showEventListDialog(
  BuildContext context, {
  required ValueNotifier<BarBeatGrid?> grid,
  required List<TrackRow> tracks,
  required int initialTrack,
  required void Function(int track, double tick) onJump,
  required VoidCallback onChanged,
}) {
  return showDialog<void>(
    context: context,
    builder: (_) => EventListDialog(
      grid: grid,
      tracks: tracks,
      initialTrack: initialTrack,
      onJump: onJump,
      onChanged: onChanged,
    ),
  );
}

class EventListDialog extends StatefulWidget {
  const EventListDialog({
    super.key,
    required this.grid,
    required this.tracks,
    required this.initialTrack,
    required this.onJump,
    required this.onChanged,
  });

  final ValueNotifier<BarBeatGrid?> grid;
  final List<TrackRow> tracks;

  /// 进入时的音轨；-1 = 全部轨道。
  final int initialTrack;
  final void Function(int track, double tick) onJump;
  final VoidCallback onChanged;

  @override
  State<EventListDialog> createState() => _EventListDialogState();
}

class _EventListDialogState extends State<EventListDialog> {
  AppLocalizations get l => AppLocalizations.of(context);

  static const int _pageSize = 300;

  final List<Map<String, dynamic>> _rows = [];
  final Set<String> _seen = {};
  final ScrollController _scroll = ScrollController();
  double _fromTick = 0;
  bool _loading = false;
  bool _done = false;
  int _page = 1;
  int _pageCount = 1;
  Map<String, dynamic>? _copied;

  /// 当前筛选轨；-1 = 全部轨道。
  late int _track =
      widget.initialTrack >= 0 && widget.initialTrack < widget.tracks.length
      ? widget.initialTrack
      : -1;

  @override
  void initState() {
    super.initState();
    _scroll.addListener(_updatePageInfo);
    _loadMore();
  }

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  String _keyOf(Map<String, dynamic> r) {
    final k = r['k'];
    return '$k|${r['tr']}|${r['t']}|${r['lane']}|${r['key']}|${r['id']}|${r['text']}';
  }

  Future<void> _loadMore() async {
    if (_loading || _done) {
      return;
    }
    setState(() => _loading = true);
    try {
      // 事件列表按时间分页；选中单轨时在客户端过滤，避免 Rust 为稀疏轨
      // 扫描整首曲子。一次最多取 4 页，防止长时间占用 UI 线程。
      var pages = 0;
      while (!_done && pages < 4) {
        pages++;
        final list = _fetchPage();
        if (list == null || list.isEmpty) {
          _done = true;
          break;
        }
        var cursorMoved = false;
        for (final it in list) {
          if (it is! Map<String, dynamic>) {
            continue;
          }
          final key = _keyOf(it);
          if (!_seen.add(key)) {
            continue;
          }
          cursorMoved = true;
          if (_track < 0 || (it['tr'] as num?)?.toInt() == _track) {
            _rows.add(it);
          }
        }
        final lastTick = (list.last['t'] as num?)?.toDouble() ?? _fromTick;
        // 有新行：从同一 tick 继续取（同 tick 的剩余事件）；没有新行说明
        // 该 tick 已经取完，前进 1 tick 避免卡住。
        final next = cursorMoved ? lastTick : lastTick + 1;
        if (next == _fromTick && !cursorMoved) {
          _done = true;
        } else {
          _fromTick = next;
        }
        if (list.length < _pageSize) {
          _done = true;
        }
        if (_rows.length >= _pageSize) {
          break;
        }
      }
    } catch (_) {
      _done = true;
    } finally {
      if (mounted) {
        setState(() => _loading = false);
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (mounted) {
            _updatePageInfo();
          }
        });
      }
    }
  }

  void _updatePageInfo() {
    if (!_scroll.hasClients) {
      return;
    }
    final viewport = _scroll.position.viewportDimension;
    if (viewport <= 0) {
      return;
    }
    final pages = (_scroll.position.maxScrollExtent / viewport).ceil() + 1;
    final page = (_scroll.position.pixels / viewport).floor() + 1;
    if (mounted && (page != _page || pages != _pageCount)) {
      setState(() {
        _page = page;
        _pageCount = pages;
      });
    }
  }

  void _scrollToPage(int page) {
    if (!_scroll.hasClients) {
      return;
    }
    final viewport = _scroll.position.viewportDimension;
    final target = ((page - 1) * viewport).clamp(
      0.0,
      _scroll.position.maxScrollExtent,
    );
    _scroll.animateTo(
      target,
      duration: const Duration(milliseconds: 180),
      curve: Curves.easeOut,
    );
  }

  void _prevPage() {
    if (_page > 1) {
      _scrollToPage(_page - 1);
    }
  }

  void _nextPage() {
    if (_page < _pageCount) {
      _scrollToPage(_page + 1);
      return;
    }
    if (!_done && !_loading) {
      _loadMore().then((_) {
        if (mounted) {
          WidgetsBinding.instance.addPostFrameCallback((_) {
            if (mounted) {
              _updatePageInfo();
              _scrollToPage(_page + 1);
            }
          });
        }
      });
    }
  }

  List<dynamic>? _fetchPage() {
    final ptr = rust.eventsJson(_fromTick, _pageSize);
    if (ptr == ffi.nullptr) {
      return null;
    }
    try {
      final decoded = jsonDecode(ptr.toDartString()) as Map<String, dynamic>;
      final list = decoded['rows'];
      return list is List ? list : null;
    } finally {
      rust.freeString(ptr);
    }
  }

  void _reload() {
    setState(() {
      _rows.clear();
      _seen.clear();
      _fromTick = 0;
      _done = false;
    });
    if (_scroll.hasClients) {
      _scroll.jumpTo(0);
    }
    _loadMore();
  }

  String _timeText(int tick) {
    final grid = widget.grid.value;
    return grid?.format(tick.toDouble()) ?? '$tick';
  }

  String _noteName(int key) => MidiNames.instance.noteName(key);

  String _ccName(int cc) => ccInsertLabel(cc);

  ({String title, String subtitle, IconData icon}) _describe(
    Map<String, dynamic> r,
  ) {
    final kind = r['k'] as String? ?? '';
    final tick = (r['t'] as num?)?.toInt() ?? 0;
    final track = r['tn'] as String? ?? '';
    final time = _timeText(tick);
    switch (kind) {
      case 'note':
        final key = (r['key'] as num?)?.toInt() ?? 0;
        final vel = (r['vel'] as num?)?.toInt() ?? 0;
        final end = (r['end'] as num?)?.toInt() ?? tick;
        return (
          title: l.eventNoteTitle(_noteName(key)),
          subtitle: l.eventNoteDetail(time, track, vel, end - tick),
          icon: Icons.music_note,
        );
      case 'cc':
        final number = (r['num'] as num?)?.toInt() ?? 0;
        final val = (r['val'] as num?)?.toDouble() ?? 0;
        return (
          title: _ccName(number),
          subtitle: l.eventValueDetail(
            time,
            track,
            val.round(),
            (r['max'] as num?)?.round() ?? 127,
          ),
          icon: Icons.tune,
        );
      case 'pb':
        final val = (r['val'] as num?)?.toDouble() ?? 0;
        return (
          title: 'Pitch Bend',
          subtitle: '$time · $track · ${val.round()}（${l.centerHint}）',
          icon: Icons.swap_vert,
        );
      case 'rpn':
        final number = (r['num'] as num?)?.toInt() ?? 0;
        final val = (r['val'] as num?)?.toDouble() ?? 0;
        return (
          title: rpnInsertLabel(number),
          subtitle: l.eventValueDetail(
            time,
            track,
            val.round(),
            (r['max'] as num?)?.round() ?? 127,
          ),
          icon: Icons.straighten,
        );
      case 'tempo':
        final bpm = (r['val'] as num?)?.toDouble() ?? 0;
        return (
          title: 'Tempo',
          subtitle: '$time · ${bpm.toStringAsFixed(2)} BPM',
          icon: Icons.speed,
        );
      case 'marker':
        final text = r['text'] as String? ?? '';
        return (
          title: text.isEmpty ? l.markerLabel : text,
          subtitle: '$time · ${l.markerLabel}',
          icon: Icons.bookmark,
        );
      default:
        return (title: kind, subtitle: time, icon: Icons.help_outline);
    }
  }

  Future<void> _openActions(Map<String, dynamic> row) async {
    final desc = _describe(row);
    final action = await showActionSheet(
      context,
      title: desc.title,
      subtitle: desc.subtitle,
      headerIcon: desc.icon,
      actions: [
        SheetAction(value: 'edit', label: l.edit, icon: Icons.edit),
        SheetAction(value: 'copy', label: l.copy, icon: Icons.copy),
        SheetAction(
          value: 'paste',
          label: l.pasteSameTime,
          icon: Icons.content_paste,
          enabled: _copied != null,
        ),
        SheetAction(
          value: 'delete',
          label: l.delete,
          icon: Icons.delete_outline,
          danger: true,
        ),
        SheetAction(value: 'jump', label: l.jumpTo, icon: Icons.my_location),
      ],
    );
    if (!mounted || action == null) {
      return;
    }
    final json = jsonEncode(row);
    final p = json.toNativeUtf8();
    try {
      switch (action) {
        case 'copy':
          _copied = row;
          showSnack(
            context,
            l.eventCopied,
            duration: const Duration(seconds: 1),
          );
        case 'delete':
          if (rust.eventDelete(p) == 1) {
            widget.onChanged();
            _reload();
          }
        case 'paste':
          final src = _copied;
          if (src != null) {
            final sp = jsonEncode(src).toNativeUtf8();
            try {
              if (rust.eventPaste(sp, p) == 1) {
                widget.onChanged();
                _reload();
              }
            } finally {
              calloc.free(sp);
            }
          }
        case 'jump':
          final track = (row['tr'] as num?)?.toInt() ?? 0;
          final tick = (row['t'] as num?)?.toDouble() ?? 0;
          Navigator.of(context).pop();
          widget.onJump(track, tick);
          return;
        case 'edit':
          await _editRow(row, p);
      }
    } finally {
      calloc.free(p);
    }
  }

  Future<void> _editRow(
    Map<String, dynamic> row,
    ffi.Pointer<Utf8> json,
  ) async {
    final kind = row['k'] as String? ?? '';
    if (kind == 'marker') {
      final text = await showInsertTextDialog(
        context,
        title: l.markerLabel,
        initial: row['text'] as String? ?? '',
      );
      if (text == null || !mounted) {
        return;
      }
      final t = text.toNativeUtf8();
      try {
        if (rust.eventSetText(json, t) == 1) {
          widget.onChanged();
          _reload();
        }
      } finally {
        calloc.free(t);
      }
      return;
    }
    if (kind == 'note') {
      final tick = (row['t'] as num?)?.toInt() ?? 0;
      final end = (row['end'] as num?)?.toInt() ?? tick + 1;
      final vel = (row['vel'] as num?)?.toInt() ?? 100;
      final result = await showNoteEditDialog(
        context,
        velocity: vel,
        gate: end - tick,
      );
      if (result == null || !mounted) {
        return;
      }
      if (rust.eventUpdate(
            json,
            result.$1.toDouble(),
            (tick + result.$2).toDouble(),
          ) ==
          1) {
        widget.onChanged();
        _reload();
      }
      return;
    }
    // 自动化 / Tempo：显示值编辑。
    final isTempo = kind == 'tempo';
    final max = isTempo ? 60000000 : ((row['max'] as num?)?.toInt() ?? 127);
    final value = ((row['val'] as num?)?.toDouble() ?? 0).round();
    final label = switch (kind) {
      'cc' => _ccName((row['num'] as num?)?.toInt() ?? 0),
      'rpn' => rpnInsertLabel((row['num'] as num?)?.toInt() ?? 0),
      'pb' => 'Pitch Bend',
      _ => 'Tempo',
    };
    final result = await showInsertValueDialog(
      context,
      title: '$label（${isTempo ? '0–60000000' : '0–$max'}）',
      min: isTempo ? 1 : 0,
      max: max,
      initial: value.clamp(0, max),
    );
    if (result == null || !mounted) {
      return;
    }
    if (rust.eventUpdate(json, result.toDouble(), 0) == 1) {
      widget.onChanged();
      _reload();
    }
  }

  /// 顶部音轨筛选：全部轨道在第一项（与 PR 下拉一致）。
  Widget _trackButton() {
    final label = _track < 0
        ? l.allTracks
        : '${_track.toString().padLeft(3, '0')}  ${widget.tracks[_track].name}';
    return PopupMenuButton<int>(
      tooltip: l.currentTrack,
      padding: EdgeInsets.zero,
      onSelected: (v) {
        if (v == _track) {
          return;
        }
        setState(() => _track = v);
        _reload();
      },
      itemBuilder: (_) => [
        PopupMenuItem<int>(
          enabled: false,
          height: 30,
          child: Text(
            l.currentTrack,
            style: TextStyle(color: YinheTheme.textLabel, fontSize: 11),
          ),
        ),
        CheckedPopupMenuItem<int>(
          value: -1,
          checked: _track < 0,
          child: Text(l.allTracks),
        ),
        const PopupMenuDivider(height: 1),
        for (var i = 0; i < widget.tracks.length; i++)
          CheckedPopupMenuItem<int>(
            value: i,
            checked: _track == i,
            child: Text(
              '${i.toString().padLeft(3, '0')}  ${widget.tracks[i].name}',
            ),
          ),
      ],
      child: Container(
        margin: const EdgeInsets.symmetric(vertical: 7),
        padding: const EdgeInsets.symmetric(horizontal: 10),
        constraints: const BoxConstraints(maxWidth: 180),
        alignment: Alignment.center,
        decoration: BoxDecoration(
          color: YinheTheme.btnBg,
          borderRadius: BorderRadius.circular(6),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Flexible(
              child: Text(
                label,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  color: _track < 0 ? YinheTheme.text : YinheTheme.accent,
                  fontSize: 12,
                ),
              ),
            ),
            Icon(
              Icons.arrow_drop_down,
              size: 18,
              color: YinheTheme.textSecondary,
            ),
          ],
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return Dialog.fullscreen(
      backgroundColor: YinheTheme.bg,
      child: SafeArea(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Container(
              height: 48,
              padding: const EdgeInsets.symmetric(horizontal: 6),
              color: YinheTheme.controlBg,
              child: Row(
                children: [
                  IconButton(
                    tooltip: l.close,
                    icon: const Icon(Icons.close),
                    color: YinheTheme.text,
                    onPressed: () => Navigator.of(context).pop(),
                  ),
                  const SizedBox(width: 4),
                  Text(
                    l.eventList,
                    style: TextStyle(color: YinheTheme.text, fontSize: 15),
                  ),
                  const SizedBox(width: 8),
                  _trackButton(),
                  Expanded(
                    child: Align(
                      alignment: Alignment.centerRight,
                      child: Padding(
                        padding: const EdgeInsets.only(right: 8),
                        child: Text(
                          l.sortHint,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            color: YinheTheme.textDisabled,
                            fontSize: 11,
                          ),
                        ),
                      ),
                    ),
                  ),
                ],
              ),
            ),
            Expanded(
              child: _rows.isEmpty
                  ? Center(
                      child: _loading
                          ? const CircularProgressIndicator()
                          : _done
                          ? Text(
                              l.noEvents,
                              style: TextStyle(
                                color: YinheTheme.textDisabled,
                                fontSize: 13,
                              ),
                            )
                          : TextButton(
                              onPressed: _loadMore,
                              child: Text(l.loadMore),
                            ),
                    )
                  : ListView.builder(
                      controller: _scroll,
                      itemCount: _rows.length + (_done ? 0 : 1),
                      itemBuilder: (context, i) {
                        if (i >= _rows.length) {
                          return Padding(
                            padding: const EdgeInsets.symmetric(vertical: 12),
                            child: Center(
                              child: _loading
                                  ? const SizedBox(
                                      width: 18,
                                      height: 18,
                                      child: CircularProgressIndicator(
                                        strokeWidth: 2,
                                      ),
                                    )
                                  : TextButton(
                                      onPressed: _loadMore,
                                      child: Text(l.loadMore),
                                    ),
                            ),
                          );
                        }
                        final row = _rows[i];
                        final d = _describe(row);
                        return ListTile(
                          dense: true,
                          leading: Icon(
                            d.icon,
                            size: 18,
                            color: YinheTheme.textSecondary,
                          ),
                          title: Text(
                            d.title,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: const TextStyle(fontSize: 14),
                          ),
                          subtitle: Text(
                            d.subtitle,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: TextStyle(
                              color: YinheTheme.textDisabled,
                              fontSize: 11,
                            ),
                          ),
                          onLongPress: () => _openActions(row),
                        );
                      },
                    ),
            ),
            Container(
              height: 44,
              color: YinheTheme.controlBg,
              child: Row(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  IconButton(
                    tooltip: l.prevPage,
                    icon: const Icon(Icons.chevron_left),
                    color: YinheTheme.textSecondary,
                    onPressed: _page > 1 ? _prevPage : null,
                  ),
                  Text(
                    '$_page / $_pageCount',
                    style: TextStyle(color: YinheTheme.textLabel, fontSize: 12),
                  ),
                  IconButton(
                    tooltip: l.nextPage,
                    icon: const Icon(Icons.chevron_right),
                    color: YinheTheme.textSecondary,
                    onPressed: (_page < _pageCount || !_done)
                        ? _nextPage
                        : null,
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// 音符编辑：力度 + 长度（tick）。
Future<(int, int)?> showNoteEditDialog(
  BuildContext context, {
  required int velocity,
  required int gate,
}) {
  final l = AppLocalizations.of(context);
  return showNoteInputDialog(
    context,
    title: l.editNote,
    velocityLabel: l.velocityRange,
    lengthLabel: l.lengthTicks,
    velocity: velocity,
    gate: gate,
    confirmText: l.save,
  );
}
