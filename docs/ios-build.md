# iOS 构建与运行

## 一次性准备

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
```

Xcode 与 iOS SDK 用系统自带的即可（`xcodebuild -version` 能看到）。真机调试要在
Xcode 的 Signing & Capabilities 里配一次开发者账号（免费账号也能装，证书 7 天过期）。

## 常用命令

```bash
cargo xtask ios              # 模拟器 debug 包（Rust 一律 release）
cargo xtask ios --release    # 模拟器 release 包
cargo xtask ios --device     # 真机包（不签名，签名在 Xcode 里配）
cargo xtask ios --install    # 构建后装到已启动的模拟器并打开
```

**Rust 改了必须走 `cargo xtask ios`**：它先编译 `libyinhe_flutter.a` 再调
`flutter build ios`。只改 Dart 时可以直接 `flutter run -d <模拟器>`（静态库没变，
不需要重编 Rust）。

产物：`flutter/build/ios/iphonesimulator/Runner.app`，Bundle ID `com.jieneng.yinhe`。

## 为什么是静态库

iOS 不允许 app 里 dlopen 自己带的 dylib，所以 `yinhe-flutter` 同时产出
`cdylib`（Android/鸿蒙）和 `staticlib`（iOS）。静态库放在
`flutter/ios/yinhe_engine/`（不入库），由 `Flutter/{Debug,Release}.xcconfig`
的 `OTHER_LDFLAGS` 用 `-force_load` 链进 Runner。

**Release 额外需要导出清单**：release 开启 dead_strip，会把「没有任何原生代码
引用、只由 Dart `DynamicLibrary.process()` 用 dlsym 查」的 FFI 函数删掉。
`cargo xtask ios` 会从静态库里抽出全部 `_yinhe_*` 符号写成
`ios/yinhe_engine/exports.txt`（入库），由 `Release.xcconfig` 传给
`-Wl,-exported_symbols_list`。Debug 不用（不 strip，而且那份 LDFLAGS 也会作用
在调试用 `Runner.debug.dylib` 上，限制导出会把它的入口符号藏掉）。

加新的 FFI 入口后不用手改清单：下次 `cargo xtask ios` 会重新生成。

## 纹理链路（与 Android/鸿蒙的差异）

| | Android | iOS |
|---|---|---|
| 生产端 | `SurfaceProducer` → `ANativeWindow` | 宿主 `YinheTexture` 的 `CVPixelBuffer`（IOSurface 背书） |
| wgpu 呈现 | 在该窗口上建 `wgpu::Surface` 并 present | 没有 surface：把 CVPixelBuffer 的 `MTLTexture` 用 `wgpu-hal` 包成 wgpu 纹理，直接画进去（零拷贝） |
| 帧缓冲 | BufferQueue 由系统管 | 宿主自管 3 个槽位轮转，跳过 Flutter 正在持有的那块 |
| 尺寸 | `setSize` 重配交换链 | `setSize` 时宿主重建纹理池 |

同步点：Dart 每帧调 `yinhe_render_frame()` → 渲染线程向宿主 `acquire` 一个槽位 →
画完 `submit` → 宿主 `textureFrameAvailable`。

## 已知限制 / 待办

- 文件选择与保存（`openFilePicker` / `saveProject` / `saveExport`）仍是 Phase 2：
  要接 `UIDocumentPicker` + 沙箱拷贝 + security-scoped bookmark。
- `AVAudioSession` 还没配置成 `.playback`：真机上拨到静音开关会没声。
- 模拟器只出 arm64 切片（`EXCLUDED_ARCHS[sdk=iphonesimulator*] = x86_64`），
  Intel Mac 上跑模拟器需要额外编 `x86_64-apple-ios` 并 `lipo` 合并。
- `flutter` 不支持模拟器 release（`Release mode is not supported for simulators`），
  所以 release 包只能在真机上验证；模拟器用 debug。
- iOS 模拟器的软件 GPU 没有 `INDIRECT_EXECUTION`：`yinhe-wgpu` 会按
  `Adapter::get_downlevel_capabilities()` 自动退回 CPU 回读路径
  （`GpuCaps::indirect_execution`），真机走间接绘制。
