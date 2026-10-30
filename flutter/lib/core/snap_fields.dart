/// 视图快照字段索引。
///
/// 与 `crates/yinhe-flutter/src/scene.rs::view_snapshot` 的 13 个 f32 一一对应；
/// 加字段时两边必须同步，这里用命名常量替代 `s[0]…s[12]` 的魔法下标。
abstract final class SnapField {
  /// 0 = AR，1 = PR。
  static const mode = 0;
  static const ppu = 1;
  static const scrollX = 2;
  static const scrollY = 3;
  static const keyHeight = 4;

  /// 左侧面板宽（PR = 键盘宽，AR = 轨道面板宽）。
  static const leftPanelW = 5;
  static const laneHeight = 6;
  static const totalTicks = 7;
  static const ppq = 8;

  /// 键 0 的 y（PR）。
  static const key0Y = 9;
  static const sigNum = 10;
  static const sigDen = 11;
  static const bpm = 12;

  /// 快照总长度。
  static const length = 13;
}
