import 'package:flutter/material.dart';

import '../core/theme.dart';
import '../l10n/app_localizations.dart';
import 'frame.dart';

/// 工程设置对话框（歌名/艺术家/PPQ/简介）。
class ProjectDialog extends StatefulWidget {
  const ProjectDialog({
    super.key,
    required this.noteCount,
    required this.initialName,
    required this.initialArtist,
    required this.initialDescription,
    required this.initialPpq,
    required this.onSave,
  });

  /// 全曲音符数（窗口内展示）。
  final int noteCount;
  final String initialName;
  final String initialArtist;
  final String initialDescription;
  final int initialPpq;
  final void Function(String name, String artist, String desc, int ppq) onSave;

  @override
  State<ProjectDialog> createState() => _ProjectDialogState();
}

class _ProjectDialogState extends State<ProjectDialog> {
  AppLocalizations get l => AppLocalizations.of(context);

  late final TextEditingController _name = TextEditingController(
    text: widget.initialName,
  );
  late final TextEditingController _artist = TextEditingController(
    text: widget.initialArtist,
  );
  late final TextEditingController _desc = TextEditingController(
    text: widget.initialDescription,
  );
  late final TextEditingController _ppq = TextEditingController(
    text: '${widget.initialPpq}',
  );
  String? _error;

  @override
  void dispose() {
    _name.dispose();
    _artist.dispose();
    _desc.dispose();
    _ppq.dispose();
    super.dispose();
  }

  void _save() {
    final ppq = int.tryParse(_ppq.text.trim());
    if (ppq == null || ppq < 1 || ppq > 32767) {
      setState(() => _error = l.ppqError);
      return;
    }
    widget.onSave(
      _name.text.trim(),
      _artist.text.trim(),
      _desc.text.trim(),
      ppq,
    );
    Navigator.of(context).pop();
  }

  @override
  Widget build(BuildContext context) {
    return YinheDialog(
      title: l.projectSettings,
      titleTrailing: Text(
        l.notesCount(widget.noteCount),
        style: TextStyle(color: YinheTheme.textLabel, fontSize: 12),
      ),
      width: 420,
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.cancel),
        ),
        FilledButton(onPressed: _save, child: Text(l.save)),
      ],
      content: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          _field(_name, l.songTitle),
          const SizedBox(height: 12),
          _field(_artist, l.artist),
          const SizedBox(height: 12),
          _field(_ppq, l.ppq, keyboardType: TextInputType.number),
          const SizedBox(height: 12),
          _field(_desc, l.description, maxLines: 3),
          if (_error != null)
            Padding(
              padding: const EdgeInsets.only(top: 10),
              child: Text(
                _error!,
                style: TextStyle(color: YinheTheme.danger, fontSize: 12),
              ),
            ),
        ],
      ),
    );
  }

  Widget _field(
    TextEditingController c,
    String label, {
    int maxLines = 1,
    TextInputType? keyboardType,
  }) {
    return TextField(
      controller: c,
      maxLines: maxLines,
      keyboardType: keyboardType,
      autocorrect: false,
      enableSuggestions: false,
      style: const TextStyle(fontSize: 14),
      decoration: dialogInputDecoration(label),
    );
  }
}
