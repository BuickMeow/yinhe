# 鸿蒙（HarmonyOS）适配计划

> 结论先行：**兼容版鸿蒙（4.x 及以下）走 APK，已修复致命门槛；纯血鸿蒙（5.x/NEXT/6）必须出 HAP，
> 需要新壳 + 两个后端（图形/音频），目前只完成了平台分支铺垫。**

## 一、先分清两条路线

| 系统 | 能否装 APK | 我们要做的事 |
| --- | --- | --- |
| HarmonyOS 2/3/4（含 4.2，AOSP 内核，报 API 29 左右） | 能 | 修构建 API、存储权限、驱动回退；纯配置工作 |
| HarmonyOS 5.0 / NEXT / 6（纯血，API 15+） | **不能**（只有 HAP） | 新建 OHOS 壳 + 平台后端，工程量大 |

卓易通/出境易 是 iSulad 容器方案（华为规划 2026 年底前随存量升级退场），只能当过渡，不能作为主线。

## 二、APK 路线（鸿蒙 4 及以下）

已修复：

- `cargo ndk -P 35` → `-P 26`。旧的 35 让 `libyinhe_flutter.so` 依赖
  `_Unwind_*@LIBC_R`（API 30），API 29 的设备（安卓 10 / 鸿蒙 4）在
  `dlopen` 阶段直接失败，表现为"装了但打不开"。
- `minSdk` 显式 26（`libaaudio.so` 自 API 26 才有，`.so` 直接 NEEDED）。

仍需真机确认：

1. **存储权限**：当前用 `MANAGE_EXTERNAL_STORAGE` + SAF。鸿蒙对"所有文件访问"
   支持不完整，`externalStoragePath()` 可能永远拿不到真实路径（会退回复制到
   `filesDir`，功能可用但音色库的采样相对路径会失效）。后续建议改为默认走 SAF
   目录授权（`ACTION_OPEN_DOCUMENT_TREE` + 持久化 URI）。
2. **图形**：wgpu 默认 Vulkan，回退 GL。鸿蒙 4 的 Kirin 驱动多为 Vulkan 1.0/1.1，
   若 `request_device` 失败会**黑屏而非闪退**，日志里能看到 `适配器` / `创建设备失败`。
3. **音频**：AAudio 在鸿蒙 4 上存在但回调线程优先级未知，若出现爆音/欠载，
   参考 MIUI 的做法继续放大 `PREVIEW_TARGET_FRAMES`（已按移动端 4096 帧处理）。
4. **安装**：鸿蒙 4 需关闭"纯净模式"（设置 → 系统和更新 → 纯净模式）才能侧载。

## 三、HAP 路线（纯血鸿蒙）

### 3.1 现有架构 → 鸿蒙的逐项映射

| 现在（Android） | 鸿蒙对应物 | 工作量 |
| --- | --- | --- |
| `MainActivity.kt` 的 `TextureRegistry.SurfaceProducer` | ArkTS `XComponent(type: 'surface')` 回调拿到 `OHNativeWindow` | 小 |
| wgpu 在 `ANativeWindow` 上建 surface | `VK_OHOS_surface` + `vkCreateSurfaceOHOS(instance, window)` | **需要 patch wgpu-hal**（当前无 OHOS surface 实现） |
| 8 个 `Java_com_jieneng_yinhe_*` JNI 入口 | 改为纯 `extern "C"` + 一层 NAPI 胶水（ArkTS 侧 import） | 小（其余 FFI 已是 `extern "C"` + dart:ffi） |
| `cpal` AAudio 后端 | OHAudio（`OH_AudioStreamBuilder`），`ohos-audio-sys`/`ohoaudio-sys` 有绑定 | 中（cpal 无 OHOS 后端，需自写或 fork） |
| `paths.rs` 的 `getFilesDir()` | 沙箱 `HOME`（已加 cfg 分支，真机核对） | 小 |
| `android_logger` | hilog（`OH_LOG_Print`，可用 `ohos_hilog`） | 小 |
| SAF 文件选择 | `DocumentViewPicker`（ArkTS 侧） | 小 |
| Flutter 壳 | **Flutter-OH 分支**（社区维护，最新 3.35.7） | 中（本仓库是 3.47.7，需评估降级或等待） |

### 3.2 已实测的编译现状（`--target aarch64-unknown-linux-ohos`）

Rust target 已安装（Tier 2，`aarch64-unknown-linux-ohos`），`cargo check` 结果：

- 通过：`yinhe-types`、`yinhe-memtrace`、`yinhe-dsp`、`yinhe-synth`、`yinhe-core`、`yinhe-wgpu`
- 卡住：
  - `yinhe-archive` / `yinhe-yin`：`zstd-sys` 构建脚本无 C 交叉编译器（缺 HarmonyOS NDK 的 clang）
  - `yinhe-midi-io` / `yinhe-audio`：`alsa-sys` —— cpal 把 OHOS 当 Linux，选中 ALSA 后端

也就是说：**纯 Rust 核心在鸿蒙上能编；剩下的都是 C 依赖和平台后端**。
装上 HarmonyOS SDK 后，`zstd-sys` 只需把 `CC_aarch64_unknown_linux_ohos` 指向
`ohos-clang`；音频则必须真正实现 OHAudio 后端（`cpal` 上游短期内不会支持）。

### 3.3 里程碑（建议顺序）

1. **图形 spike**（最高风险）：最小 OHOS 工程 + `XComponent` + 裸 Vulkan `vkCreateSurfaceOHOS`，
   确认 `VK_OHOS_surface` 在目标机型可用；再决定是给 wgpu-hal 提 OHOS surface 补丁还是本地 fork。
2. **音频 spike**：用 `ohos-audio-sys` 写一个最小 `AudioBackend`，先不接 cpal，验证回调线程与延迟。
3. **壳迁移**：`flutter create --platforms=ohos .` + ArkTS 侧 NAPI 胶水，替换 8 个 JNI 入口。
4. **端到端**：打开工程 → 卷帘渲染 → 播放 → 编辑 → 导出。
5. **分发**：AGC 签名（.p7b/.cer/.p12），上架需要软著等材料。

### 3.4 环境依赖（当前机器都没有）

- DevEco Studio + HarmonyOS SDK（API 15+），用于 `ohos-clang`、`hvigor`、`hdc`
- Flutter-OH：`https://gitcode.com/openharmony-tpc/flutter_flutter`（`oh-3.35.7-release`）
- 真机或模拟器（只支持 arm64）

## 四、本次已落地的改动

- 安卓 API ≤ 29 启动失败修复（`-P 26`、去掉 35/ 软链、`minSdk=26`）
- 平台分支：`yinhe-memtrace`（系统分配器 / `mallopt` 直调）、`yinhe-editor-core::paths`
  （`HOME` 沙箱）、`yinhe-audio`（预览缓冲按移动端）、`yinhe-clap`（无插件目录）、
  `yinhe-flutter::audio`（自动复音上限）
