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

- 只支持 **OpenGL ES 2.0/3.0**，不支持 Vulkan，也不支持 GLES 3.1 的 SSBO/计算着色器；
- 因此：**wgpu 的 GPU cull（compute）在模拟器上大概率跑不起来**，真机（Mali/Adreno，ES 3.2）才准；
- 帧率/内存数据在模拟器上没有参考价值，只验证"能起来、能交互、文件能读写"。

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
