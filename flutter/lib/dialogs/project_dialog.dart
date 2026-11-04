import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../core/theme.dart';
import '../l10n.dart';
import '../l10n/app_localizations.dart';
import 'frame.dart';

/// 工程设置对话框（歌曲信息 / 艺术家列表 / 专辑信息 / PPQ / 工作时间）。
class ProjectDialog extends StatefulWidget {
  const ProjectDialog({
    super.key,
    required this.noteCount,
    required this.initialName,
    required this.initialArtists,
    required this.initialDescription,
    required this.initialAlbum,
    required this.initialAlbumArtist,
    required this.initialGenre,
    required this.initialYear,
    required this.initialTrackNo,
    required this.initialDiscNo,
    required this.initialPpq,
    required this.workTime,
    required this.onSave,
  });

  /// 全曲音符数（窗口内展示）。
  final int noteCount;
  final String initialName;

  /// 有序艺术家列表（第一项为主）；每项是 ArtistRef 的 JSON 形态。
  final List<Map<String, dynamic>> initialArtists;

  final String initialDescription;
  final String initialAlbum;
  final String initialAlbumArtist;
  final String initialGenre;
  final String initialYear;
  final int initialTrackNo;
  final int initialDiscNo;
  final int initialPpq;

  /// 正在累计的前台工作时间（秒）；对话框内实时刷新。
  final ValueListenable<int> workTime;

  final void Function(Map<String, dynamic> info) onSave;

  @override
  State<ProjectDialog> createState() => _ProjectDialogState();
}

/// 单个艺术家的编辑控制器。
class _ArtistDraft {
  _ArtistDraft(Map<String, dynamic> json)
    : id = json['id'] as String? ?? '',
      name = TextEditingController(text: json['name'] as String? ?? ''),
      role = TextEditingController(text: json['role'] as String? ?? ''),
      websites = TextEditingController(
        text: ((json['websites'] as List?) ?? const [])
            .whereType<String>()
            .join('\n'),
      ),
      bio = TextEditingController(text: json['bio'] as String? ?? '');

  _ArtistDraft.empty()
    : id = '',
      name = TextEditingController(),
      role = TextEditingController(),
      websites = TextEditingController(),
      bio = TextEditingController();

  final String id;
  final TextEditingController name;
  final TextEditingController role;
  final TextEditingController websites;
  final TextEditingController bio;

  Map<String, dynamic> toJson() => {
    'id': id,
    'name': name.text.trim(),
    'role': role.text.trim(),
    'websites': websites.text
        .split(RegExp(r'\r?\n'))
        .map((e) => e.trim())
        .where((e) => e.isNotEmpty)
        .toList(),
    'bio': bio.text.trim(),
  };

  void dispose() {
    name.dispose();
    role.dispose();
    websites.dispose();
    bio.dispose();
  }
}

class _ProjectDialogState extends State<ProjectDialog> {
  AppLocalizations get l => AppLocalizations.of(context);

  late final TextEditingController _name = TextEditingController(
    text: widget.initialName,
  );
  late final TextEditingController _desc = TextEditingController(
    text: widget.initialDescription,
  );
  late final TextEditingController _album = TextEditingController(
    text: widget.initialAlbum,
  );
  late final TextEditingController _albumArtist = TextEditingController(
    text: widget.initialAlbumArtist,
  );
  late final TextEditingController _genre = TextEditingController(
    text: widget.initialGenre,
  );
  late final TextEditingController _year = TextEditingController(
    text: widget.initialYear,
  );
  late final TextEditingController _trackNo = TextEditingController(
    text: widget.initialTrackNo > 0 ? '${widget.initialTrackNo}' : '',
  );
  late final TextEditingController _discNo = TextEditingController(
    text: widget.initialDiscNo > 0 ? '${widget.initialDiscNo}' : '',
  );
  late final TextEditingController _ppq = TextEditingController(
    text: '${widget.initialPpq}',
  );
  final List<_ArtistDraft> _artists = [];
  String? _error;

  @override
  void initState() {
    super.initState();
    for (final a in widget.initialArtists) {
      _artists.add(_ArtistDraft(a));
    }
  }

  @override
  void dispose() {
    _name.dispose();
    _desc.dispose();
    _album.dispose();
    _albumArtist.dispose();
    _genre.dispose();
    _year.dispose();
    _trackNo.dispose();
    _discNo.dispose();
    _ppq.dispose();
    for (final a in _artists) {
      a.dispose();
    }
    super.dispose();
  }

  int _number(TextEditingController c) {
    final v = int.tryParse(c.text.trim());
    return v == null || v < 0 ? 0 : v;
  }

  void _addArtist() {
    setState(() => _artists.add(_ArtistDraft.empty()));
  }

  void _removeArtist(int index) {
    setState(() {
      _artists.removeAt(index).dispose();
    });
  }

  void _save() {
    final ppq = int.tryParse(_ppq.text.trim());
    if (ppq == null || ppq < 1 || ppq > 32767) {
      setState(() => _error = l.ppqError);
      return;
    }
    final artists = [
      for (final a in _artists)
        if (a.name.text.trim().isNotEmpty) a.toJson(),
    ];
    widget.onSave({
      'name': _name.text.trim(),
      'artist': artists.isEmpty ? '' : artists.first['name'] as String,
      'artists': artists,
      'description': _desc.text.trim(),
      'album': _album.text.trim(),
      'album_artist': _albumArtist.text.trim(),
      'genre': _genre.text.trim(),
      'year': _year.text.trim(),
      'track_no': _number(_trackNo),
      'disc_no': _number(_discNo),
      'ppq': ppq,
    });
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
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _field(_name, l.songTitle),
          const SizedBox(height: 12),
          Row(
            children: [
              Text(
                L10n.isZh ? '艺术家 / 版权' : 'Artists / Copyright',
                style: TextStyle(
                  color: YinheTheme.text,
                  fontSize: 13,
                  fontWeight: FontWeight.w600,
                ),
              ),
              const Spacer(),
              TextButton.icon(
                onPressed: _addArtist,
                icon: const Icon(Icons.add, size: 16),
                label: Text(
                  L10n.isZh ? '添加艺术家' : 'Add artist',
                  style: const TextStyle(fontSize: 12),
                ),
              ),
            ],
          ),
          if (_artists.isEmpty)
            Padding(
              padding: const EdgeInsets.only(bottom: 8),
              child: Text(
                L10n.isZh ? '未设置艺术家' : 'No artists',
                style: TextStyle(color: YinheTheme.textMuted, fontSize: 12),
              ),
            ),
          for (var i = 0; i < _artists.length; i++) _artistCard(i, _artists[i]),
          const SizedBox(height: 12),
          // PPQ 保持在第三个主要字段：名称 → 艺术家 → PPQ。
          _field(_ppq, l.ppq, keyboardType: TextInputType.number),
          const SizedBox(height: 12),
          _workTimeRow(),
          const SizedBox(height: 12),
          _field(_album, L10n.isZh ? '专辑' : 'Album'),
          const SizedBox(height: 12),
          _field(_albumArtist, L10n.isZh ? '专辑艺术家' : 'Album artist'),
          const SizedBox(height: 12),
          _field(_genre, L10n.isZh ? '流派' : 'Genre'),
          const SizedBox(height: 12),
          _field(_year, L10n.isZh ? '年份' : 'Year'),
          const SizedBox(height: 12),
          Row(
            children: [
              Expanded(
                child: _field(
                  _trackNo,
                  L10n.isZh ? '音轨号' : 'Track no.',
                  keyboardType: TextInputType.number,
                ),
              ),
              const SizedBox(width: 12),
              Expanded(
                child: _field(
                  _discNo,
                  L10n.isZh ? '碟号' : 'Disc no.',
                  keyboardType: TextInputType.number,
                ),
              ),
            ],
          ),
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

  Widget _artistCard(int index, _ArtistDraft artist) {
    return Card(
      margin: const EdgeInsets.only(bottom: 8),
      elevation: 0,
      color: YinheTheme.controlBg,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(6)),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(10, 6, 10, 10),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Row(
              children: [
                Text(
                  '${L10n.isZh ? '艺术家' : 'Artist'} ${index + 1}',
                  style: TextStyle(
                    color: YinheTheme.text,
                    fontSize: 13,
                    fontWeight: FontWeight.w600,
                  ),
                ),
                if (index == 0)
                  Padding(
                    padding: const EdgeInsets.only(left: 8),
                    child: Text(
                      L10n.isZh ? '主' : 'Primary',
                      style: TextStyle(color: YinheTheme.accent, fontSize: 11),
                    ),
                  ),
                const Spacer(),
                IconButton(
                  tooltip: L10n.isZh ? '删除艺术家' : 'Remove artist',
                  icon: const Icon(Icons.delete_outline, size: 18),
                  color: YinheTheme.danger,
                  visualDensity: VisualDensity.compact,
                  onPressed: () => _removeArtist(index),
                ),
              ],
            ),
            _field(artist.name, L10n.isZh ? '名称' : 'Name', dense: true),
            const SizedBox(height: 8),
            _field(
              artist.role,
              L10n.isZh ? '角色（可选，如 main / feat.）' : 'Role (optional)',
              dense: true,
            ),
            const SizedBox(height: 8),
            _field(
              artist.websites,
              L10n.isZh ? '网站（每行一条）' : 'Websites (one per line)',
              maxLines: 2,
              dense: true,
            ),
            const SizedBox(height: 8),
            _field(
              artist.bio,
              L10n.isZh ? '介绍' : 'Bio',
              maxLines: 2,
              dense: true,
            ),
          ],
        ),
      ),
    );
  }

  Widget _workTimeRow() {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
      decoration: BoxDecoration(
        color: YinheTheme.controlBg,
        borderRadius: BorderRadius.circular(6),
      ),
      child: Row(
        children: [
          Text(
            L10n.isZh ? '工作时间' : 'Work time',
            style: TextStyle(color: YinheTheme.textSecondary, fontSize: 13),
          ),
          const Spacer(),
          ValueListenableBuilder<int>(
            valueListenable: widget.workTime,
            builder: (context, secs, _) => Text(
              _formatWorkTime(secs),
              style: TextStyle(
                color: YinheTheme.accent,
                fontSize: 14,
                fontFeatures: const [FontFeature.tabularFigures()],
              ),
            ),
          ),
        ],
      ),
    );
  }

  String _formatWorkTime(int secs) {
    final h = secs ~/ 3600;
    final m = (secs % 3600) ~/ 60;
    final s = secs % 60;
    return '${h.toString().padLeft(2, '0')}:'
        '${m.toString().padLeft(2, '0')}:'
        '${s.toString().padLeft(2, '0')}';
  }

  Widget _field(
    TextEditingController c,
    String label, {
    int maxLines = 1,
    TextInputType? keyboardType,
    bool dense = false,
  }) {
    return TextField(
      controller: c,
      maxLines: maxLines,
      keyboardType: keyboardType,
      // 不要传 `enableSuggestions: false`：Flutter 会把它翻成
      // TYPE_TEXT_VARIATION_VISIBLE_PASSWORD，小米/搜狗等输入法会切成
      // "安全键盘"。只关自动纠错即可，联想候选保持正常。
      autocorrect: false,
      style: const TextStyle(fontSize: 14),
      decoration: dialogInputDecoration(label, isDense: dense),
    );
  }
}
