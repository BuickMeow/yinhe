# 鸿蒙调试清单

## 一、常用命令

```sh
# 设备/模拟器
hdc list targets
# 安装 & 启动
hdc install -r flutter/ohos/entry/build/default/outputs/default/entry-default-unsigned.hap
hdc shell aa start -b com.jieneng.yinhe -a EntryAbility
# 日志（Rust 的日志都在 hilog，tag=yinhe）
hdc shell hilog | grep -i yinhe
# 只清屏后看一轮
hdc shell hilog -r && hdc shell aa start -b com.jieneng.yinhe -a EntryAbility
```

## 二、按现象定位

| 现象 | 先看什么日志 | 常见原因 |
| --- | --- | --- |
| 启动即闪退 | `hdc shell hilog \| grep -iE "yinhe\|dlopen\|napi"` | `libyinhe_flutter.so` 没打进 HAP、ABI 不匹配（模拟器 x86_64）、libc++_shared 缺失 |
| 白屏（Flutter UI 有、画布空） | `适配器:`、`初始化 wgpu`、`创建 surface 失败`、`surface 无可用格式` | GLES 后端拿不到 adapter（模拟器只有 GLES 3.0）、surface 创建失败 |
| 画布黑 + `wgpu uncaptured error` | 设备创建/管线报错 | GPU cull 依赖 storage buffer（SSBO），GLES 3.0 模拟器没有 |
| 无声 | `OHAudio ... failed` | 模拟器音频输出被宿主机占用、采样率不支持（后端固定 48k） |
| 设置/最近文件不保存 | `set_files_dir` 之后有没有写盘错误 | `HOME`/filesDir 没传进来（`yinhe_ohos_init` 参数为空） |
| 打开工程/选音色库没反应 | `pick_failed`、Dart 侧异常 | DocumentViewPicker 权限/取消、复制沙箱失败 |
| 打包时 hvigor 报 SDK 版本 | — | `build-profile.json5` 与 SDK 目录名不一致（见 `docs/ohos-build.md`） |

## 三、模拟器的已知限制

官方文档（[模拟器与真机的差异](https://developer.huawei.com/consumer/cn/doc/harmonyos-guides/ide-emulator-specification)，
Windows/macOS 同一份，2026-10 更新）明确写了两条：

> - 模拟器不支持 OpenGL ES 3.1、3.2 接口。
> - DevEco Studio 6.1.0 Beta2 之前不支持 Vulkan；此后支持除
>   `vkGetSwapchainGrallocUsageOHOS`、`vkAcquireImageOHOS`、`vkQueueSignalReleaseImageOHOS`
>   之外的接口（后两个正是外接纹理/原生缓冲相关）。

- 所以模拟器（无论 Windows 还是 macOS 宿主）只给 **OpenGL ES 3.0**
  （`OpenGL ES 3.0 (4.1 Metal - 91.7)`，华为翻译层之上）；
- 因此**画布在模拟器上不可能出图**：音符/自动化渲染依赖顶点阶段 storage buffer，
  缺 SSBO 时管线创建直接失败（日志会打一条「设备不支持 shader storage buffer」）。
  真机（Mali/Adreno，ES 3.2）支持，桌面 Metal/Vulkan 同样支持；
- Vulkan 即便在新版模拟器上可用（我们这台 DevEco 26.0.0 实测枚举到 **0 个设备**），
  wgpu-hal 也没有实现 `VK_OHOS_surface`，要用得自己给 wgpu 打补丁；
- 另一条独立阻塞：模拟器的外部纹理生产端不工作——我们的 EGL 交换链
  `eglSwapBuffers` 成功，但 Bufferqueue 计数保持 `Requested: 0 Flushed: 0`，
  引擎每帧报 `OH_NativeImage_AcquireNativeWindowBuffer() failed ... ret=40601000`，
  所以连纯色清屏都显示不出来（与渲染代码无关）；
- 结论：模拟器只适合验证 UI、文件读写、音频初始化等非画布部分；
  帧率/内存/正确性一律以真机为准；本地想验渲染请跑桌面版（`cargo run`）。

## 四、真机 vs 模拟器

- 模拟器通常免签名；真机需要华为签名（DevEco 自动签名，登录账号 + 连接设备）；
- 真机上 `hdc install` 报 `9568289 / signature verify failed` = 证书/profile 不匹配或 UDID 未注册，
  用 DevEco 的 `Signing Configs` 重新自动签名即可。

## 五、构建产物速查

| 文件 | 说明 |
| --- | --- |
| `flutter/ohos/entry/build/default/outputs/default/entry-default-unsigned.hap` | 未签名 HAP |
| `flutter/ohos/entry/libs/arm64-v8a/libyinhe_flutter.so` | Rust 引擎（每次构建覆盖） |
| `flutter/ohos/entry/libs/arm64-v8a/libyinhe_napi.so` | NAPI 桥（hvigor 编译产出） |
| `flutter/ohos/entry/libs/arm64-v8a/libc++_shared.so` | C++ 运行时（必须随包） |
