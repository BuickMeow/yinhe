package com.jieneng.yinhe

import android.os.Build
import android.os.Bundle
import android.view.View
import android.view.WindowInsets
import android.view.WindowInsetsController
import com.google.androidgamesdk.GameActivity

/**
 * GameActivity 外壳：加载 libyinhe_android.so 并转发生命周期。
 * 所有 UI 逻辑都在 Rust 侧（yinhe-android crate）。
 */
class MainActivity : GameActivity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        hideSystemBars()
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        // 失去焦点时系统可能恢复导航栏（如通知栏下拉），回焦后重新隐藏。
        if (hasFocus) {
            hideSystemBars()
        }
    }

    /**
     * 沉浸式全屏：隐藏状态栏 + 导航栏（三大金刚键/小白条）。
     * 滑动屏幕边缘可临时唤出，几秒后自动收回（transient bars）。
     */
    private fun hideSystemBars() {
        @Suppress("DEPRECATION")
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            window.setDecorFitsSystemWindows(false)
            val controller = window.insetsController
            controller?.hide(WindowInsets.Type.systemBars())
            controller?.systemBarsBehavior =
                WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        } else {
            window.decorView.systemUiVisibility = (
                View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                    or View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                    or View.SYSTEM_UI_FLAG_FULLSCREEN
                )
        }
    }
}
