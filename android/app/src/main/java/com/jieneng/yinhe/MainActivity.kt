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
import com.google.androidgamesdk.gametextinput.State
import java.io.File

/**
 * GameActivity 外壳：加载 libyinhe_android.so 并转发生命周期。
 * 所有 UI 逻辑都在 Rust 侧（yinhe-android crate）。
 */
class MainActivity : YinheActivity() {

    companion object {
        private const val REQ_OPEN_FILE = 1001
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        allowDrawingIntoCutout()
        // IME 配置：多行输入（描述框换行）+ 完成键 + 关闭提取模式（白色放大编辑区）。
        setImeEditorInfoFields(
            android.text.InputType.TYPE_CLASS_TEXT or android.text.InputType.TYPE_TEXT_FLAG_MULTI_LINE,
            android.view.inputmethod.EditorInfo.IME_ACTION_DONE,
            android.view.inputmethod.EditorInfo.IME_FLAG_NO_EXTRACT_UI
        )
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

    override fun onConfigurationChanged(newConfig: android.content.res.Configuration) {
        super.onConfigurationChanged(newConfig)
        // 旋转（configChanges 不重建 Activity）时 WindowInsets 不会重新派发，
        // 必须手动重读；post 等旋转布局完成后再读（否则还是旧值）。
        window.decorView.post {
            pushCurrentInsets()
            android.util.Log.i("yinhe", "onConfigurationChanged: 旋转后重读 insets")
        }
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

    /**
     * IME：输入目标为 GameActivity 的 mSurfaceView（YinheInputSurfaceView，
     * 始终返回 GameTextInput InputConnection）。键盘显示/隐藏经
     * InputMethodManager 显式调用（SHOW_IMPLICIT 会被 Android 12+ 忽略）；
     * 输入法文本由本类覆写 stateChanged 回调（每帧全量文本+选区）经 JNI
     * 回流 Rust，绕开 winit 0.30 安卓后端不转发输入法事件的断链。
     */

    /**
     * GameTextInput 文本状态变化回调（输入目标绑定后每帧全量文本+选区）。
     * 不调 super：native 侧 TextEvent 无消费者（winit 丢弃），只走 JNI 回流。
     */
    override fun stateChanged(state: State, dismissed: Boolean) {
        // selectionStart 是 UTF-16 偏移，egui 光标按 Unicode 码点计，换算一下。
        val cursor = state.text.codePointCount(
            0, state.selectionStart.coerceIn(0, state.text.length)
        )
        onImeText(state.text, cursor)
    }

    /** 输入法 action 键（完成）触发：收键盘；多行换行键走 commitText 不受影响。 */
    override fun onEditorAction(action: Int) {
        hideIme()
    }

    /** Rust 侧（ime 模块）调用：显示软键盘（输入法）。 */
    fun showIme() {
        val imm = getSystemService(
            android.content.Context.INPUT_METHOD_SERVICE
        ) as android.view.inputmethod.InputMethodManager
        val surface = mSurfaceView
        if (surface == null) {
            android.util.Log.w("yinhe", "showIme: mSurfaceView 未初始化")
            return
        }
        // SurfaceView 默认不可聚焦，IMM 需要焦点 view 绑定 InputConnection；
        // 显式开启（GameTextInput 激活时也会设置，这里提前保证 requestFocus 有效）。
        surface.isFocusableInTouchMode = true
        surface.isFocusable = true
        surface.requestFocus()
        // flags=0 为显式请求（SHOW_IMPLICIT 会被 Android 12+ 的 IME 政策忽略）。
        val ok = imm.showSoftInput(surface, 0)
        android.util.Log.i("yinhe", "showIme: result=$ok")
    }

    /** Rust 侧（ime 模块）调用：隐藏软键盘。 */
    fun hideIme() {
        val imm = getSystemService(
            android.content.Context.INPUT_METHOD_SERVICE
        ) as android.view.inputmethod.InputMethodManager
        imm.hideSoftInputFromWindow(window.decorView.windowToken, 0)
        android.util.Log.i("yinhe", "hideIme")
    }

    /** Rust 侧（ime 模块）调用：焦点切换时同步 InputConnection 文本（防残留）。
     *  setState 内部不触发 stateChanged（只更新 mEditable + 选区 + restartInput），
     *  无回环。 */
    fun setImeText(text: String) {
        val conn = imeConnection() ?: return
        conn.setState(State(text, text.length, text.length, -1, -1))
    }

    /** Rust 侧（ime 模块）调用：egui 光标变化时同步 InputConnection 选区（UTF-16 偏移）。
     *  setSelection 只移动光标，不触发 stateChanged，无回环。 */
    fun setImeSelection(pos: Int) {
        val conn = imeConnection() ?: return
        val text = conn.getEditable()?.toString() ?: return
        val max = text.codePointCount(0, text.length)
        val utf16 = text.offsetByCodePoints(0, pos.coerceIn(0, max))
        conn.setSelection(utf16, utf16)
    }

    private fun imeConnection(): com.google.androidgamesdk.gametextinput.InputConnection? {
        return (mSurfaceView as? YinheInputSurfaceView)?.getConnection()
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

    /** Rust 侧（ime 模块）的回调：输入法文本变化（全量文本 + 光标按码点计）。 */
    private external fun onImeText(text: String, cursor: Int)
}
