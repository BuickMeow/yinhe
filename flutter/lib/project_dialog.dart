import 'package:flutter/material.dart';

import 'theme.dart';

/// 工程设置对话框（歌名/艺术家/PPQ/简介）。
class ProjectDialog extends StatefulWidget {
  const ProjectDialog({
    super.key,
    required this.initialName,
    required this.initialArtist,
    required this.initialDescription,
    required this.initialPpq,
    required this.onSave,
  });

  final String initialName;
  final String initialArtist;
  final String initialDescription;
  final int initialPpq;
  final void Function(String name, String artist, String desc, int ppq) onSave;

  @override
  State<ProjectDialog> createState() => _ProjectDialogState();
}

class _ProjectDialogState extends State<ProjectDialog> {
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
      setState(() => _error = 'PPQ 需为 1..32767 的整数');
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
    return AlertDialog(
      backgroundColor: YinheTheme.bg,
      title: const Text('工程设置', style: TextStyle(fontSize: 18)),
      content: SizedBox(
        width: 420,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              _field(_name, '歌名'),
              const SizedBox(height: 12),
              _field(_artist, '艺术家'),
              const SizedBox(height: 12),
              _field(
                _ppq,
                'PPQ（已加载音符时修改会缩放全部时间）',
                keyboardType: TextInputType.number,
              ),
              const SizedBox(height: 12),
              _field(_desc, '简介', maxLines: 3),
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
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('取消'),
        ),
        FilledButton(onPressed: _save, child: const Text('保存')),
      ],
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
      style: const TextStyle(fontSize: 14),
      decoration: InputDecoration(
        labelText: label,
        border: const OutlineInputBorder(),
        isDense: true,
      ),
    );
  }
}
