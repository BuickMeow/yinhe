import 'dart:convert';
import 'dart:ffi' as ffi;

import 'package:ffi/ffi.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../core/ffi.dart' as rust;
import '../l10n.dart';
import '../l10n/app_localizations.dart';
import '../core/theme.dart';

/// 设置页：Scaffold + AppBar，宽屏 NavigationRail、窄屏 NavigationBar 切页。
///
/// 音色库为多库表：长按整行拖动排序、点整行启用/关闭、左滑删除。
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
  _SfItem({
    required this.path,
    required this.name,
    required this.enabled,
    this.warnings = const [],
  });

  final String path;
  final String name;
  bool enabled;

  /// xsynth 不支持的 opcode 提示（空 = 没有或不是 SFZ）。
  final List<String> warnings;
}

class _SettingsViewState extends State<SettingsView> {
  AppLocalizations get l => AppLocalizations.of(context);

  int _page = 0;
  List<_SfItem> _fonts = const [];

  /// 音频设置（与 Rust 同步；默认值同桌面/引擎回退）。
  int _maxVoices = 512;
  int _bufferSize = 2048;
  int _ignoreVelocity = 1;
  bool _allowOverlappingNotes = true;
  bool _followPlayhead = true;
  bool _pauseOnHome = false;
  bool _pauseOnExit = false;

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
                warnings: [
                  for (final w
                      in (it['warnings'] as List<dynamic>? ?? const []))
                    if (w is String) w,
                ],
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

  /// 展示该 SFZ 里会被忽略的 opcode。
  void _showWarnings(_SfItem item) {
    showDialog<void>(
      context: context,
      builder: (context) => AlertDialog(
        backgroundColor: YinheTheme.bg,
        title: Text(
          L10n.isZh ? '兼容性提示' : 'Compatibility',
          style: TextStyle(color: YinheTheme.text, fontSize: 16),
        ),
        content: SizedBox(
          width: 420,
          child: SingleChildScrollView(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  item.name.isNotEmpty ? item.name : item.path,
                  style: TextStyle(
                    color: YinheTheme.textSecondary,
                    fontSize: 13,
                  ),
                ),
                const SizedBox(height: 10),
                for (final w in item.warnings)
                  Padding(
                    padding: const EdgeInsets.only(bottom: 6),
                    child: Text(
                      '• $w',
                      style: TextStyle(color: YinheTheme.text, fontSize: 13),
                    ),
                  ),
                const SizedBox(height: 4),
                Text(
                  L10n.isZh ? '这些 opcode 当前会被忽略，音色可能与原库不一致。' : 'These opcodes are currently ignored; the sound may differ.',
                  style: TextStyle(color: YinheTheme.textMuted, fontSize: 12),
                ),
              ],
            ),
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(),
            child: Text(L10n.isZh ? '知道了' : 'OK'),
          ),
        ],
      ),
    );
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
        _allowOverlappingNotes =
            json['allow_overlapping_notes'] as bool? ?? _allowOverlappingNotes;
        _followPlayhead = json['follow_playhead'] as bool? ?? _followPlayhead;
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

  void _setAllowOverlappingNotes(bool value) {
    setState(() => _allowOverlappingNotes = value);
    rust.setAllowOverlappingNotes(value ? 1 : 0);
  }

  void _setFollowPlayhead(bool value) {
    setState(() => _followPlayhead = value);
    rust.setFollowPlayhead(value ? 1 : 0);
  }

  void _setPauseOnHome(bool value) {
    setState(() => _pauseOnHome = value);
    rust.setPauseOnHome(value ? 1 : 0);
  }

  void _setPauseOnExit(bool value) {
    setState(() => _pauseOnExit = value);
    rust.setPauseOnExit(value ? 1 : 0);
  }

  @override
  Widget build(BuildContext context) {
    final wide = MediaQuery.sizeOf(context).width >= 600;
    final ios = Theme.of(context).platform == TargetPlatform.iOS;
    final page = switch (_page) {
      0 => _soundFontPage(),
      1 => _audioPage(),
      2 => _editingPage(),
      _ => _languagePage(),
    };
    final tabs = <(IconData, String)>[
      (Icons.piano, l.soundfonts),
      (Icons.volume_up, l.audio),
      (Icons.edit, l.edit),
      (Icons.language, l.language),
    ];
    return MediaQuery.removePadding(
      context: context,
      // 外层 StudioShell 已经为状态栏/导航栏让位；这里只保留 iOS 的
      // home indicator 安全区，避免 NavigationBar 被小白条压住。
      removeTop: true,
      removeBottom: !ios,
      child: Scaffold(
        backgroundColor: YinheTheme.bg,
        appBar: AppBar(
          backgroundColor: YinheTheme.controlBg,
          foregroundColor: YinheTheme.text,
          surfaceTintColor: Colors.transparent,
          elevation: 0,
          scrolledUnderElevation: 0,
          leading: IconButton(
            tooltip: l.back,
            icon: const Icon(Icons.arrow_back),
            onPressed: widget.onBack,
          ),
          title: Text(
            l.settings,
            style: TextStyle(
              color: YinheTheme.text,
              fontSize: 17,
              fontWeight: FontWeight.w600,
            ),
          ),
        ),
        body: wide
            ? Row(
                children: [
                  NavigationRail(
                    selectedIndex: _page,
                    onDestinationSelected: (i) => setState(() => _page = i),
                    labelType: NavigationRailLabelType.all,
                    backgroundColor: YinheTheme.bg,
                    groupAlignment: -0.85,
                    destinations: [
                      for (final (icon, label) in tabs)
                        NavigationRailDestination(
                          icon: Icon(icon),
                          label: Text(label),
                        ),
                    ],
                  ),
                  VerticalDivider(width: 1, color: YinheTheme.lineFg),
                  Expanded(
                    child: Padding(
                      padding: const EdgeInsets.fromLTRB(12, 12, 24, 16),
                      child: page,
                    ),
                  ),
                ],
              )
            : Column(
                children: [
                  Expanded(
                    child: Padding(
                      padding: const EdgeInsets.fromLTRB(12, 8, 12, 12),
                      child: page,
                    ),
                  ),
                  NavigationBar(
                    selectedIndex: _page,
                    onDestinationSelected: (i) => setState(() => _page = i),
                    backgroundColor: YinheTheme.controlBg,
                    indicatorColor: YinheTheme.selectedBg,
                    destinations: [
                      for (final (icon, label) in tabs)
                        NavigationDestination(icon: Icon(icon), label: label),
                    ],
                  ),
                ],
              ),
      ),
    );
  }

  /// 设置行统一外壳：Material 卡片 + adaptive 开关。
  ///
  /// iOS/iPadOS 自动使用 CupertinoSwitch，Android 使用 Material Switch；
  /// 两个设置页共用这一份样式，避免出现两种开关。
  Widget _switchCard({
    required String title,
    required bool value,
    required ValueChanged<bool> onChanged,
    String? subtitle,
    IconData? icon,
  }) {
    return Card(
      margin: const EdgeInsets.only(bottom: 8),
      elevation: 0,
      color: YinheTheme.controlBg,
      clipBehavior: Clip.antiAlias,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(6)),
      child: SwitchListTile.adaptive(
        contentPadding: const EdgeInsets.symmetric(horizontal: 12),
        secondary: icon == null
            ? null
            : Icon(icon, size: 20, color: YinheTheme.textSecondary),
        title: Text(
          title,
          style: TextStyle(color: YinheTheme.text, fontSize: 14),
        ),
        subtitle: subtitle == null
            ? null
            : Text(
                subtitle,
                style: TextStyle(color: YinheTheme.textSecondary, fontSize: 12),
              ),
        value: value,
        onChanged: onChanged,
      ),
    );
  }

  /// 编辑页：开关行与音色库页共用同一套 adaptive 样式。
  Widget _editingPage() {
    return ListView(
      padding: EdgeInsets.zero,
      children: [
        Text(
          l.edit,
          style: TextStyle(
            color: YinheTheme.text,
            fontSize: 15,
            fontWeight: FontWeight.w600,
          ),
        ),
        const SizedBox(height: 12),
        _switchCard(
          title: l.allowOverlappingNotes,
          subtitle: L10n.isZh
              ? '关闭后，新建或移动的音符不能与已有音符重叠'
              : 'When off, new or moved notes cannot overlap existing notes',
          value: _allowOverlappingNotes,
          onChanged: _setAllowOverlappingNotes,
        ),
        _switchCard(
          title: L10n.isZh ? '跟随播放头' : 'Follow playhead',
          subtitle: L10n.isZh ? '播放时卷帘居中跟随；手动拖动后，播放头离开视口 2 秒会恢复' : 'Center the roll on the playhead; resumes 2 s after it leaves the view',
          icon: Icons.center_focus_strong,
          value: _followPlayhead,
          onChanged: _setFollowPlayhead,
        ),
        _switchCard(
          title: L10n.isZh ? '打开侧栏时暂停' : 'Pause on menu',
          subtitle: L10n.isZh
              ? '打开侧栏时停止播放（默认关闭）'
              : 'Stop playback when opening the menu',
          icon: Icons.home_outlined,
          value: _pauseOnHome,
          onChanged: _setPauseOnHome,
        ),
        _switchCard(
          title: L10n.isZh ? '退出应用时暂停' : 'Pause on exit',
          subtitle: L10n.isZh
              ? '切到后台或退出时停止播放（默认关闭）'
              : 'Stop playback when leaving the app',
          icon: Icons.exit_to_app,
          value: _pauseOnExit,
          onChanged: _setPauseOnExit,
        ),
      ],
    );
  }

  /// 语言页：跟随系统 + 桌面端同款 15 种语言（原生名称）。
  Widget _languagePage() {
    final options = [
      ('system', l.followSystem),
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
            // 手机/平板默认长按整行拖动；桌面端才显示 drag handle。
            buildDefaultDragHandles: true,
            itemCount: _fonts.length,
            onReorderItem: _reorderFonts,
            itemBuilder: (context, i) {
              final e = _fonts[i];
              final name = e.name.isNotEmpty ? e.name : e.path;
              return Dismissible(
                key: ValueKey(e.path),
                direction: DismissDirection.endToStart,
                onDismissed: (_) => _removeFont(i),
                background: Container(
                  margin: const EdgeInsets.only(bottom: 8),
                  padding: const EdgeInsets.symmetric(horizontal: 18),
                  alignment: Alignment.centerRight,
                  decoration: BoxDecoration(
                    color: YinheTheme.danger,
                    borderRadius: BorderRadius.circular(6),
                  ),
                  child: const Icon(Icons.delete_outline, color: Colors.white),
                ),
                child: Card(
                  margin: const EdgeInsets.only(bottom: 8),
                  elevation: 0,
                  color: YinheTheme.controlBg,
                  clipBehavior: Clip.antiAlias,
                  shape: RoundedRectangleBorder(
                    borderRadius: BorderRadius.circular(6),
                  ),
                  child: SwitchListTile.adaptive(
                    contentPadding: const EdgeInsets.only(left: 16, right: 8),
                    // 固定占位：有/无警告时标题都从同一列开始。
                    secondary: SizedBox(
                      width: 32,
                      child: e.warnings.isEmpty
                          ? null
                          : IconButton(
                              tooltip: L10n.isZh ? '兼容性提示' : 'Compatibility',
                              icon: const Icon(Icons.warning_amber_rounded),
                              iconSize: 18,
                              padding: EdgeInsets.zero,
                              constraints: const BoxConstraints.tightFor(
                                width: 32,
                                height: 32,
                              ),
                              color: YinheTheme.warning,
                              onPressed: () => _showWarnings(e),
                            ),
                    ),
                    title: Text(
                      name,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        color: e.enabled
                            ? YinheTheme.text
                            : YinheTheme.textDisabled,
                        fontSize: 14,
                      ),
                    ),
                    subtitle: e.name.isNotEmpty && e.name != e.path
                        ? Text(
                            e.path,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: TextStyle(
                              color: YinheTheme.textDisabled,
                              fontSize: 11,
                            ),
                          )
                        : null,
                    value: e.enabled,
                    onChanged: (v) => _toggleFont(i, v),
                  ),
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
          _sectionTitle(
            l.maxVoices,
            hint: L10n.isZh
                ? '越高，音色越完整；也越吃性能，可能更卡顿'
                : 'Higher = fuller sound, but heavier and may stutter',
          ),
          const SizedBox(height: 8),
          _options(
            const [0, 256, 512, 1024, 2048, 4096],
            _maxVoices,
            (v) => v == 0 ? l.auto : '$v',
            (v) => _applyAudio(maxVoices: v),
          ),
          const SizedBox(height: 22),
          _sectionTitle(
            l.audioBuffer,
            hint: L10n.isZh
                ? '越小延迟越低，但更容易爆音；越大越稳'
                : 'Smaller = lower latency, but more likely to crackle',
          ),
          const SizedBox(height: 8),
          _options(
            const [0, 512, 1024, 2048, 4096],
            _bufferSize,
            (v) => v == 0
                ? l.deviceDefault
                : (L10n.isZh ? '$v ${l.frames}' : '$v frames'),
            (v) => _applyAudio(bufferSize: v),
          ),
          const SizedBox(height: 22),
          _sectionTitle(
            l.ignoreVelocity,
            hint: L10n.isZh
                ? '力度不超过该值的音符不发声（黑乐谱音符画）'
                : 'Notes at or below this velocity are silent',
          ),
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

  Widget _sectionTitle(String text, {String? hint}) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(
          text,
          style: TextStyle(
            color: YinheTheme.text,
            fontSize: 14,
            fontWeight: FontWeight.w600,
          ),
        ),
        if (hint != null)
          Padding(
            padding: const EdgeInsets.only(top: 3),
            child: Text(
              hint,
              style: TextStyle(color: YinheTheme.textSecondary, fontSize: 11),
            ),
          ),
      ],
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
          ChoiceChip(
            label: Text(label(o)),
            selected: o == value,
            showCheckmark: false,
            onSelected: (_) => onTap(o),
          ),
      ],
    );
  }
}
