import 'package:flutter/material.dart';

import '../core/theme.dart';
import '../l10n.dart';
import '../l10n/app_localizations.dart';
import 'frame.dart';

/// 工程设置对话框（歌曲信息 / 专辑信息 / PPQ）。
class ProjectDialog extends StatefulWidget {
  const ProjectDialog({
    super.key,
    required this.noteCount,
    required this.initialName,
    required this.initialArtist,
    required this.initialArtists,
    required this.initialArtistWebsites,
    required this.initialArtistBio,
    required this.initialDescription,
    required this.initialAlbum,
    required this.initialAlbumArtist,
    required this.initialGenre,
    required this.initialYear,
    required this.initialTrackNo,
    required this.initialDiscNo,
    required this.initialCopyright,
    required this.initialPpq,
    required this.onSave,
  });

  /// 全曲音符数（窗口内展示）。
  final int noteCount;
  final String initialName;
  final String initialArtist;

  /// 多音乐人（逗号分隔，首位为主）。
  final String initialArtists;
  final String initialArtistWebsites;
  final String initialArtistBio;
  final String initialDescription;
  final String initialAlbum;
  final String initialAlbumArtist;
  final String initialGenre;
  final String initialYear;
  final int initialTrackNo;
  final int initialDiscNo;
  final String initialCopyright;
  final int initialPpq;
  final void Function(Map<String, dynamic> info) onSave;

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
  late final TextEditingController _artists = TextEditingController(
    text: widget.initialArtists,
  );
  late final TextEditingController _artistWebsites = TextEditingController(
    text: widget.initialArtistWebsites,
  );
  late final TextEditingController _artistBio = TextEditingController(
    text: widget.initialArtistBio,
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
  late final TextEditingController _copyright = TextEditingController(
    text: widget.initialCopyright,
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
    _artists.dispose();
    _artistWebsites.dispose();
    _artistBio.dispose();
    _album.dispose();
    _albumArtist.dispose();
    _genre.dispose();
    _year.dispose();
    _trackNo.dispose();
    _discNo.dispose();
    _copyright.dispose();
    _ppq.dispose();
    super.dispose();
  }

  int _number(TextEditingController c) {
    final v = int.tryParse(c.text.trim());
    return v == null || v < 0 ? 0 : v;
  }

  void _save() {
    final ppq = int.tryParse(_ppq.text.trim());
    if (ppq == null || ppq < 1 || ppq > 32767) {
      setState(() => _error = l.ppqError);
      return;
    }
    final artists = _artists.text
        .split(RegExp(r'[,，、]'))
        .map((e) => e.trim())
        .where((e) => e.isNotEmpty)
        .map((e) => {'id': '', 'name': e, 'role': ''})
        .toList();
    widget.onSave({
      'name': _name.text.trim(),
      'artist': _artist.text.trim(),
      'artists': artists,
      'artist_websites': _artistWebsites.text
          .split(RegExp(r'\r?\n'))
          .map((e) => e.trim())
          .where((e) => e.isNotEmpty)
          .toList(),
      'artist_bio': _artistBio.text.trim(),
      'description': _desc.text.trim(),
      'album': _album.text.trim(),
      'album_artist': _albumArtist.text.trim(),
      'genre': _genre.text.trim(),
      'year': _year.text.trim(),
      'track_no': _number(_trackNo),
      'disc_no': _number(_discNo),
      'copyright': _copyright.text.trim(),
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
          _field(_artist, l.artist),
          const SizedBox(height: 12),
          _field(
            _artists,
            L10n.isZh
                ? '音乐人列表（逗号分隔，首位为主）'
                : 'Artists (comma-separated, first is primary)',
          ),
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
          _field(_copyright, L10n.isZh ? '版权' : 'Copyright'),
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
      // 不要传 `enableSuggestions: false`：Flutter 会把它翻成
      // TYPE_TEXT_VARIATION_VISIBLE_PASSWORD，小米/搜狗等输入法会切成
      // "安全键盘"。只关自动纠错即可，联想候选保持正常。
      autocorrect: false,
      style: const TextStyle(fontSize: 14),
      decoration: dialogInputDecoration(label),
    );
  }
}
