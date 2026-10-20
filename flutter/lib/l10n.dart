import 'package:flutter/widgets.dart';

import 'l10n_gen.dart';

/// 轻量多语言：中文原文 → 英文译文。
///
/// - 界面里用 `tr('中文')` 取当前语言文本；没有译文时回退中文。
/// - `appLanguage` 取值 `system` / `zh` / `en`，持久化在 Rust settings。
/// - 语言切换通过 [appLanguage] 通知整棵树重建（main 里监听）。
class L10n {
  /// 桌面端同款 15 种语言。
  static const List<String> supported = [
    'zh-CN',
    'zh-TW',
    'zh-HK',
    'en-US',
    'ja-JP',
    'ko-KR',
    'de-DE',
    'fr-FR',
    'es-ES',
    'it-IT',
    'pt-BR',
    'ru-RU',
    'pl-PL',
    'cs-CZ',
    'tr-TR',
  ];

  /// 当前语言设置（system = 跟随系统；否则是 supported 里的完整代码）。
  static final ValueNotifier<String> appLanguage = ValueNotifier('system');

  /// 解析后的实际语言代码。
  static String get resolved {
    final v = appLanguage.value;
    if (v != 'system') {
      return v;
    }
    final l = WidgetsBinding.instance.platformDispatcher.locale;
    final exact = '${l.languageCode}-${l.countryCode ?? ''}';
    if (supported.contains(exact)) {
      return exact;
    }
    for (final code in supported) {
      if (code.split('-').first == l.languageCode) {
        return code;
      }
    }
    return l.languageCode == 'zh' ? 'zh-CN' : 'en-US';
  }

  /// 当前 Locale（给 Material 组件用）。
  static Locale get locale {
    final parts = resolved.split('-');
    return Locale.fromSubtags(
      languageCode: parts[0],
      countryCode: parts.length > 1 ? parts[1] : null,
    );
  }

  static bool get isZh => resolved.startsWith('zh');
}

/// 取当前语言文本（中文原文作为 key）。
String tr(String zh) {
  final lang = L10n.resolved;
  if (lang == 'zh-CN') {
    return zh;
  }
  final desktop = kDesktopL10n[lang];
  final v = desktop?[zh];
  if (v != null) {
    return v;
  }
  if (lang == 'zh-TW' || lang == 'zh-HK') {
    return zh;
  }
  if (lang == 'en-US') {
    return _en[zh] ?? zh;
  }
  // 其它语言缺译文时回退英文，再回退中文。
  return _en[zh] ?? zh;
}

const Map<String, String> _en = {
  // ── 通用 ──
  '取消': 'Cancel',
  '保存': 'Save',
  '确定': 'OK',
  '返回': 'Back',
  '删除': 'Delete',
  '修改': 'Edit',
  '复制': 'Copy',
  '粘贴': 'Paste',
  '关闭': 'Close',
  '音符': 'notes',
  '加载更多': 'Load more',
  '没有事件': 'No events',
  // ── 主页 ──
  '打开文件': 'Open file',
  '新建工程': 'New project',
  '另存为': 'Save as',
  '设置': 'Settings',
  '最近修改的文件': 'Recent files',
  '暂无最近文件': 'No recent files',
  '返回工作区': 'Back to workspace',
  // ── 顶部栏 ──
  '主页': 'Home',
  '返回走带': 'Back to arrangement',
  '播放': 'Play',
  '暂停': 'Pause',
  '停止': 'Stop',
  '量化（吸附档位）': 'Quantize',
  '关闭吸附': 'Off',
  '自动（随缩放）': 'Auto (by zoom)',
  '工具': 'Tool',
  '撤销': 'Undo',
  '重做': 'Redo',
  '保存工程': 'Save project',
  '已保存': 'Saved',
  '当前编辑音轨': 'Current track',
  '全部轨道': 'All tracks',
  '本工程默认 BPM': 'Project default BPM',
  '本工程默认拍号': 'Project default time signature',
  '工程': 'Project',
  '首页': 'Home',
  // ── 工程设置 ──
  '工程设置': 'Project settings',
  '歌名': 'Title',
  '艺术家': 'Artist',
  '简介': 'Description',
  'PPQ（已加载音符时修改会缩放全部时间）': 'PPQ (rescales time when notes exist)',
  'BPM（0–60000000）': 'BPM (0–60000000)',
  'BPM 需为 0–60000000': 'BPM must be 0–60000000',
  '拍号（如 4/4）': 'Time signature (e.g. 4/4)',
  // ── 设置页 ──
  '音色库': 'Soundfonts',
  '音频': 'Audio',
  '语言': 'Language',
  '跟随系统': 'Follow system',
  '简体中文': '简体中文',
  'English': 'English',
  '复音上限': 'Max voices',
  '音频缓冲': 'Audio buffer',
  '忽略力度': 'Ignore velocity',
  '自动': 'Auto',
  '设备默认': 'Device default',
  '添加音色库': 'Add soundfont',
  '移除': 'Remove',
  // ── 音轨菜单/信息 ──
  '编辑音轨信息': 'Edit track info',
  '事件列表': 'Event list',
  '删除音轨': 'Delete track',
  '音轨名称': 'Track name',
  '端口': 'Port',
  '通道': 'Channel',
  '本通道默认乐器：跟随 MIDI PC': 'Channel default instrument: follow MIDI PC',
  '选择': 'Select',
  '清除': 'Clear',
  '选择本通道默认乐器': 'Select channel default instrument',
  // ── 动态/插入 ──
  '插入': 'Insert',
  '速度（Tempo）': 'Tempo',
  '拍号 Time Signature': 'Time signature',
  '调号 Key Signature': 'Key signature',
  '标签 Marker': 'Marker',
  '自定义 CC 号…': 'Custom CC…',
  '自定义 RPN 号…': 'Custom RPN…',
  '主音': 'Root',
  '调式': 'Scale',
  '大调 Major': 'Major',
  '自然小调 Minor': 'Natural minor',
  '值': 'Value',
  // ── 压缩包 ──
  '压缩包需要密码': 'Password required',
  '密码': 'Password',
  '打开': 'Open',
  '压缩包里没有 MIDI 文件': 'No MIDI files in archive',
  // ── 添加音轨 ──
  '添加音轨': 'Add tracks',
  '轨道数量': 'Track count',
  '起始通道': 'Start channel',
  '循环分配通道': 'Cycle channels',
  '每轨使用相同通道': 'Same channel for all',
  '添加': 'Add',
  // ── 事件列表 ──
  '粘贴（同时间位置）': 'Paste (same time)',
  '前往该地点': 'Go to location',
  '已复制事件': 'Event copied',
  '力度': 'Velocity',
  '长度': 'Length',
  '力度（1–127）': 'Velocity (1–127)',
  '长度（tick）': 'Length (ticks)',
  '修改音符': 'Edit note',
  '请先打开 MIDI 或新建工程': 'Open a MIDI or create a project first',
  '工程导出失败': 'Project export failed',
  '已复制': 'Copied',
  '复制选区': 'Copy selection',
  // ── 工具 ──
  '手形': 'Hand',
  '铅笔': 'Pencil',
  '橡皮擦': 'Eraser',
  '刷子': 'Brush',
  // ── 音轨信息 ──
  '编辑 Master 轨信息': 'Edit master track',
  '歌曲标题': 'Song title',
  'Master（Conductor）轨承载速度/拍号等全局事件，不能放置音符，也没有端口/通道设置。': 'The master (Conductor) track holds tempo/time-signature events; it has no notes and no port/channel.',
  '通道级默认乐器：同一通道的所有音轨共享；MIDI 内的 PC 事件优先于它。': 'Channel default instrument is shared by all tracks on that channel; MIDI PC events take priority.',
  '移动端目前仅端口 A 参与播放；其他端口会保存，供桌面端使用。': 'Only port A is audible on mobile; other ports are saved for the desktop app.',
  '旋律 Bank 0': 'Melodic Bank 0',
  '鼓组 Bank 128': 'Drum Bank 128',
  // ── 添加音轨 ──
  '轨道数量（1–256）': 'Track count (1–256)',
  '数量需为 1–256': 'Count must be 1–256',
  '从通道开始循环': 'Cycle from channel',
  '全部相同通道': 'Same channel',
  // ── 自动化面板 ──
  '自动化类型': 'Automation type',
  // ── 杂项 ──
  '压缩包打开失败': 'Failed to open archive',
  '自定义 CC 号': 'Custom CC number',
  '自定义 RPN 号': 'Custom RPN number',
  '开启': 'On',
  '插入拍号': 'Insert time signature',
  '插入调号': 'Insert key signature',
};
