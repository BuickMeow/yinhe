# android-activity (yinhe 本地补丁版)

android-activity 0.6.1 的精简副本，通过根 `Cargo.toml` 的 `[patch.crates-io]` 生效。

## 补丁内容

`src/game_activity/mod.rs` 的 `show_soft_input` / `hide_soft_input` 改为空操作：
egui TextEdit 聚焦时 winit 会经它们激活 GameActivity 的 GameTextInput（白色原生输入框），
与应用自己的 IME 桥（隐藏 EditText + JNI，见 crates/yinhe-android/src/ime.rs）冲突，
实测不 patch 则输入完全失效。键盘显示/隐藏、文本回流全部由应用侧 JNI 桥负责。

## 精简说明

只保留 aarch64 相关文件（项目仅构建 arm64-v8a）：删除了
`ffi_arm.rs` / `ffi_i686.rs` / `ffi_x86_64.rs`（ffi.rs 按 target_arch 条件 include）。
如需构建其他架构，从 crates.io 的 android-activity 0.6.1 恢复对应文件并重新打补丁。
