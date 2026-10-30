# 鸿蒙（OpenHarmony/HarmonyOS）构建说明

**现状：debug HAP 可构建（未签名）**，Flutter 壳 + Rust 引擎（wgpu/GLES + OHAudio）都已进包，
尚未在真机上验证（装真机需要华为签名）。

## 一、构建

```sh
# 需要 hvigorw 在 PATH（DevEco 自带，或本机的 ~/harmony/bin/hvigorw 包装脚本）
export PATH=~/harmony/bin:~/harmony/oh-command-line-tools/bin:$PATH
cargo xtask ohos            # debug
cargo xtask ohos --release  # release（签名后才会产出 signed.hap）
```

xtask 会做四件事：
1. `cargo build --target aarch64-unknown-linux-ohos -p yinhe-flutter`（自动设置 ohos-clang、
   sysroot、`-lstdc++` → `libc++_shared` 的软链）；
2. 把 `libyinhe_flutter.so` + `libc++_shared.so` 拷到 `flutter/ohos/entry/libs/arm64-v8a`（gitignore）；
3. 缺 `flutter-hvigor-plugin` 时自动 `npm install`；
4. `hvigorw assembleHap`（内部会跑 `flutter assemble` 编 Dart）。

产物：`flutter/ohos/entry/build/default/outputs/default/entry-default-unsigned.hap`

> 不走 `flutter build hap`：它强制要求 HarmonyOS（商业）SDK（`HOS_SDK_HOME`），
> 本机只有 OpenHarmony SDK。

## 二、本机环境

| 组件 | 位置 | 说明 |
| --- | --- | --- |
| OpenHarmony SDK 26 (arm64) | `~/harmony/sdk26/26.0.0` | `OHOS_BASE_SDK_HOME=~/harmony/sdk26` |
| OpenHarmony SDK 23（hdc 用） | `~/harmony/sdk` | `OHOS_SDK_HOME`，提供 `hdc` |
| Flutter-OH 3.47.7 | `~/harmony/flutter-oh` | 与官方 3.47.7 同 engine revision |
| ohpm 5.0.2 | `~/harmony/oh-command-line-tools/bin` | |
| hvigor 6.26.8 | npm 全局 + `~/harmony/bin/hvigorw` | 顶替 DevEco |

## 三、运行链路

```
Dart: MethodChannel('yinhe/surface').getTextureId()
  → ArkTS YinheSurfacePlugin: renderer.registerTexture()
  → Flutter-OH 外接纹理的 OHNativeWindow*（bigint）
  → NAPI（libyinhe_napi.so）→ yinhe_ohos_set_surface(ptr)
  → Rust 渲染线程：wgpu Surface（RawWindowHandle::OhosNdk，GLES 后端）
  → Flutter Texture(textureId) 合成
```

- 尺寸：Dart `setSize(w,h,lw,lh)` → `renderer.setTextureBufferSize` + `yinhe_ohos_set_size/set_viewport`；
- AM 面板同理（`setAutomationSize` / 独立的第二个纹理）；
- 音频：`OHAudio`（`OH_AudioRenderer`，48k/立体声/F32），不走 cpal。

## 四、配置要点（换 SDK 版本时要一起改）

1. `build-profile.json5`：`runtimeOS: OpenHarmony`，`compileSdkVersion/compatibleSdkVersion/targetSdkVersion: "26.0.0"`。
   - API 10–25 写数字（如 `23`），API ≥ 26 必须写字符串（`"26.0.0"`）。
2. `hvigor/hvigor-config.json5`、`oh-package.json5`：`modelVersion` 必须是 `6.0.0`。
3. `entry/build-profile.json5`：`runtimeOS: OpenHarmony` + `externalNativeOptions`（NAPI 的 CMake）。
4. `entry/src/main/module.json5`：`deviceTypes` 用 `"default"`（OpenHarmony SDK 没有 `phone` 的 syscap）。
5. SDK 端（本机 shim 已做）：
   - SDK 目录名必须是 `26.0.0`（hvigor 按 platformVersion 找，不认 `26/`）；
   - 每个组件目录（ets/js/native/toolchains/previewer）需要 `sdk-pkg.json`：
     `{"meta":{"version":"3.0.0"},"data":{"path":"26.0.0,<comp>","platformVersion":"26.0.0","apiVersion":"26","fullApiVersion":"26.0.0",...}}`
   - `oh-uni-package.json` 的 `metaVersion` 需要是 `3.0.0`。
6. `GeneratedPluginRegistrant.ets` 是 Flutter 工具生成的（无插件时为空实现），已提交。

## 五、尚未完成 / 已知限制

- **签名**：产物是 unsigned。装真机需要华为签名（调试证书 profile + 设备 UDID 注册，
  需要华为开发者账号）。
- **真机未验证**：图形（GLES）、音频（OHAudio）、触摸、横竖屏都没在设备上跑过。
- **日志**：OHOS 上还没有 hilog 后端（`log` 宏目前丢弃），排查问题前需要补。
- **文件选择 / 导出**：`YinheSurfacePlugin` 对未实现的方法返回 `unsupported`。
- **性能**：OHOS 走 GLES 后端（Vulkan 需要给 wgpu-hal 补 `VK_OHOS_surface`）；
  黑乐谱的间接绘制策略未在 Mali GPU 上调过。
