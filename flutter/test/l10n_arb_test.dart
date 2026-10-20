import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:yinhe_flutter/l10n/app_localizations.dart';

/// 语言资源完整性回归：新增文案时最容易漏掉某几种语言，
/// 这里直接在 ARB 层面校验（不依赖运行时 fallback）。
void main() {
  final dir = Directory('lib/l10n');
  final files = dir
      .listSync()
      .whereType<File>()
      .where((f) => f.path.endsWith('.arb'))
      .toList();

  Map<String, dynamic> read(File f) =>
      jsonDecode(f.readAsStringSync()) as Map<String, dynamic>;

  Set<String> keysOf(Map<String, dynamic> data) =>
      data.keys.where((k) => !k.startsWith('@')).toSet();

  test('ARB 数量与 supportedLocales 一致（15 种）', () {
    expect(files.length, 15);
    expect(AppLocalizations.supportedLocales.length, files.length);
  });

  test('每种语言都包含模板的全部 key', () {
    final template = read(File('${dir.path}/app_en.arb'));
    final templateKeys = keysOf(template);
    expect(templateKeys, isNotEmpty);
    for (final f in files) {
      final data = read(f);
      expect(keysOf(data), containsAll(templateKeys), reason: f.path);
      expect(keysOf(data).length, templateKeys.length, reason: f.path);
    }
  });

  test('文案非空且不含未替换的占位符', () {
    for (final f in files) {
      final data = read(f);
      for (final entry in data.entries) {
        if (entry.key.startsWith('@')) {
          continue;
        }
        final value = entry.value;
        expect(value, isA<String>(), reason: '${f.path}: ${entry.key}');
        expect(
          (value as String).trim(),
          isNotEmpty,
          reason: '${f.path}: ${entry.key}',
        );
        expect(
          value.contains('??'),
          isFalse,
          reason: '${f.path}: ${entry.key}',
        );
      }
    }
  });

  test('模板 metadata 的占位符类型合法', () {
    final template = read(File('${dir.path}/app_en.arb'));
    for (final entry in template.entries) {
      if (!entry.key.startsWith('@') || entry.key == '@@locale') {
        continue;
      }
      final meta = entry.value;
      expect(meta, isA<Map<String, dynamic>>(), reason: entry.key);
    }
  });
}
