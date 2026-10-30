/// 平台文件契约：三端壳只负责「弹系统选择器 + 拷贝到沙箱/写回目标」，
/// 扩展名策略、压缩包判定、拒绝提示都在这一层，避免每家壳各抄一份文件类型表。
///
/// 方法约定（`yinhe/surface` 通道）：
/// - `openFilePicker` / `openSoundFontPicker` 带 `exts`（小写、不含点）：
///   壳按它校验用户选择的文件，不匹配回 `onFileRejected(name)`；
///   匹配则复制进沙箱后回 `onFilePicked(path)` / `onSoundFontPicked(path)`。
/// - `saveProject` / `saveExport` 由壳选位置并写回，回 `onProjectSaved` /
///   `onExportSaved` / `onProjectSaveCancelled` / `onProjectSaveFailed`。
library;

/// 压缩包扩展名（与 Rust/桌面同一份列表）。
const List<String> archiveExtensions = [
  'zip',
  '7z',
  'rar',
  'lzh',
  'lha',
  'tar',
  'gz',
  'xz',
  'tgz',
  'txz',
  'tbz',
  'bz2',
];

/// 打开工程：MIDI / `.yin` / 压缩包。
final List<String> projectExtensions = [
  'mid',
  'midi',
  'yin',
  ...archiveExtensions,
];

/// 音色库：裸 SF2/SFZ + 压缩包（包内可含 SF2/SFZ 及采样）。
final List<String> soundfontExtensions = ['sf2', 'sfz', ...archiveExtensions];

/// 是否压缩包扩展名。
bool isArchivePath(String path) {
  final lower = path.toLowerCase();
  return archiveExtensions.any((e) => lower.endsWith('.$e'));
}
