# 鸿蒙（OpenHarmony/HarmonyOS）构建说明

本目录由 `flutter create --platforms=ohos .` 生成，随后按本机 SDK 调整过配置。
**当前能产出 debug HAP（未签名）**，但尚未接入 Rust 引擎（音频/渲染后端在做）。

## 一、本机环境（已装好）

| 组件 | 位置 | 说明 |
| --- | --- | --- |
| OpenHarmony SDK 26 (arm64) | `~/harmony/sdk26/26.0.0` | `OHOS_BASE_SDK_HOME=~/harmony/sdk26` |
| OpenHarmony SDK 23（hdc 用） | `~/harmony/sdk` | `OHOS_SDK_HOME=~/harmony/sdk`，提供 `hdc` |
| Flutter-OH 3.47.7 | `~/harmony/flutter-oh` | 与官方 3.47.7 同一 engine revision（`deb287481e`） |
| ohpm 5.0.2 | `~/harmony/oh-command-line-tools/bin` | |
| hvigor 6.26.8 | npm 全局 + `~/harmony/bin/hvigorw` 包装脚本 | DevEco 未安装，用脚本顶替 |

## 二、构建

```sh
export OHOS_BASE_SDK_HOME=~/harmony/sdk26
export OHOS_SDK_HOME=~/harmony/sdk26
export PATH=~/harmony/flutter-oh/bin:~/harmony/oh-command-line-tools/bin:~/harmony/sdk26/26.0.0/toolchains:~/harmony/bin:$PATH
export PUB_HOSTED_URL=https://pub.flutter-io.cn
export FLUTTER_STORAGE_BASE_URL=https://storage.flutter-io.cn

cd flutter/ohos
hvigorw assembleHap -p product=default -p buildMode=debug --no-daemon
# 产物：entry/build/default/outputs/default/entry-default-unsigned.hap
```

> 不走 `flutter build hap`：它强制要求 HarmonyOS（商业）SDK（`HOS_SDK_HOME`），
> 我们只有 OpenHarmony SDK。`hvigorw` 内部的 FlutterTask 会自己跑 `flutter assemble`，
> Dart 侧编译不受影响。

## 三、配置要点（改 SDK 版本时要一起改）

1. `build-profile.json5`：`runtimeOS: OpenHarmony`，`compileSdkVersion/compatibleSdkVersion/targetSdkVersion: "26.0.0"`。
   - API 10–25 要写数字（如 `23`），API ≥ 26 必须写字符串（`"26.0.0"`）。
2. `hvigor/hvigor-config.json5`、`oh-package.json5`：`modelVersion` 必须是 `6.0.0`（API 26 的要求）。
3. `entry/build-profile.json5`：`runtimeOS` 也要是 `OpenHarmony`。
4. `entry/src/main/module.json5`：`deviceTypes` 用 `"default"`（OpenHarmony SDK 没有 `phone` 的 syscap 定义）。
5. SDK 端要求（本机 shim 已做）：
   - SDK 目录名必须是 `26.0.0`（hvigor 按 platformVersion 找目录，不认 `26/`）；
   - 每个组件目录（ets/js/native/toolchains/previewer）需要 `sdk-pkg.json`：
     `{"meta":{"version":"3.0.0"},"data":{"path":"26.0.0,<comp>","platformVersion":"26.0.0","apiVersion":"26","fullApiVersion":"26.0.0",...}}`
   - `oh-uni-package.json` 的 `metaVersion` 需要是 `3.0.0`。
6. `package.json`（已 gitignore）里的 `flutter-hvigor-plugin` 指向 Flutter-OH 安装目录，缺失时执行：
   `npm install`（在 `flutter/ohos` 下）。
7. `GeneratedPluginRegistrant.ets` 是 Flutter 工具生成的，但因为我们直接调 `hvigorw`，
   该文件已提交（没有插件时是空实现）。

## 四、尚未完成 / 已知限制

- **签名**：产物是 `entry-default-unsigned.hap`。装到真机需要华为签名（`hap-sign-tool` +
  调试证书 profile，需要华为开发者账号与设备 UDID 注册）；本机暂时只保证"能编译"。
- **Rust 引擎未接入**：`libyinhe_flutter.so` 还没为 `aarch64-unknown-linux-ohos` 编出来
  （卡在 cpal 无 OHAudio 后端），所以现在的 HAP 只有 Flutter 壳，Dart 里的 FFI 调用会失败。
- 只打 arm64，没有 x86_64 模拟器验证。
