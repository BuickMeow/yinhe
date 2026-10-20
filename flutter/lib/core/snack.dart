import 'package:flutter/material.dart';

/// 统一提示条（默认 1.6s）。
void showSnack(
  BuildContext context,
  String text, {
  Duration duration = const Duration(milliseconds: 1600),
}) {
  ScaffoldMessenger.of(context)
      .showSnackBar(SnackBar(content: Text(text), duration: duration));
}
