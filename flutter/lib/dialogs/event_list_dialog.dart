import 'dart:async';
import 'dart:convert';
import 'dart:ffi' as ffi;

import 'package:ffi/ffi.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

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
  required List<TrackRow> tracks,
  required int initialTrack,
  required void Function(int track, double tick) onJump,
  required VoidCallback onChanged,
}) {
  return showDialog<void>(
    context: context,
    builder: (_) => EventListDialog(
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
    required this.tracks,
    required this.initialTrack,
    required this.onJump,
    required this.onChanged,
  });

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

  /// 已加载的页（0-based）；翻回上一页直接复用，不重新请求。
  final Map<int, List<Map<String, dynamic>>> _pages = {};
  final TextEditingController _pageCtrl = TextEditingController(text: '1');

  /// 下一页的 Rust 游标：从 `_cursor` 的第 `_skip` 个事件开始。
  /// skip 只在同一 tick 内累加，避免一拍黑乐谱被 300 行截断。
  double _cursor = 0;
  int _skip = 0;
  bool _loading = false;
  bool _done = false;
  int _total = 0;
  int _page = 0; // 0-based
  int _generation = 0;
  Map<String, dynamic>? _copied;

  /// 当前筛选轨；-1 = 全部轨道。
  late int _track =
      widget.initialTrack >= 0 && widget.initialTrack < widget.tracks.length
      ? widget.initialTrack
      : -1;

  List<Map<String, dynamic>> get _rows => _pages[_page] ?? const [];

  /// 真实总页数：由 Rust 返回的事件总数计算，而不是滚动高度。
  int get _pageCount {
    if (_total <= 0) {
      return 1;
    }
    return (_total + _pageSize - 1) ~/ _pageSize;
  }

  @override
  void initState() {
    super.initState();
    unawaited(_ensurePage(0));
  }

  @override
  void dispose() {
    _pageCtrl.dispose();
    super.dispose();
  }

  ({int total, List<Map<String, dynamic>> rows, int? nextTick, int nextSkip})?
  _fetchPage() {
    final ptr = rust.eventsJson(_cursor, _pageSize, _track, _skip);
    if (ptr == ffi.nullptr) {
      return null;
    }
    try {
      final decoded = jsonDecode(ptr.toDartString()) as Map<String, dynamic>;
      final total = (decoded['total'] as num?)?.toInt() ?? 0;
      final raw = decoded['rows'];
      final rows = <Map<String, dynamic>>[];
      if (raw is List) {
        for (final it in raw) {
          if (it is Map<String, dynamic>) {
            rows.add(it);
          }
        }
      }
      final next = decoded['next'];
      int? nextTick;
      var nextSkip = 0;
      if (next is Map) {
        nextTick = (next['tick'] as num?)?.toInt();
        nextSkip = (next['skip'] as num?)?.toInt() ?? 0;
      }
      return (total: total, rows: rows, nextTick: nextTick, nextSkip: nextSkip);
    } catch (_) {
      return null;
    } finally {
      rust.freeString(ptr);
    }
  }

  /// 取下一页真实数据；返回 false = 没有更多事件。
  bool _loadNextPage() {
    final page = _fetchPage();
    if (page == null) {
      _done = true;
      return false;
    }
    _total = page.total;
    if (page.rows.isEmpty) {
      _done = true;
      return false;
    }
    _pages[_pages.length] = page.rows;
    final nextTick = page.nextTick;
    if (nextTick == null) {
      _done = true;
    } else {
      _cursor = nextTick.toDouble();
      _skip = page.nextSkip;
    }
    return true;
  }

  /// 确保第 `index` 页已加载。跨度大时逐页取，并在页间让出事件循环。
  Future<void> _ensurePage(int index) async {
    if (_loading) {
      return;
    }
    final gen = _generation;
    setState(() => _loading = true);
    try {
      while (mounted &&
          gen == _generation &&
          !_done &&
          _pages.length <= index) {
        final ok = _loadNextPage();
        if (!mounted || gen != _generation) {
          return;
        }
        if (!ok) {
          break;
        }
        setState(() {});
        if (_pages.length <= index) {
          await Future<void>.delayed(Duration.zero);
        }
      }
    } finally {
      if (mounted && gen == _generation) {
        setState(() {
          _loading = false;
          final last = _pages.isEmpty ? 0 : _pages.length - 1;
          if (_page > last) {
            _page = last;
          }
          _syncPageCtrl();
        });
      }
    }
  }

  void _syncPageCtrl() {
    final text = '${_page + 1}';
    if (_pageCtrl.text != text) {
      _pageCtrl.text = text;
    }
  }

  Future<void> _goToPage(int number) async {
    if (_loading) {
      return;
    }
    final target = number.clamp(1, _pageCount) - 1;
    if (target == _page && _pages.containsKey(target)) {
      _syncPageCtrl();
      return;
    }
    setState(() {
      _page = target;
      _syncPageCtrl();
    });
    if (!_pages.containsKey(target)) {
      await _ensurePage(target);
    }
  }

  void _submitPage(String raw) {
    final page = int.tryParse(raw.trim());
    if (page == null) {
      _syncPageCtrl();
      return;
    }
    FocusScope.of(context).unfocus();
    unawaited(_goToPage(page));
  }

  void _reload() {
    _generation++;
    setState(() {
      _pages.clear();
      _cursor = 0;
      _skip = 0;
      _done = false;
      _total = 0;
      _page = 0;
      _loading = false;
      _pageCtrl.text = '1';
    });
    unawaited(_ensurePage(0));
  }

  String _noteName(int key) => MidiNames.instance.noteName(key);

  String _ccName(int cc) => ccInsertLabel(cc);

  ({String title, String subtitle, IconData icon}) _describe(
    Map<String, dynamic> r,
  ) {
    final kind = r['k'] as String? ?? '';
    final tick = (r['t'] as num?)?.toInt() ?? 0;
    final track = r['tn'] as String? ?? '';
    final time = r['tt'] as String? ?? '$tick';
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

  Widget _pageControls() {
    final canPrev = _page > 0 && !_loading;
    final canNext = _page + 1 < _pageCount && !_loading;
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        IconButton(
          tooltip: l.prevPage,
          icon: const Icon(Icons.chevron_left),
          color: YinheTheme.textSecondary,
          visualDensity: VisualDensity.compact,
          padding: EdgeInsets.zero,
          constraints: const BoxConstraints.tightFor(width: 34, height: 34),
          onPressed: canPrev ? () => unawaited(_goToPage(_page)) : null,
        ),
        SizedBox(
          width: 46,
          height: 28,
          child: TextField(
            controller: _pageCtrl,
            enabled: !_loading,
            textAlign: TextAlign.center,
            keyboardType: TextInputType.number,
            inputFormatters: [FilteringTextInputFormatter.digitsOnly],
            style: const TextStyle(fontSize: 12),
            decoration: const InputDecoration(
              isDense: true,
              border: OutlineInputBorder(),
              contentPadding: EdgeInsets.symmetric(horizontal: 4, vertical: 4),
            ),
            onSubmitted: _submitPage,
            onTapOutside: (_) => _syncPageCtrl(),
          ),
        ),
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 4),
          child: Text(
            '/ $_pageCount',
            style: TextStyle(color: YinheTheme.textLabel, fontSize: 12),
          ),
        ),
        IconButton(
          tooltip: l.nextPage,
          icon: const Icon(Icons.chevron_right),
          color: YinheTheme.textSecondary,
          visualDensity: VisualDensity.compact,
          padding: EdgeInsets.zero,
          constraints: const BoxConstraints.tightFor(width: 34, height: 34),
          onPressed: canNext ? () => unawaited(_goToPage(_page + 2)) : null,
        ),
      ],
    );
  }

  @override
  Widget build(BuildContext context) {
    final rows = _rows;
    return Dialog.fullscreen(
      backgroundColor: YinheTheme.bg,
      child: SafeArea(
        top: false,
        bottom: false,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Container(
              height: 42,
              decoration: BoxDecoration(
                color: YinheTheme.controlBg,
                border: Border(bottom: BorderSide(color: YinheTheme.lineFg)),
              ),
              child: Stack(
                children: [
                  Row(
                    children: [
                      const SizedBox(width: 6),
                      SizedBox(
                        width: 40,
                        height: 40,
                        child: IconButton(
                          tooltip: l.back,
                          icon: const Icon(Icons.arrow_back),
                          color: YinheTheme.text,
                          visualDensity: VisualDensity.compact,
                          padding: EdgeInsets.zero,
                          constraints: const BoxConstraints(
                            minWidth: 40,
                            minHeight: 40,
                          ),
                          onPressed: () => Navigator.of(context).pop(),
                        ),
                      ),
                      const SizedBox(width: 4),
                      Text(
                        l.eventList,
                        style: TextStyle(color: YinheTheme.text, fontSize: 15),
                      ),
                      const Spacer(),
                      _pageControls(),
                      const SizedBox(width: 6),
                    ],
                  ),
                  Align(alignment: Alignment.center, child: _trackButton()),
                ],
              ),
            ),
            Expanded(
              child: rows.isEmpty
                  ? Center(
                      child: _loading
                          ? const CircularProgressIndicator()
                          : Text(
                              l.noEvents,
                              style: TextStyle(
                                color: YinheTheme.textDisabled,
                                fontSize: 13,
                              ),
                            ),
                    )
                  : ListView.builder(
                      key: ValueKey(_page),
                      itemCount: rows.length,
                      itemBuilder: (context, i) {
                        final row = rows[i];
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
