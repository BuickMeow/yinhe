import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../core/edit_status.dart';
import '../core/models.dart';
import '../core/quantize_presets.dart';
import '../core/theme.dart';
import '../l10n/app_localizations.dart';
import '../core/tools.dart';

/// 顶部栏：左侧视图入口 + 走带，右侧量化/工具/撤销重做；
/// 中央在 PR 显示"显示轨道"长按钮，在 AR 显示歌曲标题（只读）。
class TopBar extends StatelessWidget {
  const TopBar({
    super.key,
    required this.isPr,
    required this.playing,
    required this.timecodeText,
    required this.editStatus,
    required this.quantizeLabel,
    required this.ppq,
    required this.mainTool,
    required this.quickTool,
    required this.quickSlotActive,
    required this.tracks,
    required this.activeTrack,
    required this.showAllTracks,
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
    required this.onQuickTool,
    required this.onUndo,
    required this.onRedo,
    required this.onSelectTrack,
    required this.onToggleAllTracks,
    required this.onProjectSettings,
    required this.onSave,
    required this.saveDone,
    required this.saving,
    required this.noteCount,
    required this.bpmText,
    required this.sigText,
    required this.onEditBpm,
    required this.onEditSig,
  });

  final bool isPr;
  final bool playing;
  final ValueListenable<String> timecodeText;

  /// 拖动时的临时读数（力度/自动化/选框）；null = 常规显示。
  final ValueListenable<EditStatus?> editStatus;
  final String quantizeLabel;
  final double ppq;

  /// 主工具按钮自己记住的工具。
  final int mainTool;

  /// 快捷工具组自己记住的工具；不随主工具 popup 改变。
  final int quickTool;

  /// 最近一次操作的是否为快捷槽；决定哪个按钮高亮。
  final bool quickSlotActive;
  final List<TrackRow> tracks;

  /// 当前编辑音轨（PR 顶部显示它；-1 = 无）。
  final int activeTrack;

  /// PR「全部轨道」开关：开 = 不按当前编辑轨过滤。
  final bool showAllTracks;
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
  final ValueChanged<int> onQuickTool;
  final VoidCallback onUndo;
  final VoidCallback onRedo;
  final ValueChanged<int> onSelectTrack;
  final VoidCallback onToggleAllTracks;
  final VoidCallback onProjectSettings;

  /// 保存工程（无改动时按钮置灰不可点）。
  final VoidCallback onSave;

  /// 刚保存成功：2 秒内把保存按钮显示为对勾。
  final bool saveDone;

  /// 保存进行中：保存按钮位置显示加载圆环。
  final bool saving;

  /// 全曲音符数（AR 歌名弹窗内显示）。
  final int noteCount;

  /// 本工程默认 BPM / 拍号文本（点击可改；原生事件优先于它们）。
  final String bpmText;
  final String sigText;
  final VoidCallback onEditBpm;
  final VoidCallback onEditSig;

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    // 顶栏是密集工具栏：限制系统字体放大，避免长歌名/大字号把按钮挤出屏幕。
    return MediaQuery.withClampedTextScaling(
      maxScaleFactor: 1.2,
      child: Container(
        height: 42,
        padding: const EdgeInsets.symmetric(horizontal: 6),
        decoration: BoxDecoration(
          // 主题 token（与桌面 app_bg/line_fg 同色系）。
          color: YinheTheme.controlBg,
          border: Border(bottom: BorderSide(color: YinheTheme.lineFg)),
        ),
        child: Row(
          children: [
            IconButton(
              onPressed: isPr ? onBack : onHome,
              icon: Icon(isPr ? Icons.arrow_back : Icons.home),
              color: YinheTheme.text,
              tooltip: isPr ? l.backToArrangement : l.home,
              visualDensity: VisualDensity.compact,
              padding: EdgeInsets.zero,
              constraints: const BoxConstraints(minWidth: 40, minHeight: 40),
            ),
            IconButton(
              onPressed: onTogglePlay,
              icon: Icon(playing ? Icons.pause : Icons.play_arrow),
              color: YinheTheme.text,
              tooltip: playing ? l.pause : l.play,
              visualDensity: VisualDensity.compact,
              padding: EdgeInsets.zero,
              constraints: const BoxConstraints(minWidth: 40, minHeight: 40),
            ),
            IconButton(
              onPressed: onStop,
              icon: const Icon(Icons.stop),
              color: YinheTheme.textSecondary,
              tooltip: l.stop,
              visualDensity: VisualDensity.compact,
              padding: EdgeInsets.zero,
              constraints: const BoxConstraints(minWidth: 40, minHeight: 40),
            ),
            const SizedBox(width: 4),
            ValueListenableBuilder<EditStatus?>(
              valueListenable: editStatus,
              builder: (context, status, _) {
                final active = status != null;
                final hasMeta =
                    status != null &&
                    (status.primary != null || status.secondary != null);
                final showPosition = !(status?.hidePosition ?? false);
                return Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    if (showPosition)
                      InkWell(
                        borderRadius: BorderRadius.circular(4),
                        onTap: active ? null : onToggleTimecode,
                        child: Padding(
                          padding: const EdgeInsets.symmetric(
                            horizontal: 8,
                            vertical: 6,
                          ),
                          child: ValueListenableBuilder<String>(
                            valueListenable: timecodeText,
                            builder: (context, text, _) => Text(
                              status?.position ?? text,
                              style: TextStyle(
                                color: active
                                    ? YinheTheme.accent
                                    : YinheTheme.textLabel,
                                fontSize: 14,
                              ),
                            ),
                          ),
                        ),
                      ),
                    if (!hasMeta) ...[
                      _metaText(bpmText, l.defaultBpm, onEditBpm),
                      _metaText(sigText, l.defaultTimeSig, onEditSig),
                    ] else ...[
                      if (status.primary != null)
                        _metaText(status.primary!, '', null, active: true),
                      if (status.secondary != null)
                        _metaText(status.secondary!, '', null, active: true),
                    ],
                  ],
                );
              },
            ),
            Expanded(
              child: LayoutBuilder(
                builder: (context, c) {
                  // 中央标题只占左右控件之外的空隙；窄屏直接隐藏，绝不盖住按钮。
                  if (c.maxWidth < 72) {
                    return const SizedBox.shrink();
                  }
                  return Center(
                    child: ConstrainedBox(
                      constraints: BoxConstraints(
                        maxWidth: (isPr ? 220.0 : 260.0).clamp(0.0, c.maxWidth),
                      ),
                      child: isPr ? _buildSongBox(l) : _titleButton(l),
                    ),
                  );
                },
              ),
            ),
            _buildQuantizeButton(context, l),
            _buildQuickToolButton(l),
            _buildToolButton(l),
            ValueListenableBuilder<int>(
              valueListenable: docState,
              builder: (context, st, _) => Row(
                children: [
                  _iconAction(
                    Icons.undo,
                    l.undo,
                    (st & 1) != 0 ? onUndo : null,
                  ),
                  _iconAction(
                    Icons.redo,
                    l.redo,
                    (st & 2) != 0 ? onRedo : null,
                  ),
                  // 保存：从右向左第一个；无改动时置灰不可点；
                  // 保存中显示圆环；保存成功后 2 秒显示对勾（普通字体色）。
                  if (saving)
                    const SizedBox(
                      width: 40,
                      height: 40,
                      child: Center(
                        child: SizedBox(
                          width: 16,
                          height: 16,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        ),
                      ),
                    )
                  else if (saveDone)
                    _iconAction(
                      Icons.check,
                      l.saved,
                      null,
                      color: YinheTheme.text,
                    )
                  else
                    _iconAction(
                      Icons.save_outlined,
                      l.saveProject,
                      (st & 4) != 0 ? onSave : null,
                    ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }

  /// 顶栏信息文本：BPM / 拍号可点击编辑；拖动读数（active）用强调色且不可点。
  Widget _metaText(
    String text,
    String tooltip,
    VoidCallback? onTap, {
    bool active = false,
  }) {
    final content = Padding(
      padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 6),
      child: Text(
        text,
        style: TextStyle(
          color: active ? YinheTheme.accent : YinheTheme.textLabel,
          fontSize: 14,
        ),
      ),
    );
    if (onTap == null) {
      return Tooltip(message: tooltip, child: content);
    }
    return InkWell(
      borderRadius: BorderRadius.circular(4),
      onTap: onTap,
      child: Tooltip(message: tooltip, child: content),
    );
  }

  /// 歌曲标题（音符数在 AR 歌名弹窗内部）。
  String _titled(AppLocalizations l) {
    return songTitle.isNotEmpty ? songTitle : (fileName ?? l.untitled);
  }

  /// 工具名（本地化）。
  String _toolName(AppLocalizations l, int i) => switch (i) {
    0 => l.toolSelect,
    1 => l.toolHand,
    2 => l.toolPencil,
    3 => l.toolEraser,
    4 => l.toolBrush,
    _ => toolDefs[i].$2,
  };

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

  Widget _buildQuantizeButton(BuildContext context, AppLocalizations l) {
    return PopupMenuButton<String>(
      color: YinheTheme.bg,
      // 保留原展开动画，但裁剪内容：否则背景还在放大时文字会先画到背景外。
      clipBehavior: Clip.antiAlias,
      tooltip: l.quantize,
      padding: EdgeInsets.zero,
      onSelected: onQuantize,
      itemBuilder: (_) => [
        PopupMenuItem(value: '关闭', child: Text(l.quantizeOff)),
        PopupMenuItem(value: '自动', child: Text(l.quantizeAuto)),
        for (final (label, ticks) in quantizePresetsForPpq(ppq))
          PopupMenuItem(
            value: label,
            child: Text(
              label == '1 tick'
                  ? '1 tick（1/${(ppq * 4).round()} 音符）'
                  : '$label（$ticks tick）',
            ),
          ),
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
            Text(switch (quantizeLabel) {
              '关闭' => l.quantizeOff,
              '自动' => l.quantizeAuto,
              _ => quantizeLabel,
            }, style: TextStyle(color: YinheTheme.textLabel, fontSize: 13)),
          ],
        ),
      ),
    );
  }

  /// 快捷工具组：与主工具下拉框使用同一套工具。
  ///
  /// 它记住自己槽位里的工具，与主工具 popup 解耦。当前工具不是槽位工具时，
  /// 第一次点击选中槽位工具；已经是槽位工具时再点打开 popup 切换组内工具。
  Widget _buildQuickToolButton(AppLocalizations l) {
    final group = isPr ? quickToolsPr : quickToolsAr;
    final shown = group.contains(quickTool) ? quickTool : group.first;
    final active = quickSlotActive;
    final tip = group.map((i) => _toolName(l, i)).join(' / ');
    return Builder(
      builder: (context) {
        var anchor = Offset.zero;
        return Tooltip(
          message: tip,
          child: InkWell(
            borderRadius: BorderRadius.circular(4),
            onTapDown: (d) => anchor = d.globalPosition,
            onTap: () {
              if (!active) {
                onQuickTool(shown);
                return;
              }
              _showToolMenu(context, group, shown, anchor, onQuickTool);
            },
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
              child: Icon(
                toolDefs[shown].$1,
                size: 18,
                color: active ? YinheTheme.accent : YinheTheme.textSecondary,
                semanticLabel: tip,
              ),
            ),
          ),
        );
      },
    );
  }

  Future<void> _showToolMenu(
    BuildContext context,
    List<int> group,
    int shown,
    Offset anchor,
    ValueChanged<int> onSelected,
  ) async {
    final overlay =
        Overlay.of(context).context.findRenderObject() as RenderBox?;
    if (overlay == null) {
      return;
    }
    final local = overlay.globalToLocal(anchor);
    final l = AppLocalizations.of(context);
    final selected = await showMenu<int>(
      context: context,
      color: YinheTheme.bg,
      position: RelativeRect.fromLTRB(
        local.dx,
        local.dy,
        overlay.size.width - local.dx,
        overlay.size.height - local.dy,
      ),
      items: [
        for (final i in group)
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
                Text(_toolName(l, i)),
              ],
            ),
          ),
      ],
    );
    if (selected != null) {
      onSelected(selected);
    }
  }

  Widget _buildToolButton(AppLocalizations l) {
    final list = isPr ? toolPrAllowed : toolArAllowed;
    final shown = list.contains(mainTool) ? mainTool : list.first;
    final active = !quickSlotActive;
    return Builder(
      builder: (context) {
        var anchor = Offset.zero;
        return Tooltip(
          message: l.tool,
          child: InkWell(
            borderRadius: BorderRadius.circular(4),
            onTapDown: (d) => anchor = d.globalPosition,
            onTap: () {
              if (!active) {
                onTool(shown);
                return;
              }
              _showToolMenu(context, list, shown, anchor, onTool);
            },
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
              child: Icon(
                toolDefs[shown].$1,
                size: 18,
                color: active ? YinheTheme.accent : YinheTheme.textSecondary,
                semanticLabel: _toolName(l, shown),
              ),
            ),
          ),
        );
      },
    );
  }

  /// AR：歌名按钮（点击直接打开工程设置大窗口；音符数在窗口内部）。
  Widget _titleButton(AppLocalizations l) {
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
              (st & 4) != 0 ? '• ${_titled(l)}' : _titled(l),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(color: YinheTheme.textLabel, fontSize: 13),
            ),
          ),
        ),
      ),
    );
  }

  /// PR：当前编辑音轨长按钮（顶部正中央）+「全部轨道」开关。
  Widget _buildSongBox(AppLocalizations l) {
    final active = activeTrack >= 0 && activeTrack < tracks.length;
    final base = active ? tracks[activeTrack].name : _titled(l);
    return PopupMenuButton<int>(
      color: YinheTheme.bg,
      clipBehavior: Clip.antiAlias,
      tooltip: l.currentTrack,
      padding: EdgeInsets.zero,
      onSelected: (v) {
        if (v >= tracks.length) {
          onToggleAllTracks();
        } else {
          // 直接选轨 = 只看该轨；先关掉「全部轨道」再切换当前编辑轨。
          if (showAllTracks) {
            onToggleAllTracks();
          }
          onSelectTrack(v);
        }
      },
      itemBuilder: (_) => [
        PopupMenuItem<int>(
          enabled: false,
          height: 30,
          child: Text(
            l.currentTrack,
            style: TextStyle(color: YinheTheme.textLabel, fontSize: 11),
          ),
        ),
        CheckedPopupMenuItem<int>(
          value: tracks.length,
          checked: showAllTracks,
          child: Text(l.allTracks),
        ),
        const PopupMenuDivider(height: 1),
        for (var i = 0; i < tracks.length; i++)
          CheckedPopupMenuItem<int>(
            value: i,
            checked: i == activeTrack,
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
              color: showAllTracks ? YinheTheme.text : YinheTheme.accent,
              fontSize: 12,
            ),
          ),
        ),
      ),
    );
  }
}
