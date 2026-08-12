package com.jieneng.yinhe

import com.google.androidgamesdk.GameActivity

/**
 * GameActivity 外壳：加载 libyinhe_android.so 并转发生命周期。
 * 所有 UI 逻辑都在 Rust 侧（yinhe-android crate）。
 */
class MainActivity : GameActivity()
