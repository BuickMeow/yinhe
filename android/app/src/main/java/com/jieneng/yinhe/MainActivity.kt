package com.jieneng.yinhe

import android.content.Intent
import android.graphics.Color
import android.os.Build
import android.os.Bundle
import android.provider.OpenableColumns
import android.text.Editable
import android.text.TextWatcher
import android.view.View
import android.view.WindowInsets
import android.view.WindowInsetsController
import android.view.WindowManager
import android.widget.EditText
import android.widget.FrameLayout
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
     * IME 输入目标：decorView 不是文本控件（无 InputConnection），
     * InputMethodManager 会拒绝为其弹键盘，必须有一个真正的 EditText
     * （1x1 透明、无光标）接收输入法文本，经 TextWatcher 回流给 Rust。
     */
    private var imeEdit: EditText? = null

    /** setImeText 同步文本时置位，跳过 TextWatcher 回调（setText 必触发回调，标志必被清除）。 */
    private var syncingImeText = false

    private fun imeEditText(): EditText {
        imeEdit?.let { return it }
        return EditText(this).also { et ->
            imeEdit = et
            et.setBackgroundColor(Color.TRANSPARENT)
            et.setTextColor(Color.TRANSPARENT)
            et.setHighlightColor(Color.TRANSPARENT)
            et.setCursorVisible(false)
            et.isFocusableInTouchMode = true
            // 多行：描述框需要换行；工程名/艺术家里 egui singleline 会过滤 \n。
            et.inputType = android.text.InputType.TYPE_CLASS_TEXT or android.text.InputType.TYPE_TEXT_FLAG_MULTI_LINE
            et.imeOptions = android.view.inputmethod.EditorInfo.IME_ACTION_DONE
            // 输入法 action 键（完成/前往/换行等）一律收键盘；多行时换行键
            // 走 commitText 插入 \n，不触发此回调，所以换行不受影响。
            et.setOnEditorActionListener { _, _, _ ->
                hideIme()
                true
            }
            et.addTextChangedListener(object : TextWatcher {
                override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
                override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {}
                override fun afterTextChanged(s: Editable?) {
                    if (syncingImeText) {
                        syncingImeText = false
                        return
                    }
                    val et = imeEdit ?: return
                    val text = s?.toString() ?: ""
                    // selectionStart 是 UTF-16 偏移，egui 光标按 Unicode 码点计，换算一下。
                    val cursor = text.codePointCount(0, et.selectionStart.coerceIn(0, text.length))
                    onImeText(text, cursor)
                }
            })
            window.addContentView(et, FrameLayout.LayoutParams(1, 1))
        }
    }

    /** Rust 侧（ime 模块）调用：显示软键盘（输入法）。 */
    fun showIme() {
        val imm = getSystemService(
            android.content.Context.INPUT_METHOD_SERVICE
        ) as android.view.inputmethod.InputMethodManager
        val et = imeEditText()
        et.requestFocus()
        // flags=0 为显式请求（SHOW_IMPLICIT 会被 Android 12+ 的 IME 政策忽略）。
        val ok = imm.showSoftInput(et, 0)
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

    /** Rust 侧（ime 模块）调用：egui 焦点切换时同步 EditText 文本，
     *  防止残留上一个输入框的内容（setText 触发 TextWatcher，用标志防回环）。 */
    fun setImeText(text: String) {
        val et = imeEdit ?: return
        syncingImeText = true
        et.setText(text)
        et.setSelection(et.text.length)
    }

    /** Rust 侧（ime 模块）调用：egui 光标变化时同步 EditText 选区（UTF-16 偏移）。
     *  setSelection 只移动光标，不触发 TextWatcher，无需防回环。 */
    fun setImeSelection(pos: Int) {
        val et = imeEdit ?: return
        val max = et.text.toString().codePointCount(0, et.text.length)
        val utf16 = et.text.toString().offsetByCodePoints(0, pos.coerceIn(0, max))
        et.setSelection(utf16)
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

    /** Rust 侧（ime 模块）的回调：输入法文本变化（全量文本 + 光标按码点计）。 */
    private external fun onImeText(text: String, cursor: Int)

    /** Rust 侧（insets 模块）的 JNI 回调，写入全局安全区状态（px）。 */
    private external fun onSystemInsetsChanged(left: Int, top: Int, right: Int, bottom: Int)
}
