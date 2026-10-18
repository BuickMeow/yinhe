import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import 'models.dart';
import 'theme.dart';
import 'tools.dart';

/// 顶部栏：左侧视图入口 + 走带，右侧量化/工具/撤销重做/曲名框。
class TopBar extends StatelessWidget {
  const TopBar({
    super.key,
    required this.isPr,
    required this.playing,
    required this.timecodeText,
    required this.quantizeLabel,
    required this.ppq,
    required this.tool,
    required this.tracks,
    required this.displayTrack,
    required this.songTitle,
    required this.fileName,
    required this.docState,
    required this.onHome,
    required this.onBack,
    required this.onTogglePlay,
    required this.onStop,
    required this.onToggleTimecode,
    required this.onQuantize,
    required this.onTool,
    required this.onUndo,
    required this.onRedo,
    required this.onSaveAs,
    required this.onSelectDisplayTrack,
  });

  final bool isPr;
  final bool playing;
  final ValueListenable<String> timecodeText;
  final String quantizeLabel;
  final double ppq;
  final int tool;
  final List<TrackRow> tracks;
  final int displayTrack;
  final String songTitle;
  final String? fileName;
  final ValueListenable<int> docState;
  final VoidCallback onHome;
  final VoidCallback onBack;
  final VoidCallback onTogglePlay;
  final VoidCallback onStop;
  final VoidCallback onToggleTimecode;
  final ValueChanged<String> onQuantize;
  final ValueChanged<int> onTool;
  final VoidCallback onUndo;
  final VoidCallback onRedo;
  final VoidCallback onSaveAs;
  final ValueChanged<int> onSelectDisplayTrack;

  @override
  Widget build(BuildContext context) {
    return Container(
      height: 42,
      color: Color.lerp(YinheTheme.bg, Colors.black, 0.04),
      padding: const EdgeInsets.symmetric(horizontal: 6),
      child: Row(
        children: [
          IconButton(
            onPressed: isPr ? onBack : onHome,
            icon: Icon(isPr ? Icons.arrow_back : Icons.home),
            color: YinheTheme.text,
            tooltip: isPr ? '返回走带' : '主页',
            visualDensity: VisualDensity.compact,
            padding: EdgeInsets.zero,
            constraints: const BoxConstraints(minWidth: 40, minHeight: 40),
          ),
          IconButton(
            onPressed: onTogglePlay,
            icon: Icon(playing ? Icons.pause : Icons.play_arrow),
            color: YinheTheme.text,
            tooltip: playing ? '暂停' : '播放',
            visualDensity: VisualDensity.compact,
            padding: EdgeInsets.zero,
            constraints: const BoxConstraints(minWidth: 40, minHeight: 40),
          ),
          IconButton(
            onPressed: onStop,
            icon: const Icon(Icons.stop),
            color: YinheTheme.textSecondary,
            tooltip: '停止',
            visualDensity: VisualDensity.compact,
            padding: EdgeInsets.zero,
            constraints: const BoxConstraints(minWidth: 40, minHeight: 40),
          ),
          const SizedBox(width: 4),
          InkWell(
            borderRadius: BorderRadius.circular(4),
            onTap: onToggleTimecode,
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
              child: ValueListenableBuilder<String>(
                valueListenable: timecodeText,
                builder: (context, text, _) => Text(
                  text,
                  style: TextStyle(color: YinheTheme.textLabel, fontSize: 14),
                ),
              ),
            ),
          ),
          const Spacer(),
          _buildQuantizeButton(context),
          _buildToolButton(),
          ValueListenableBuilder<int>(
            valueListenable: docState,
            builder: (context, st, _) => Row(
              children: [
                _iconAction(Icons.undo, '撤销', (st & 1) != 0 ? onUndo : null),
                _iconAction(Icons.redo, '重做', (st & 2) != 0 ? onRedo : null),
              ],
            ),
          ),
          _buildSongBox(),
        ],
      ),
    );
  }

  Widget _iconAction(IconData icon, String tooltip, VoidCallback? onTap) {
    return IconButton(
      onPressed: onTap,
      icon: Icon(icon),
      color: onTap == null ? YinheTheme.textDisabled : YinheTheme.textSecondary,
      tooltip: tooltip,
      visualDensity: VisualDensity.compact,
      padding: EdgeInsets.zero,
      constraints: const BoxConstraints(minWidth: 40, minHeight: 40),
    );
  }

  Widget _buildQuantizeButton(BuildContext context) {
    return PopupMenuButton<String>(
      tooltip: '量化（吸附档位）',
      padding: EdgeInsets.zero,
      onSelected: onQuantize,
      itemBuilder: (_) => [
        const PopupMenuItem(value: '关闭', child: Text('关闭吸附')),
        const PopupMenuItem(value: '自动', child: Text('自动（随缩放）')),
        for (final (label, ticks) in _quantizePresets(ppq))
          PopupMenuItem(value: label, child: Text('$label（$ticks tick）')),
      ],
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              Icons.motion_photos_on,
              size: 16,
              color: YinheTheme.textSecondary,
            ),
            const SizedBox(width: 4),
            Text(
              quantizeLabel,
              style: TextStyle(color: YinheTheme.textLabel, fontSize: 13),
            ),
          ],
        ),
      ),
    );
  }

  /// 常用量化档位（与桌面 `presets_for_ppq` 同规则）。
  static List<(String, int)> _quantizePresets(double ppq) {
    final whole = ppq.round().clamp(1, 1 << 24) * 4;
    final out = <(String, int)>[];
    void chain(int start) {
      var den = start;
      while (den <= whole && whole % den == 0) {
        out.add(('1/$den', whole ~/ den));
        den *= 2;
      }
    }

    chain(1);
    chain(6);
    return out;
  }

  Widget _buildToolButton() {
    final list = isPr ? toolPrAllowed : toolArAllowed;
    final shown = list.contains(tool) ? tool : list.first;
    return PopupMenuButton<int>(
      tooltip: '工具',
      padding: EdgeInsets.zero,
      onSelected: onTool,
      itemBuilder: (_) => [
        for (final i in list)
          PopupMenuItem(
            value: i,
            child: Row(
              children: [
                Icon(
                  toolDefs[i].$1,
                  size: 18,
                  color: i == shown
                      ? YinheTheme.accent
                      : YinheTheme.textSecondary,
                ),
                const SizedBox(width: 8),
                Text(toolDefs[i].$2),
              ],
            ),
          ),
      ],
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
        child: Icon(
          toolDefs[shown].$1,
          size: 18,
          color: YinheTheme.textSecondary,
        ),
      ),
    );
  }

  Widget _buildSongBox() {
    final filtering = displayTrack >= 0 && displayTrack < tracks.length;
    final base = filtering
        ? tracks[displayTrack].name
        : (songTitle.isNotEmpty ? songTitle : (fileName ?? '未命名'));
    return PopupMenuButton<int>(
      tooltip: '显示轨道 / 保存工程',
      padding: EdgeInsets.zero,
      onSelected: (v) {
        if (v == -2) {
          onSaveAs();
        } else {
          onSelectDisplayTrack(v);
        }
      },
      itemBuilder: (_) => [
        const PopupMenuItem(value: -2, child: Text('保存为工程（.ying）…')),
        const PopupMenuDivider(),
        const PopupMenuItem(value: -1, child: Text('全部轨道')),
        for (var i = 0; i < tracks.length; i++)
          PopupMenuItem(
            value: i,
            child: Text('${i.toString().padLeft(3, '0')}  ${tracks[i].name}'),
          ),
      ],
      child: Container(
        margin: const EdgeInsets.symmetric(vertical: 7),
        padding: const EdgeInsets.symmetric(horizontal: 10),
        constraints: const BoxConstraints(maxWidth: 190),
        alignment: Alignment.center,
        decoration: BoxDecoration(
          color: YinheTheme.controlBg,
          borderRadius: BorderRadius.circular(6),
        ),
        child: ValueListenableBuilder<int>(
          valueListenable: docState,
          builder: (context, st, _) => Text(
            (st & 4) != 0 ? '• $base' : base,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              color: filtering ? YinheTheme.accent : YinheTheme.text,
              fontSize: 12,
            ),
          ),
        ),
      ),
    );
  }
}
