#import "GeneratedPluginRegistrant.h"

#include <stdint.h>

// ── Rust 引擎（libyinhe_flutter.a）导出的 C 入口 ──
// 与 crates/yinhe-flutter 的 iOS 一节一一对应；Android 那套在 JNI 里。

void yinhe_ios_init(const char *files_dir);
void yinhe_ios_set_documents_dir(const char *documents_dir);
void yinhe_ios_ensure(int32_t kind);
void yinhe_ios_set_size(int32_t kind, int32_t width, int32_t height);
void yinhe_ios_set_viewport(float width, float height);
void *yinhe_ios_metal_device(void);

/// 取一个可写纹理槽位（返回 MTLTexture*，槽位写入 slot）。
typedef void *(*yinhe_ios_acquire_fn)(int32_t kind, uint32_t width, uint32_t height,
                                     int32_t *slot);
/// 提交一帧（标记最新槽位 + 通知 Flutter）。
typedef void (*yinhe_ios_submit_fn)(int32_t kind, int32_t slot);
void yinhe_ios_set_host(yinhe_ios_acquire_fn acquire, yinhe_ios_submit_fn submit);
