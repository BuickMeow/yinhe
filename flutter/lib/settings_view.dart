import 'dart:convert';
import 'dart:ffi' as ffi;

import 'package:ffi/ffi.dart';
import 'package:flutter/material.dart';

import 'ffi.dart' as rust;
import 'theme.dart';

/// 设置页（全屏浮层）：分页显示（音色库 / 音频）。
class SettingsView extends StatefulWidget {
  const SettingsView({
    super.key,
    required this.soundFontName,
    required this.onPickSoundFont,
    required this.onBack,
  });

  final String? soundFontName;
  final VoidCallback onPickSoundFont;
  final VoidCallback onBack;

  @override
  State<SettingsView> createState() => _SettingsViewState();
}

class _SettingsViewState extends State<SettingsView> {
  int _page = 0;

  /// 音频设置（与 Rust 同步；默认值同桌面/引擎回退）。
  int _maxVoices = 1024;
  int _bufferSize = 2048;
  int _ignoreVelocity = 1;

  @override
  void initState() {
    super.initState();
    _loadAudioSettings();
  }

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
    final changedBuffer = bufferSize != null;
    rust.setAudioSettings(_maxVoices, _bufferSize, _ignoreVelocity);
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(
        content: Text(changedBuffer ? '音频设置已保存（缓冲下次启动生效）' : '音频设置已保存'),
        duration: const Duration(milliseconds: 1200),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return ColoredBox(
      color: YinheTheme.bg,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          // 页头
          Container(
            height: 42,
            color: Color.lerp(YinheTheme.bg, Colors.black, 0.04),
            padding: const EdgeInsets.symmetric(horizontal: 4),
            child: Row(
              children: [
                IconButton(
                  tooltip: '返回',
                  icon: const Icon(Icons.arrow_back),
                  color: YinheTheme.textSecondary,
                  visualDensity: VisualDensity.compact,
                  padding: EdgeInsets.zero,
                  onPressed: widget.onBack,
                ),
                const SizedBox(width: 4),
                Text(
                  '设置',
                  style: TextStyle(color: YinheTheme.text, fontSize: 15),
                ),
              ],
            ),
          ),
          // 分页标签
          Padding(
            padding: const EdgeInsets.fromLTRB(24, 14, 24, 0),
            child: Row(
              children: [
                _tab('音色库', 0),
                const SizedBox(width: 8),
                _tab('音频', 1),
              ],
            ),
          ),
          Expanded(
            child: Padding(
              padding: const EdgeInsets.all(24),
              child: _page == 0 ? _soundFontPage() : _audioPage(),
            ),
          ),
        ],
      ),
    );
  }

  Widget _tab(String label, int index) {
    final selected = _page == index;
    return InkWell(
      borderRadius: BorderRadius.circular(6),
      onTap: () => setState(() => _page = index),
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 7),
        decoration: BoxDecoration(
          color: selected ? YinheTheme.accent : YinheTheme.controlBg,
          borderRadius: BorderRadius.circular(6),
        ),
        child: Text(
          label,
          style: TextStyle(
            color: selected ? Colors.white : YinheTheme.text,
            fontSize: 13,
          ),
        ),
      ),
    );
  }

  Widget _soundFontPage() {
    final name = widget.soundFontName;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(
          '音色库',
          style: TextStyle(
            color: YinheTheme.text,
            fontSize: 15,
            fontWeight: FontWeight.w600,
          ),
        ),
        const SizedBox(height: 12),
        Text(
          '已加载',
          style: TextStyle(color: YinheTheme.textLabel, fontSize: 12),
        ),
        const SizedBox(height: 6),
        Container(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
          decoration: BoxDecoration(
            color: YinheTheme.controlBg,
            borderRadius: BorderRadius.circular(6),
          ),
          child: Row(
            children: [
              Icon(Icons.piano, size: 18, color: YinheTheme.textLabel),
              const SizedBox(width: 8),
              Expanded(
                child: Text(
                  name ?? '未选择音色库（播放无声）',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    color: name == null
                        ? YinheTheme.textDisabled
                        : YinheTheme.text,
                    fontSize: 13,
                  ),
                ),
              ),
              TextButton(
                onPressed: widget.onPickSoundFont,
                child: const Text('选择…'),
              ),
            ],
          ),
        ),
        const SizedBox(height: 16),
        Text(
          '音色库选择后会持久保存，下次启动自动恢复。',
          style: TextStyle(color: YinheTheme.textDisabled, fontSize: 11),
        ),
      ],
    );
  }

  Widget _audioPage() {
    return SingleChildScrollView(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          _sectionTitle('复音上限'),
          _hint('超限时优先淘汰较旧的音符。0 = 自动（桌面默认不限制）。'),
          const SizedBox(height: 8),
          _options(
            const [0, 256, 512, 1024, 2048, 4096],
            _maxVoices,
            (v) => v == 0 ? '自动' : '$v',
            (v) => _applyAudio(maxVoices: v),
          ),
          const SizedBox(height: 22),
          _sectionTitle('音频缓冲'),
          _hint('数值越大越抗卡顿，但延迟更高（帧 @48kHz）。0 = 设备默认。修改后下次启动生效。'),
          const SizedBox(height: 8),
          _options(
            const [0, 512, 1024, 2048, 4096],
            _bufferSize,
            (v) => v == 0 ? '设备默认' : '$v 帧',
            (v) => _applyAudio(bufferSize: v),
          ),
          const SizedBox(height: 22),
          _sectionTitle('忽略力度'),
          _hint('力度不超过该值的音符不发声（桌面默认 1）。'),
          const SizedBox(height: 8),
          _options(
            const [0, 1, 2, 4, 8, 16],
            _ignoreVelocity,
            (v) => '$v',
            (v) => _applyAudio(ignoreVelocity: v),
          ),
          const SizedBox(height: 24),
          Text(
            '以上设置会持久保存（settings.txt），下次启动自动恢复。',
            style: TextStyle(color: YinheTheme.textDisabled, fontSize: 11),
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

  Widget _hint(String text) {
    return Padding(
      padding: const EdgeInsets.only(top: 4),
      child: Text(
        text,
        style: TextStyle(color: YinheTheme.textDisabled, fontSize: 11),
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
