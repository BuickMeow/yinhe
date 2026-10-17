import 'package:flutter/material.dart';

import 'theme.dart';

/// 设置页（全屏浮层）：分页显示，目前只有「音色库」一页。
///
/// 设置项与桌面端不同（移动端需要的不一样），暂不跟进桌面设置。
class SettingsView extends StatelessWidget {
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
  Widget build(BuildContext context) {
    return ColoredBox(
      color: YinheTheme.bg,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
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
                  onPressed: onBack,
                ),
                const SizedBox(width: 4),
                Text(
                  '设置',
                  style: TextStyle(color: YinheTheme.text, fontSize: 15),
                ),
              ],
            ),
          ),
          Expanded(
            child: Padding(
              padding: const EdgeInsets.all(24),
              child: Column(
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
                    padding: const EdgeInsets.symmetric(
                      horizontal: 12,
                      vertical: 10,
                    ),
                    decoration: BoxDecoration(
                      color: YinheTheme.controlBg,
                      borderRadius: BorderRadius.circular(6),
                    ),
                    child: Row(
                      children: [
                        Icon(
                          Icons.piano,
                          size: 18,
                          color: YinheTheme.textLabel,
                        ),
                        const SizedBox(width: 8),
                        Expanded(
                          child: Text(
                            soundFontName ?? '未选择音色库（播放无声）',
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: TextStyle(
                              color: soundFontName == null
                                  ? YinheTheme.textDisabled
                                  : YinheTheme.text,
                              fontSize: 13,
                            ),
                          ),
                        ),
                        TextButton(
                          onPressed: onPickSoundFont,
                          child: const Text('选择…'),
                        ),
                      ],
                    ),
                  ),
                  const SizedBox(height: 16),
                  Text(
                    '音色库选择后会持久保存，下次启动自动恢复。',
                    style: TextStyle(
                      color: YinheTheme.textDisabled,
                      fontSize: 11,
                    ),
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}
