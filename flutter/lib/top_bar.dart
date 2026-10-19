import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import 'models.dart';
import 'theme.dart';
import 'tools.dart';

/// 顶部栏：左侧视图入口 + 走带，右侧量化/工具/撤销重做；
/// 中央在 PR 显示"显示轨道"长按钮，在 AR 显示歌曲标题（只读）。
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
    required this.onSelectDisplayTrack,
    required this.onProjectSettings,
    required this.onSave,
    required this.saveDone,
    required this.noteCount,
    required this.bpmText,
    required this.sigText,
    required this.onEditBpm,
    required this.onEditSig,
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
  final ValueChanged<int> onSelectDisplayTrack;
  final VoidCallback onProjectSettings;

  /// 保存工程（无改动时按钮置灰不可点）。
  final VoidCallback onSave;

  /// 刚保存成功：2 秒内把保存按钮显示为对勾。
  final bool saveDone;

  /// 全曲音符数（AR 歌名弹窗内显示）。
  final int noteCount;

  /// 本工程默认 BPM / 拍号文本（点击可改；原生事件优先于它们）。
  final String bpmText;
  final String sigText;
  final VoidCallback onEditBpm;
  final VoidCallback onEditSig;

  @override
  Widget build(BuildContext context) {
    return Container(
      height: 42,
      padding: const EdgeInsets.symmetric(horizontal: 6),
      decoration: BoxDecoration(
        // 主题 token（与桌面 app_bg/line_fg 同色系）。
        color: YinheTheme.controlBg,
        border: Border(bottom: BorderSide(color: YinheTheme.lineFg)),
      ),
      child: Stack(
        children: [
          Row(
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
                  padding: const EdgeInsets.symmetric(
                    horizontal: 8,
                    vertical: 6,
                  ),
                  child: ValueListenableBuilder<String>(
                    valueListenable: timecodeText,
                    builder: (context, text, _) => Text(
                      text,
                      style: TextStyle(
                        color: YinheTheme.textLabel,
                        fontSize: 14,
                      ),
                    ),
                  ),
                ),
              ),
              _metaText(bpmText, '本工程默认 BPM', onEditBpm),
              _metaText(sigText, '本工程默认拍号', onEditSig),
              const Spacer(),
              _buildQuantizeButton(context),
              _buildToolButton(),
              ValueListenableBuilder<int>(
                valueListenable: docState,
                builder: (context, st, _) => Row(
                  children: [
                    _iconAction(
                      Icons.undo,
                      '撤销',
                      (st & 1) != 0 ? onUndo : null,
                    ),
                    _iconAction(
                      Icons.redo,
                      '重做',
                      (st & 2) != 0 ? onRedo : null,
                    ),
                    // 保存：从右向左第一个；无改动时置灰不可点；
                    // 保存成功后 2 秒内显示对勾。
                    if (saveDone)
                      _iconAction(
                        Icons.check,
                        '已保存',
                        null,
                        color: YinheTheme.accent,
                      )
                    else
                      _iconAction(
                        Icons.save_outlined,
                        '保存工程',
                        (st & 4) != 0 ? onSave : null,
                      ),
                  ],
                ),
              ),
            ],
          ),
          // 中央：PR = 显示轨道长按钮；AR = 歌名按钮（直接打开工程设置）。
          Positioned.fill(
            child: Align(
              alignment: Alignment.center,
              child: isPr ? _buildSongBox() : _titleButton(),
            ),
          ),
        ],
      ),
    );
  }

  /// 顶栏只读信息（与时间码同色）：BPM / 拍号，点击编辑工程默认值。
  Widget _metaText(String text, String tooltip, VoidCallback onTap) {
    return InkWell(
      borderRadius: BorderRadius.circular(4),
      onTap: onTap,
      child: Tooltip(
        message: tooltip,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 6),
          child: Text(
            text,
            style: TextStyle(color: YinheTheme.textLabel, fontSize: 14),
          ),
        ),
      ),
    );
  }

  /// 歌曲标题（音符数在 AR 歌名弹窗内部）。
  String _titled() {
    return songTitle.isNotEmpty ? songTitle : (fileName ?? '未命名');
  }

  Widget _iconAction(
    IconData icon,
    String tooltip,
    VoidCallback? onTap, {
    Color? color,
  }) {
    // 颜色显式给到 Icon：IconButton 在禁用态会用主题 disabledColor 覆盖 color。
    final c =
        color ??
        (onTap == null ? YinheTheme.textDisabled : YinheTheme.textSecondary);
    return IconButton(
      onPressed: onTap,
      icon: Icon(icon, color: c),
      color: c,
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

  /// AR：歌名按钮（点击直接打开工程设置大窗口；音符数在窗口内部）。
  Widget _titleButton() {
    return InkWell(
      borderRadius: BorderRadius.circular(6),
      onTap: onProjectSettings,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 260),
          child: ValueListenableBuilder<int>(
            valueListenable: docState,
            builder: (context, st, _) => Text(
              (st & 4) != 0 ? '• ${_titled()}' : _titled(),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(color: YinheTheme.textLabel, fontSize: 13),
            ),
          ),
        ),
      ),
    );
  }

  /// PR：显示轨道长按钮（顶部正中央；保存工程只在主页）。
  Widget _buildSongBox() {
    final filtering = displayTrack >= 0 && displayTrack < tracks.length;
    final base = filtering ? tracks[displayTrack].name : _titled();
    return PopupMenuButton<int>(
      tooltip: '显示轨道',
      padding: EdgeInsets.zero,
      onSelected: onSelectDisplayTrack,
      itemBuilder: (_) => [
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
        constraints: const BoxConstraints(maxWidth: 220),
        alignment: Alignment.center,
        decoration: BoxDecoration(
          // 比顶栏底（controlBg）稍深一档，长按钮才可见。
          color: YinheTheme.btnBg,
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
