import 'dart:convert';
import 'dart:ffi' as ffi;

import 'package:ffi/ffi.dart';
import 'package:flutter/material.dart';

import '../core/ffi.dart' as rust;
import '../core/theme.dart';
import '../l10n.dart';
import 'frame.dart';

class LibraryItem {
  const LibraryItem({
    required this.path,
    required this.name,
    required this.artist,
    required this.album,
    required this.albumArtist,
    required this.year,
    required this.trackNo,
  });

  final String path;
  final String name;
  final String artist;
  final String album;
  final String albumArtist;
  final String year;
  final int trackNo;

  static LibraryItem? fromJson(Object? raw) {
    if (raw is! Map) {
      return null;
    }
    final path = raw['path'] as String? ?? '';
    if (path.isEmpty) {
      return null;
    }
    return LibraryItem(
      path: path,
      name: raw['name'] as String? ?? '',
      artist: raw['artist'] as String? ?? '',
      album: raw['album'] as String? ?? '',
      albumArtist: raw['album_artist'] as String? ?? '',
      year: raw['year'] as String? ?? '',
      trackNo: (raw['track_no'] as num?)?.toInt() ?? 0,
    );
  }
}

class LibraryGroup {
  const LibraryGroup({
    required this.album,
    required this.albumArtist,
    required this.items,
  });

  final String album;
  final String albumArtist;
  final List<LibraryItem> items;

  static LibraryGroup? fromJson(Object? raw) {
    if (raw is! Map) {
      return null;
    }
    final items = <LibraryItem>[];
    final list = raw['items'];
    if (list is List) {
      for (final e in list) {
        final item = LibraryItem.fromJson(e);
        if (item != null) {
          items.add(item);
        }
      }
    }
    return LibraryGroup(
      album: raw['album'] as String? ?? '',
      albumArtist: raw['album_artist'] as String? ?? '',
      items: items,
    );
  }
}

List<LibraryGroup> readLibrary() {
  final ptr = rust.libraryJson();
  if (ptr == ffi.nullptr) {
    return const [];
  }
  try {
    final data = jsonDecode(ptr.toDartString());
    if (data is! Map) {
      return const [];
    }
    final groups = <LibraryGroup>[];
    final list = data['groups'];
    if (list is List) {
      for (final e in list) {
        final g = LibraryGroup.fromJson(e);
        if (g != null && g.items.isNotEmpty) {
          groups.add(g);
        }
      }
    }
    return groups;
  } catch (_) {
    return const [];
  } finally {
    rust.freeString(ptr);
  }
}

/// 音乐库：按专辑分组的最近/已保存工程。
Future<String?> showLibraryDialog(BuildContext context) {
  final groups = readLibrary();
  return showDialog<String>(
    context: context,
    builder: (_) => _LibraryDialog(groups: groups),
  );
}

class _LibraryDialog extends StatelessWidget {
  const _LibraryDialog({required this.groups});

  final List<LibraryGroup> groups;

  @override
  Widget build(BuildContext context) {
    final zh = L10n.isZh;
    return YinheDialog(
      title: zh ? '音乐库' : 'Library',
      width: 520,
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(zh ? '关闭' : 'Close'),
        ),
      ],
      content: groups.isEmpty
          ? Padding(
              padding: const EdgeInsets.symmetric(vertical: 24),
              child: Text(
                zh ? '还没有保存过工程。' : 'No saved projects yet.',
                textAlign: TextAlign.center,
                style: TextStyle(color: YinheTheme.textSecondary),
              ),
            )
          : Column(
              mainAxisSize: MainAxisSize.min,
              children: [for (final g in groups) _GroupTile(group: g)],
            ),
    );
  }
}

class _GroupTile extends StatelessWidget {
  const _GroupTile({required this.group});

  final LibraryGroup group;

  @override
  Widget build(BuildContext context) {
    final zh = L10n.isZh;
    final album = group.album.isEmpty ? (zh ? '单曲' : 'Singles') : group.album;
    final subtitle = group.albumArtist.isEmpty ? null : group.albumArtist;
    return Card(
      margin: const EdgeInsets.only(bottom: 8),
      color: YinheTheme.controlBg,
      child: ExpansionTile(
        initiallyExpanded: true,
        leading: const Icon(Icons.album_outlined),
        title: Text(album, maxLines: 1, overflow: TextOverflow.ellipsis),
        subtitle: subtitle == null
            ? null
            : Text(subtitle, maxLines: 1, overflow: TextOverflow.ellipsis),
        children: [
          for (final item in group.items)
            ListTile(
              dense: true,
              leading: item.trackNo > 0
                  ? Text(
                      item.trackNo.toString().padLeft(2, '0'),
                      style: TextStyle(color: YinheTheme.textSecondary),
                    )
                  : const Icon(Icons.music_note, size: 18),
              title: Text(
                item.name,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
              subtitle: Text(
                [
                  if (item.artist.isNotEmpty) item.artist,
                  if (item.year.isNotEmpty) item.year,
                ].join(' · '),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(color: YinheTheme.textSecondary, fontSize: 12),
              ),
              onTap: () => Navigator.of(context).pop(item.path),
            ),
        ],
      ),
    );
  }
}
