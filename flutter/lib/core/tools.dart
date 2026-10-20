import 'package:flutter/material.dart';

/// 工具定义（图标 + 名称；索引与 Rust 编号对齐：
/// 0 选框、1 手形、2 铅笔、3 橡皮、4 刷子）。
const List<(IconData, String)> toolDefs = [
  (Icons.highlight_alt, '选择'),
  (Icons.pan_tool, '手形'),
  (Icons.edit, '铅笔'),
  (Icons.auto_fix_normal, '橡皮擦'),
  (Icons.brush, '刷子'),
];

/// PR 允许的工具。
const List<int> toolPrAllowed = [0, 1, 2, 3, 4];

/// AR 允许的工具。
const List<int> toolArAllowed = [0, 1];
