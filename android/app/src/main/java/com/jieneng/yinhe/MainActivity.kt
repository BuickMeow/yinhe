package com.jieneng.yinhe

import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.provider.OpenableColumns
import android.view.View
import android.view.WindowInsets
import android.view.WindowInsetsController
import android.view.WindowManager
import androidx.core.graphics.Insets
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import com.google.androidgamesdk.GameActivity
import java.io.File

/**
 * GameActivity 外壳：加载 libyinhe_android.so 并转发生命周期。
 * 所有 UI 逻辑都在 Rust 侧（yinhe-android crate）。
 */
class MainActivity : GameActivity() {

    companion object {
        private const val REQ_OPEN_FILE = 1001
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        allowDrawingIntoCutout()
        // WindowInsets 变化（挖孔/系统栏/圆角）时推送给 Rust 侧做安全区布局。
        ViewCompat.setOnApplyWindowInsetsListener(window.decorView) { _, insets ->
            pushInsets(insets)
            insets
        }
        hideSystemBars()
    }

    override fun onResume() {
        super.onResume()
        // 兜底：直接读当前 insets 推一次（listener 首次派发时机不定）。
        pushCurrentInsets()
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        // 失去焦点时系统可能恢复导航栏（如通知栏下拉），回焦后重新隐藏。
        if (hasFocus) {
            hideSystemBars()
            pushCurrentInsets()
        }
    }

    /**
     * 允许内容绘制进挖孔/刘海区域。横屏时居中挖孔可能落在长边（顶部/底部
     * 中间），SHORT_EDGES 管不到，因此 API 30+ 用 ALWAYS；
     * API 28-29 只有 SHORT_EDGES；API 26-27 无 cutout 概念，系统自行兜底。
     */
    private fun allowDrawingIntoCutout() {
        val mode = when {
            Build.VERSION.SDK_INT >= 30 ->
                WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS
            Build.VERSION.SDK_INT >= 28 ->
                WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES
            else -> return
        }
        window.attributes = window.attributes.apply { layoutInDisplayCutoutMode = mode }
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

    /** 安全区 = 系统栏 + 挖孔（物理像素 px），UI 需要避开的最小区域。 */
    private fun safeInsets(insets: WindowInsetsCompat): Insets {
        return insets.getInsets(
            WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
        )
    }

    private fun pushInsets(insets: WindowInsetsCompat) {
        val s = safeInsets(insets)
        onSystemInsetsChanged(s.left, s.top, s.right, s.bottom)
    }

    /** 兜底：读取当前窗口 insets 推一次。 */
    private fun pushCurrentInsets() {
        val root = window.decorView.rootWindowInsets ?: return
        pushInsets(WindowInsetsCompat.toWindowInsetsCompat(root, window.decorView))
    }

    /** Rust 侧（file_picker 模块）在菜单"本地打开"时通过 JNI 调用。 */
    fun openFilePicker() {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "*/*"
            putExtra(Intent.EXTRA_MIME_TYPES, arrayOf("audio/midi", "audio/x-midi", "audio/mid"))
        }
        startActivityForResult(intent, REQ_OPEN_FILE)
    }

    @Deprecated("Deprecated in Java")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != REQ_OPEN_FILE || resultCode != RESULT_OK) {
            return
        }
        val uri = data?.data ?: return
        // SAF uri 只在授权期内可读，复制到私有目录后 Rust 侧拿稳定文件路径。
        val name = displayName(uri) ?: "picked.mid"
        val dest = File(filesDir, name)
        try {
            contentResolver.openInputStream(uri)?.use { input ->
                dest.outputStream().use { output -> input.copyTo(output) }
            }
            onFilePicked(dest.absolutePath)
        } catch (e: Exception) {
            android.util.Log.e("yinhe", "复制所选文件失败: $e")
        }
    }

    private fun displayName(uri: android.net.Uri): String? {
        return contentResolver.query(uri, null, null, null, null)?.use { c ->
            val idx = c.getColumnIndex(OpenableColumns.DISPLAY_NAME)
            if (idx >= 0) c.getString(idx) else null
        }
    }

    /** Rust 侧（file_picker 模块）的回调：文件已复制到私有目录。 */
    private external fun onFilePicked(path: String)

    /** Rust 侧（insets 模块）的 JNI 回调，写入全局安全区状态（px）。 */
    private external fun onSystemInsetsChanged(left: Int, top: Int, right: Int, bottom: Int)
}
