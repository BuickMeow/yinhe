import 'package:flutter/material.dart';

import '../l10n/app_localizations.dart';
import 'inputs.dart';

/// 本工程默认 BPM / 拍号编辑对话框。
///
/// 这两个值只在 conductor 没有原生 Tempo/拍号事件时生效：
/// 原生事件优先级更高，可以完全代替它们。

/// 弹出「本工程默认 BPM」编辑框；返回保存的 BPM（取消返回 null）。
Future<double?> showDefaultBpmDialog(BuildContext context, double initial) {
  final l = AppLocalizations.of(context);
  return showDecimalInputDialog(
    context,
    title: l.defaultBpm,
    label: l.bpmField,
    min: 0,
    max: 60000000,
    initial: initial,
    errorText: l.bpmError,
  );
}

/// 弹出「本工程默认拍号」编辑框；返回 `(分子, 分母)`（取消返回 null）。
Future<(int, int)?> showDefaultTimeSigDialog(
  BuildContext context,
  int num,
  int den,
) {
  final l = AppLocalizations.of(context);
  return showTimeSigInputDialog(
    context,
    title: l.defaultTimeSig,
    label: l.timeSigField,
    numerator: num,
    denominator: den,
  );
}
