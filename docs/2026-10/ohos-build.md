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

产物大小（release，单 arm64）：约 **46MB**；debug 约 170MB（含 55MB Dart kernel + x86_64 引擎）。
| 组成 | 大小 |
| --- | --- |
| libflutter.so（OHOS 引擎 arm64） | 18.2MB |
| libyinhe_flutter.so（Rust 引擎） | 16.7MB |
| libapp.so（Dart AOT） | 6.8MB |
| MaterialIcons 字体 | 1.6MB |
| libc++_shared.so | 1.2MB |

> 原生库在 HAP 里是 **Stored（不压缩）**，所以包体≈各 .so 之和。

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
5. hvigor 6.26.8 用了 Node 16 已移除的 `fs.rmdirSync(recursive)`（release 的
   `BuildNativeWithNinja` 会踩到）：本机 `~/harmony/bin/hvigorw` 通过
   `node --require ~/harmony/hvigorw-polyfill.js` 打补丁，否则 release 构建必失败。
6. SDK 端（本机 shim 已做）：
   - SDK 目录名必须是 `26.0.0`（hvigor 按 platformVersion 找，不认 `26/`）；
   - 每个组件目录（ets/js/native/toolchains/previewer）需要 `sdk-pkg.json`：
     `{"meta":{"version":"3.0.0"},"data":{"path":"26.0.0,<comp>","platformVersion":"26.0.0","apiVersion":"26","fullApiVersion":"26.0.0",...}}`
   - `oh-uni-package.json` 的 `metaVersion` 需要是 `3.0.0`。
6. `GeneratedPluginRegistrant.ets` 是 Flutter 工具生成的（无插件时为空实现），已提交。

## 四·五、签名

| 场景 | 需要的证书 | 产物 |
| --- | --- | --- |
| 模拟器 / 本地真机调试 | **调试证书 + 调试 Profile**（绑定设备 UDID） | `entry-default-signed.hap` |
| AGC 云调试 / 云测试 / 上架 | **发布证书 + 发布 Profile**（DevEco 自动签名生成的是调试证书，云调试会拒"未使用证书签名"） | 同上，`type=release` |

生成发布证书的流程：
1. DevEco `Build → Generate Key and CSR` 生成 `.p12` + `.csr`；
2. AGC → `证书、APP ID 和 Profile` → 新增**发布证书**（上传 csr）→ 下载 `.cer`；
3. AGC → 新增**发布 Profile**（绑定应用 + 发布证书）→ 下载 `.p7b`；
4. DevEco `Project Structure → Signing Configs` 手动填 p12/cer/p7b；
5. `cargo xtask ohos --release`。

验证签名（确认是 release）：

```sh
java -jar <sdk>/toolchains/lib/hap-sign-tool.jar verify-app \
  -inFile entry-default-signed.hap -outCertChain /tmp/x.cer -outProfile /tmp/x.p7b
```

> 签名相关的 `build-profile.json5` 本地改动已 `git update-index --skip-worktree`，不会提交证书路径/密码。

## 五、没有真机时怎么跑（模拟器）

1. 下载安装 **DevEco Studio（macOS ARM 版）**；
2. 用 DevEco 自带的 Emulator CLI（不用打开 IDE）：

```sh
D=/Applications/DevEco-Studio.app/Contents/tools/emulator
$D/Emulator -license accept
$D/Emulator -install -deviceType phone -osVersion "HarmonyOS 7.0.0(26.0.0)"   # 约 2.3GB
$D/Emulator -create yinhe_phone -deviceType phone -osVersion "HarmonyOS 7.0.0(26.0.0)"
$D/Emulator -start yinhe_phone          # 加 -noWindow 可无界面启动
$D/Emulator -list -details             # 名字只允许字母/数字/空格/下划线/加号
```

3. 本机执行：

```sh
cargo xtask ohos --install
# 等价于：hdc install -r <hap> && hdc shell aa start -b com.jieneng.yinhe -a EntryAbility
```

4. 看日志：`hdc shell hilog | grep yinhe`（Rust 的日志都走 hilog，tag `yinhe`）。

说明：
- 模拟器**通常不需要签名**即可安装（官方文档：模拟器不需要签名，除非用到 ODID / Push）；
  如果 `hdc install` 报签名错误，就用 DevEco 的自动签名（登录后勾选
  `Project Structure > Signing Configs > Automatically generate signature`），
  它会把 `signingConfigs` 写进 `build-profile.json5`，我们的 hvigor 构建会自动带上。
- 模拟器只支持 **OpenGL ES 2.0/3.0**，不支持 Vulkan —— 正好对应我们的 GLES 路线。

## 六、尚未完成 / 已知限制

- **签名**：产物是 unsigned。装真机需要华为签名（调试证书 profile + 设备 UDID 注册，
  需要华为开发者账号）。
- **真机未验证**：图形（GLES）、音频（OHAudio）、触摸、横竖屏都没在设备上跑过。
- **日志**：OHOS 上还没有 hilog 后端（`log` 宏目前丢弃），排查问题前需要补。
- **文件选择**：已接 DocumentViewPicker（打开工程/音色库/另存/导出），选中文件会复制进沙箱；
  真机上需要验证「覆盖保存」的 uri 复用与权限。
- **性能**：OHOS 走 GLES 后端（Vulkan 需要给 wgpu-hal 补 `VK_OHOS_surface`）；
  黑乐谱的间接绘制策略未在 Mali GPU 上调过。
