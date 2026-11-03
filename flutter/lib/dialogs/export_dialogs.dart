import 'package:flutter/material.dart';

import '../core/theme.dart';
import '../l10n.dart';
import '../l10n/app_localizations.dart';
import 'frame.dart';

/// 音频导出选项：位深。返回 null = 取消。
Future<int?> showAudioExportOptions(BuildContext context) {
  final l = AppLocalizations.of(context);
  return showDialog<int>(
    context: context,
    builder: (_) => _AudioExportDialog(l: l),
  );
}

class _AudioExportDialog extends StatefulWidget {
  const _AudioExportDialog({required this.l});

  final AppLocalizations l;

  @override
  State<_AudioExportDialog> createState() => _AudioExportDialogState();
}

class _AudioExportDialogState extends State<_AudioExportDialog> {
  int _depth = 16;

  @override
  Widget build(BuildContext context) {
    final l = widget.l;
    return YinheDialog(
      title: l.exportAudio,
      width: 420,
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.cancel),
        ),
        FilledButton(
          onPressed: () => Navigator.of(context).pop(_depth),
          child: Text(l.exportAudio),
        ),
      ],
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            l.bitDepth,
            style: TextStyle(color: YinheTheme.textLabel, fontSize: 12),
          ),
          const SizedBox(height: 8),
          SegmentedButton<int>(
            segments: const [
              ButtonSegment(value: 16, label: Text('16-bit')),
              ButtonSegment(value: 24, label: Text('24-bit')),
              ButtonSegment(value: 32, label: Text('32-bit float')),
            ],
            selected: {_depth},
            onSelectionChanged: (s) => setState(() => _depth = s.first),
          ),
        ],
      ),
    );
  }
}

/// 压缩 MIDI 容器格式。
enum MidiArchiveFormat { sevenZ, zip }

/// 选择 7z / zip。返回 null = 取消。
Future<MidiArchiveFormat?> showMidiArchiveFormatDialog(BuildContext context) {
  final zh = L10n.isZh;
  return showDialog<MidiArchiveFormat>(
    context: context,
    builder: (context) {
      return YinheDialog(
        title: zh ? '导出压缩 MIDI' : 'Export compressed MIDI',
        width: 420,
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(),
            child: Text(zh ? '取消' : 'Cancel'),
          ),
        ],
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(
              zh ? '选择压缩格式：' : 'Choose a format:',
              style: TextStyle(color: YinheTheme.textSecondary, fontSize: 12),
            ),
            const SizedBox(height: 10),
            ListTile(
              leading: const Icon(Icons.archive_outlined),
              title: const Text('7z (LZMA2)'),
              subtitle: Text(
                zh ? '体积最小，推荐' : 'Smallest, recommended',
                style: const TextStyle(fontSize: 12),
              ),
              onTap: () => Navigator.of(context).pop(MidiArchiveFormat.sevenZ),
            ),
            ListTile(
              leading: const Icon(Icons.folder_zip_outlined),
              title: const Text('ZIP (Deflate)'),
              subtitle: Text(
                zh ? '兼容性最好，但 MIDI 压缩率差' : 'Best compatibility, larger files',
                style: const TextStyle(fontSize: 12),
              ),
              onTap: () => Navigator.of(context).pop(MidiArchiveFormat.zip),
            ),
          ],
        ),
      );
    },
  );
}
