import 'package:flutter/widgets.dart';

/// 移动端多语言设置（文案资源见 `lib/l10n/*.arb`，由 Flutter gen_l10n 生成
/// `AppLocalizations`；界面用 `AppLocalizations.of(context).xxx` 取文案）。
///
/// - `appLanguage` 取值 `system` 或 15 种语言代码之一，持久化在 Rust settings。
/// - 语言切换通过 [appLanguage] 通知整棵树重建（main 里监听）。
class L10n {
  /// 与 ARB 资源一致的 15 种语言（zh 为简体；zh-TW/zh-HK 为繁体）。
  static const List<String> supported = [
    'zh',
    'zh-TW',
    'zh-HK',
    'en',
    'ja',
    'ko',
    'de',
    'fr',
    'es',
    'it',
    'pt',
    'ru',
    'pl',
    'cs',
    'tr',
  ];

  /// 当前语言设置（system = 跟随系统；否则是 supported 里的代码）。
  static final ValueNotifier<String> appLanguage = ValueNotifier('system');

  /// 解析后的实际语言代码（保证在 supported 内）。
  static String get resolved {
    var v = appLanguage.value;
    // 兼容旧设置。
    if (v == 'zh-CN') {
      v = 'zh';
    } else if (v == 'en-US') {
      v = 'en';
    }
    if (v != 'system' && supported.contains(v)) {
      return v;
    }
    final l = WidgetsBinding.instance.platformDispatcher.locale;
    if (l.languageCode == 'zh') {
      if (l.countryCode == 'TW') {
        return 'zh-TW';
      }
      if (l.countryCode == 'HK' || l.countryCode == 'MO') {
        return 'zh-HK';
      }
      return 'zh';
    }
    return supported.contains(l.languageCode) ? l.languageCode : 'en';
  }

  /// 当前 Locale（给 Material / gen_l10n 用）。
  static Locale get locale {
    return switch (resolved) {
      'zh-TW' => const Locale('zh', 'TW'),
      'zh-HK' => const Locale('zh', 'HK'),
      final code => Locale(code),
    };
  }

  static bool get isZh => resolved.startsWith('zh');
}
