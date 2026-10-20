import 'dart:convert';
import 'dart:ffi' as ffi;

import 'package:ffi/ffi.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import 'ffi.dart' as rust;
import 'l10n.dart';
import 'l10n/app_localizations.dart';
import 'theme.dart';

/// 设置页（全屏浮层）：左侧栏切页（音色库 / 音频）+ 右内容两栏。
///
/// 无上边栏；返回按钮与工作区顶栏的主页/返回按钮同 xy、同尺寸。
/// 音色库为多库表：可拖动排序（顶部覆盖底部）、可启用/关闭。
class SettingsView extends StatefulWidget {
  const SettingsView({
    super.key,
    required this.revision,
    required this.onPickSoundFont,
    required this.onBack,
  });

  /// 音色库表修订号（原生选择完成后 +1，重新读取列表）。
  final ValueListenable<int> revision;
  final VoidCallback onPickSoundFont;
  final VoidCallback onBack;

  @override
  State<SettingsView> createState() => _SettingsViewState();
}

/// 一行音色库。
class _SfItem {
  _SfItem({required this.path, required this.name, required this.enabled});

  final String path;
  final String name;
  bool enabled;
}

class _SettingsViewState extends State<SettingsView> {
  AppLocalizations get l => AppLocalizations.of(context);

  int _page = 0;
  List<_SfItem> _fonts = const [];

  /// 音频设置（与 Rust 同步；默认值同桌面/引擎回退）。
  int _maxVoices = 1024;
  int _bufferSize = 2048;
  int _ignoreVelocity = 1;

  @override
  void initState() {
    super.initState();
    _loadAudioSettings();
    _loadFonts();
    widget.revision.addListener(_loadFonts);
  }

  @override
  void dispose() {
    widget.revision.removeListener(_loadFonts);
    super.dispose();
  }

  // ── 音色库 ──

  void _loadFonts() {
    final ptr = rust.soundfontsJson();
    if (ptr == ffi.nullptr) {
      return;
    }
    try {
      final list = jsonDecode(ptr.toDartString()) as List<dynamic>;
      if (!mounted) {
        return;
      }
      setState(() {
        _fonts = [
          for (final it in list)
            if (it is Map<String, dynamic>)
              _SfItem(
                path: it['path'] as String? ?? '',
                name: it['name'] as String? ?? '',
                enabled: it['enabled'] as bool? ?? true,
              ),
        ];
      });
    } catch (_) {
      // 解析失败保持旧列表。
    } finally {
      rust.freeString(ptr);
    }
  }

  /// 把当前列表（顺序 = 优先级）写回 Rust 并持久化。
  void _applyFonts() {
    final json = jsonEncode([
      for (final e in _fonts) {'path': e.path, 'enabled': e.enabled},
    ]);
    final p = json.toNativeUtf8();
    try {
      rust.setSoundfonts(p);
    } finally {
      calloc.free(p);
    }
  }

  void _reorderFonts(int oldIndex, int newIndex) {
    setState(() {
      final e = _fonts.removeAt(oldIndex);
      _fonts.insert(newIndex, e);
    });
    _applyFonts();
  }

  void _toggleFont(int index, bool enabled) {
    setState(() => _fonts[index].enabled = enabled);
    _applyFonts();
  }

  void _removeFont(int index) {
    setState(() => _fonts.removeAt(index));
    _applyFonts();
  }

  // ── 音频设置 ──

  void _loadAudioSettings() {
    final ptr = rust.audioSettingsJson();
    if (ptr == ffi.nullptr) {
      return;
    }
    try {
      final json = jsonDecode(ptr.toDartString()) as Map<String, dynamic>;
      setState(() {
        _maxVoices = (json['max_voices'] as num?)?.toInt() ?? _maxVoices;
        _bufferSize = (json['buffer_size'] as num?)?.toInt() ?? _bufferSize;
        _ignoreVelocity =
            (json['ignore_velocity'] as num?)?.toInt() ?? _ignoreVelocity;
      });
    } catch (_) {
      // 回退默认值。
    } finally {
      rust.freeString(ptr);
    }
  }

  void _applyAudio({int? maxVoices, int? bufferSize, int? ignoreVelocity}) {
    setState(() {
      _maxVoices = maxVoices ?? _maxVoices;
      _bufferSize = bufferSize ?? _bufferSize;
      _ignoreVelocity = ignoreVelocity ?? _ignoreVelocity;
    });
    rust.setAudioSettings(_maxVoices, _bufferSize, _ignoreVelocity);
  }

  @override
  Widget build(BuildContext context) {
    return ColoredBox(
      color: YinheTheme.bg,
      child: Stack(
        children: [
          Row(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              SizedBox(width: 170, child: _sidebar()),
              Expanded(
                child: Padding(
                  padding: const EdgeInsets.fromLTRB(8, 16, 24, 16),
                  child: switch (_page) {
                    0 => _soundFontPage(),
                    1 => _audioPage(),
                    _ => _languagePage(),
                  },
                ),
              ),
            ],
          ),
          // 左上角返回：与工作区顶栏返回/主页按钮同 xy、同尺寸。
          Positioned(
            left: 6,
            top: 1,
            width: 40,
            height: 40,
            child: IconButton(
              tooltip: l.back,
              icon: const Icon(Icons.arrow_back),
              color: YinheTheme.text,
              visualDensity: VisualDensity.compact,
              padding: EdgeInsets.zero,
              constraints: const BoxConstraints(minWidth: 40, minHeight: 40),
              onPressed: widget.onBack,
            ),
          ),
        ],
      ),
    );
  }

  /// 左侧栏：标签页（顶部留出返回按钮的空间），每项带图标。
  Widget _sidebar() {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SizedBox(height: 52),
        _sideItem(Icons.piano, l.soundfonts, 0),
        _sideItem(Icons.volume_up, l.audio, 1),
        _sideItem(Icons.language, l.language, 2),
      ],
    );
  }

  Widget _sideItem(IconData icon, String label, int index) {
    final selected = _page == index;
    return InkWell(
      onTap: () => setState(() => _page = index),
      child: Container(
        margin: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 9),
        decoration: BoxDecoration(
          color: selected ? YinheTheme.controlBg : Colors.transparent,
          borderRadius: BorderRadius.circular(6),
        ),
        child: Row(
          children: [
            Icon(
              icon,
              size: 18,
              color: selected ? YinheTheme.accent : YinheTheme.textSecondary,
            ),
            const SizedBox(width: 8),
            Text(
              label,
              style: TextStyle(
                color: selected ? YinheTheme.accent : YinheTheme.text,
                fontSize: 14,
              ),
            ),
          ],
        ),
      ),
    );
  }

  /// 语言页：跟随系统 + 桌面端同款 15 种语言（原生名称）。
  Widget _languagePage() {
    const options = [
      ('system', '跟随系统'),
      ('zh', '简体中文'),
      ('zh-TW', '繁體中文（台灣）'),
      ('zh-HK', '繁體中文（香港）'),
      ('en', 'English'),
      ('ja', '日本語'),
      ('ko', '한국어'),
      ('de', 'Deutsch'),
      ('fr', 'Français'),
      ('es', 'Español'),
      ('it', 'Italiano'),
      ('pt', 'Português (Brasil)'),
      ('ru', 'Русский'),
      ('pl', 'Polski'),
      ('cs', 'Čeština'),
      ('tr', 'Türkçe'),
    ];
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          l.language,
          style: TextStyle(
            color: YinheTheme.text,
            fontSize: 15,
            fontWeight: FontWeight.w600,
          ),
        ),
        const SizedBox(height: 8),
        Expanded(
          child: ListView(
            children: [
              for (final (value, label) in options)
                ListTile(
                  dense: true,
                  leading: Icon(
                    value == 'system' ? Icons.settings_suggest : Icons.language,
                    size: 18,
                    color: L10n.appLanguage.value == value
                        ? YinheTheme.accent
                        : YinheTheme.textSecondary,
                  ),
                  title: Text(
                    value == 'system' ? l.followSystem : label,
                    style: TextStyle(
                      color: L10n.appLanguage.value == value
                          ? YinheTheme.accent
                          : YinheTheme.text,
                      fontSize: 14,
                    ),
                  ),
                  trailing: L10n.appLanguage.value == value
                      ? Icon(Icons.check, size: 18, color: YinheTheme.accent)
                      : null,
                  onTap: () {
                    L10n.appLanguage.value = value;
                    final p = value.toNativeUtf8();
                    try {
                      rust.setUiLanguage(p);
                    } finally {
                      calloc.free(p);
                    }
                    setState(() {});
                  },
                ),
            ],
          ),
        ),
      ],
    );
  }

  Widget _soundFontPage() {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Row(
          children: [
            Text(
              l.soundfonts,
              style: TextStyle(
                color: YinheTheme.text,
                fontSize: 15,
                fontWeight: FontWeight.w600,
              ),
            ),
            const Spacer(),
            IconButton(
              tooltip: l.addSoundfont,
              icon: const Icon(Icons.add),
              color: YinheTheme.textSecondary,
              visualDensity: VisualDensity.compact,
              onPressed: widget.onPickSoundFont,
            ),
          ],
        ),
        const SizedBox(height: 4),
        Expanded(
          child: ReorderableListView.builder(
            buildDefaultDragHandles: false,
            itemCount: _fonts.length,
            onReorderItem: _reorderFonts,
            itemBuilder: (context, i) {
              final e = _fonts[i];
              return Container(
                key: ValueKey(e.path),
                margin: const EdgeInsets.only(bottom: 6),
                padding: const EdgeInsets.symmetric(horizontal: 4),
                decoration: BoxDecoration(
                  color: YinheTheme.controlBg,
                  borderRadius: BorderRadius.circular(6),
                ),
                child: Row(
                  children: [
                    ReorderableDragStartListener(
                      index: i,
                      child: Icon(
                        Icons.drag_indicator,
                        size: 18,
                        color: YinheTheme.textDisabled,
                      ),
                    ),
                    const SizedBox(width: 6),
                    Expanded(
                      child: Text(
                        e.name.isNotEmpty ? e.name : e.path,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          color: e.enabled
                              ? YinheTheme.text
                              : YinheTheme.textDisabled,
                          fontSize: 13,
                        ),
                      ),
                    ),
                    Transform.scale(
                      scale: 0.8,
                      child: Switch(
                        value: e.enabled,
                        onChanged: (v) => _toggleFont(i, v),
                      ),
                    ),
                    IconButton(
                      tooltip: l.remove,
                      icon: const Icon(Icons.delete_outline),
                      iconSize: 18,
                      color: YinheTheme.textSecondary,
                      visualDensity: VisualDensity.compact,
                      onPressed: () => _removeFont(i),
                    ),
                  ],
                ),
              );
            },
          ),
        ),
      ],
    );
  }

  Widget _audioPage() {
    return SingleChildScrollView(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          _sectionTitle(l.maxVoices),
          const SizedBox(height: 8),
          _options(
            const [0, 256, 512, 1024, 2048, 4096],
            _maxVoices,
            (v) => v == 0 ? l.auto : '$v',
            (v) => _applyAudio(maxVoices: v),
          ),
          const SizedBox(height: 22),
          _sectionTitle(l.audioBuffer),
          const SizedBox(height: 8),
          _options(
            const [0, 512, 1024, 2048, 4096],
            _bufferSize,
            (v) =>
                v == 0 ? l.deviceDefault : (L10n.isZh ? '$v 帧' : '$v frames'),
            (v) => _applyAudio(bufferSize: v),
          ),
          const SizedBox(height: 22),
          _sectionTitle(l.ignoreVelocity),
          const SizedBox(height: 8),
          _options(
            const [0, 1, 2, 4, 8, 16],
            _ignoreVelocity,
            (v) => '$v',
            (v) => _applyAudio(ignoreVelocity: v),
          ),
        ],
      ),
    );
  }

  Widget _sectionTitle(String text) {
    return Text(
      text,
      style: TextStyle(
        color: YinheTheme.text,
        fontSize: 14,
        fontWeight: FontWeight.w600,
      ),
    );
  }

  Widget _options(
    List<int> options,
    int value,
    String Function(int) label,
    ValueChanged<int> onTap,
  ) {
    return Wrap(
      spacing: 8,
      runSpacing: 8,
      children: [
        for (final o in options)
          InkWell(
            borderRadius: BorderRadius.circular(6),
            onTap: () => onTap(o),
            child: Container(
              padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 8),
              decoration: BoxDecoration(
                color: o == value ? YinheTheme.accent : YinheTheme.controlBg,
                borderRadius: BorderRadius.circular(6),
              ),
              child: Text(
                label(o),
                style: TextStyle(
                  color: o == value ? Colors.white : YinheTheme.text,
                  fontSize: 13,
                ),
              ),
            ),
          ),
      ],
    );
  }
}
