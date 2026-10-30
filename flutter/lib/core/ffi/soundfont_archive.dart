part of '../ffi.dart';

typedef CStrVoidNative = ffi.Void Function(ffi.Pointer<Utf8>);
typedef CStrVoidDart = void Function(ffi.Pointer<Utf8>);

/// 音色库入表 + 持久化（平台壳只负责把文件拷进沙箱并回路径）。
final CStrVoidDart addSoundfont = bridge
    .lookupFunction<CStrVoidNative, CStrVoidDart>('yinhe_add_soundfont');

typedef SfArchiveOpenNative = ffi.Int32 Function(ffi.Pointer<Utf8>);
typedef SfArchiveOpenDart = int Function(ffi.Pointer<Utf8>);

/// 打开音色库压缩包（后台列 SF2/SFZ）。
final SfArchiveOpenDart soundfontArchiveOpen = bridge
    .lookupFunction<SfArchiveOpenNative, SfArchiveOpenDart>(
      'yinhe_soundfont_archive_open',
    );

typedef SfArchiveOpenPwNative = ffi.Int32 Function(
  ffi.Pointer<Utf8>,
  ffi.Pointer<Utf8>,
);
typedef SfArchiveOpenPwDart = int Function(
  ffi.Pointer<Utf8>,
  ffi.Pointer<Utf8>,
);

/// 带密码打开音色库压缩包。
final SfArchiveOpenPwDart soundfontArchiveOpenPassword = bridge
    .lookupFunction<SfArchiveOpenPwNative, SfArchiveOpenPwDart>(
      'yinhe_soundfont_archive_open_password',
    );

typedef SfArchiveStatusNative = ffi.Int32 Function();
typedef SfArchiveStatusDart = int Function();

/// 0=无 1=打开中 2=就绪 3=需要密码 4=失败。
final SfArchiveStatusDart soundfontArchiveStatus = bridge
    .lookupFunction<SfArchiveStatusNative, SfArchiveStatusDart>(
      'yinhe_soundfont_archive_status',
    );

/// 音色库压缩包条目 JSON。
final CStrDart soundfontArchiveJson = bridge
    .lookupFunction<CStrNative, CStrDart>('yinhe_soundfont_archive_json');

typedef SfArchiveImportNative = ffi.Int32 Function(ffi.Int32);
typedef SfArchiveImportDart = int Function(int);

/// 导入第 index 个条目（后台解压）。
final SfArchiveImportDart soundfontArchiveImport = bridge
    .lookupFunction<SfArchiveImportNative, SfArchiveImportDart>(
      'yinhe_soundfont_archive_import',
    );

/// 音色库导入进度 JSON。
final CStrDart soundfontArchiveImportJson = bridge
    .lookupFunction<CStrNative, CStrDart>(
      'yinhe_soundfont_archive_import_json',
    );
