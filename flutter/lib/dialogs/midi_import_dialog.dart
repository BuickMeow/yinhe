import 'dart:convert';

import 'package:flutter/material.dart';

import '../core/theme.dart';
import '../l10n.dart';
import 'frame.dart';

/// 一条源轨道（来自 Rust 解析摘要）。
class MidiImportTrack {
  const MidiImportTrack({
    required this.name,
    required this.port,
    required this.channel,
    required this.notes,
    required this.automation,
  });

  final String name;
  final int port;
  final int channel;
  final int notes;
  final bool automation;

  static MidiImportTrack? fromJson(Object? raw) {
    if (raw is! Map) {
      return null;
    }
    return MidiImportTrack(
      name: raw['name'] as String? ?? '',
      port: (raw['port'] as num?)?.toInt() ?? 0,
      channel: (raw['channel'] as num?)?.toInt() ?? 0,
      notes: (raw['notes'] as num?)?.toInt() ?? 0,
      automation: raw['automation'] as bool? ?? false,
    );
  }
}

/// 解析摘要。
class MidiImportSummary {
  const MidiImportSummary({
    required this.tracks,
    required this.ppq,
    required this.noteCount,
    required this.lengthTicks,
    required this.tempoEvents,
    required this.timeSigEvents,
  });

  final List<MidiImportTrack> tracks;
  final int ppq;
  final int noteCount;
  final int lengthTicks;
  final int tempoEvents;
  final int timeSigEvents;

  static MidiImportSummary? fromJson(Object? raw) {
    if (raw is! Map) {
      return null;
    }
    final tracks = <MidiImportTrack>[];
    final list = raw['tracks'];
    if (list is List) {
      for (final t in list) {
        final track = MidiImportTrack.fromJson(t);
        if (track != null) {
          tracks.add(track);
        }
      }
    }
    return MidiImportSummary(
      tracks: tracks,
      ppq: (raw['ppq'] as num?)?.toInt() ?? 480,
      noteCount: (raw['note_count'] as num?)?.toInt() ?? 0,
      lengthTicks: (raw['length_ticks'] as num?)?.toInt() ?? 0,
      tempoEvents: (raw['tempo_events'] as num?)?.toInt() ?? 0,
      timeSigEvents: (raw['time_sig_events'] as num?)?.toInt() ?? 0,
    );
  }

  static MidiImportSummary? tryDecode(String raw) {
    try {
      return fromJson(jsonDecode(raw));
    } catch (_) {
      return null;
    }
  }
}

/// 用户选择结果。
class MidiImportChoice {
  const MidiImportChoice({required this.include, required this.allocateFree});

  final List<bool> include;
  final bool allocateFree;
}

/// 导入确认框：摘要 + 逐轨勾选 + 通道策略。
///
/// 默认全选、保留源通道、追加到工程末尾；用户直接点「导入」即可。
Future<MidiImportChoice?> showMidiImportDialog(
  BuildContext context,
  MidiImportSummary summary,
) {
  return showDialog<MidiImportChoice>(
    context: context,
    barrierDismissible: true,
    builder: (_) => _MidiImportDialog(summary: summary),
  );
}

class _MidiImportDialog extends StatefulWidget {
  const _MidiImportDialog({required this.summary});

  final MidiImportSummary summary;

  @override
  State<_MidiImportDialog> createState() => _MidiImportDialogState();
}

class _MidiImportDialogState extends State<_MidiImportDialog> {
  late final List<bool> _include = List<bool>.filled(
    widget.summary.tracks.length,
    true,
  );
  bool _allocateFree = false;

  String get _tracksLine {
    final total = widget.summary.tracks.length;
    final selected = _include.where((v) => v).length;
    return L10n.isZh
        ? '$total 条源轨，已选 $selected 条'
        : '$total source tracks, $selected selected';
  }

  @override
  Widget build(BuildContext context) {
    final s = widget.summary;
    return YinheDialog(
      title: L10n.isZh ? '导入 MIDI' : 'Import MIDI',
      width: 560,
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(L10n.isZh ? '取消' : 'Cancel'),
        ),
        FilledButton(
          onPressed: _include.any((v) => v)
              ? () => Navigator.of(context).pop(
                  MidiImportChoice(
                    include: List.of(_include),
                    allocateFree: _allocateFree,
                  ),
                )
              : null,
          child: Text(L10n.isZh ? '导入' : 'Import'),
        ),
      ],
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            L10n.isZh
                ? '插入位置：工程末尾（速度/拍号也接在末尾）'
                : 'Insert at project end (tempo/time signature too)',
            style: TextStyle(color: YinheTheme.textSecondary, fontSize: 13),
          ),
          const SizedBox(height: 4),
          Text(
            '${s.noteCount} notes · PPQ ${s.ppq} · '
            '${s.tempoEvents} tempo · ${s.timeSigEvents} time sig',
            style: TextStyle(color: YinheTheme.textSecondary, fontSize: 12),
          ),
          const SizedBox(height: 10),
          Text(_tracksLine, style: const TextStyle(fontSize: 13)),
          const SizedBox(height: 6),
          Container(
            height: 240,
            decoration: BoxDecoration(
              color: YinheTheme.controlBg,
              borderRadius: BorderRadius.circular(6),
              border: Border.all(color: YinheTheme.lineFg),
            ),
            child: s.tracks.isEmpty
                ? Center(
                    child: Text(
                      L10n.isZh ? '没有可导入的音轨' : 'No importable tracks',
                      style: TextStyle(color: YinheTheme.textSecondary),
                    ),
                  )
                : ListView.builder(
                    itemCount: s.tracks.length,
                    itemBuilder: (context, i) {
                      final t = s.tracks[i];
                      final ch =
                          '${String.fromCharCode(65 + t.port)}${t.channel + 1}';
                      return CheckboxListTile(
                        dense: true,
                        value: _include[i],
                        onChanged: (v) =>
                            setState(() => _include[i] = v ?? false),
                        controlAffinity: ListTileControlAffinity.leading,
                        title: Text(
                          t.name.isEmpty ? 'Track ${i + 1}' : t.name,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: const TextStyle(fontSize: 13),
                        ),
                        subtitle: Text(
                          'CH $ch · ${t.notes}${t.automation ? ' · CC' : ''}',
                          style: TextStyle(
                            fontSize: 11,
                            color: YinheTheme.textSecondary,
                          ),
                        ),
                      );
                    },
                  ),
          ),
          const SizedBox(height: 8),
          RadioGroup<bool>(
            groupValue: _allocateFree,
            onChanged: (v) => setState(() => _allocateFree = v ?? false),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                RadioListTile<bool>(
                  dense: true,
                  value: false,
                  title: Text(
                    L10n.isZh
                        ? '保留源通道（可能共享乐器）'
                        : 'Keep source channels (share instruments)',
                    style: const TextStyle(fontSize: 13),
                  ),
                ),
                RadioListTile<bool>(
                  dense: true,
                  value: true,
                  title: Text(
                    L10n.isZh
                        ? '分配空闲通道（音色隔离）'
                        : 'Assign free channels (isolate instruments)',
                    style: const TextStyle(fontSize: 13),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
