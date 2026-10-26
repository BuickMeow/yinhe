/// 顶部栏临时编辑读数。
///
/// 非 null 时覆盖常规的「位置 / BPM / 拍号」显示：
/// - `position`：位置槽（为空则继续显示播放位置时间码）；
/// - `primary` / `secondary`：BPM、拍号两个槽位的临时文本。
class EditStatus {
  const EditStatus({
    this.position,
    this.primary,
    this.secondary,
    this.hidePosition = false,
  });

  final String? position;
  final String? primary;
  final String? secondary;

  /// true = 不显示左侧位置槽（相对拖动的读数只占后两个槽）。
  final bool hidePosition;
}
