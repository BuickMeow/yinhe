package com.jieneng.yinhe

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import android.provider.Settings
import android.util.Log
import android.view.InputDevice
import android.view.MotionEvent
import android.view.Surface
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import io.flutter.view.TextureRegistry
import java.io.File
import android.widget.Toast

private const val TAG = "yinhe-bridge"

/**
 * 纹理桥宿主：把 SurfaceProducer 的 Surface 交给 Rust(wgpu) 渲染。
 *
 * - 引擎通过 BufferQueue 消费 Rust 生产的最新帧，画布动画无需 Dart 侧 setState。
 * - 回调约定（Flutter 3.27+）：onSurfaceAvailable 后调用 getSurface() 取当前
 *   surface（不要缓存：纹理 resize/换格式时 surface 会换）；onSurfaceCleanup
 *   时停止渲染。
 * - 尺寸分两套：SurfaceProducer 用物理像素（交换链），nativeSetViewport 传
 *   逻辑像素（Rust 的视图/手势坐标系）。
 * - Dart 只用 MethodChannel 拿 textureId / 上报尺寸；触摸走 dart:ffi 直连。
 */
class MainActivity : FlutterActivity() {
    companion object {
        /** SAF 文件选择请求码。 */
        private const val REQ_OPEN_FILE = 1001

        /** SAF 音色库选择请求码。 */
        private const val REQ_OPEN_SF = 1002

        /** 运行时存储权限（Android 10 及以下）。 */
        private const val REQ_PERM_STORAGE = 1003

        /** SAF 保存工程请求码。 */
        private const val REQ_SAVE_FILE = 1003
        private const val REQ_SAVE_EXPORT = 1004

        /** SAF 目录选择（SFZ 音色库连同采样一起导入）。 */
        /** 导出工程的临时文件（Rust 写入，Kotlin 复制到用户选择的位置）。 */
        private const val EXPORT_TMP = "export.yin"

        /** 加载工程：MIDI / .yin / 压缩包。 */
        private val PROJECT_EXTS = setOf("mid", "midi", "yin")

        /** 裸音色库文件只收 SF2；SFZ 的采样只能随压缩包整包导入。 */
        private val SOUNDFONT_EXTS = setOf("sf2")

        /** 压缩包扩展名（与 Rust/Dart 同一份列表）。 */
        private val ARCHIVE_EXTS = setOf(
            "zip", "7z", "rar", "lzh", "lha", "tar",
            "gz", "xz", "tgz", "txz", "tbz", "bz2",
        )

        init {
            // 压缩包支持（unarc-rs）带 C++ 代码：先加载 NDK 的 C++ 运行时，
            // 否则 libyinhe_flutter.so 里的 std::length_error 等符号无法解析。
            System.loadLibrary("c++_shared")
            System.loadLibrary("yinhe_flutter")
        }
    }

    /** 延迟到 Dart 请求纹理时才创建 SurfaceProducer（见 SurfaceSlot.ensure）。 */
    private var engine: FlutterEngine? = null

    /** 主画布 surface：懒创建、尺寸上报与 Rust 回调。 */
    private val canvasSurface = SurfaceSlot(
        label = "canvas",
        setSurface = { nativeSetSurface(it) },
        clearSurface = { nativeClearSurface() },
        onSize = { w, h -> nativeSetSize(w, h) },
    )

    /** AM 面板独立 surface（力度条/自动化曲线）。 */
    private val automationSurface = SurfaceSlot(
        label = "AM",
        setSurface = { nativeSetAutomationSurface(it) },
        clearSurface = { nativeClearAutomationSurface() },
        onSize = { w, h -> nativeSetAutomationSize(w, h) },
    )

    /** 原生层已拦截的鼠标右键拖拽（避免 Android 把右键当 BACK）。 */
    private var rightPanActive = false

    /** 当前工程的 SAF uri（“保存工程”直接覆盖；另存为/打开时更新）。 */
    private var lastProjectUri: android.net.Uri? = null

    /** 主页导出：当前等待保存的临时文件扩展名（wav/mid）。 */
    private var pendingExportExt: String = ""

    /** 持久化的工程 uri（按文件名），供最近列表打开后直接覆盖保存。 */
    private val prefs by lazy { getSharedPreferences("yinhe", MODE_PRIVATE) }

    private fun rememberUri(name: String, uri: android.net.Uri) {
        lastProjectUri = uri
        prefs.edit().putString("uri:$name", uri.toString()).apply()
    }

    private fun restoreUri(name: String) {
        val saved = prefs.getString("uri:$name", null) ?: return
        lastProjectUri = runCatching { android.net.Uri.parse(saved) }.getOrNull()
    }

    /** 与 Dart 的桥（双向：Dart 请求 + 文件选择结果回传）。 */
    private var methodChannel: MethodChannel? = null

    private external fun nativeSetSurface(surface: Surface)

    private external fun nativeClearSurface()

    private external fun nativeSetAutomationSurface(surface: Surface)

    private external fun nativeClearAutomationSurface()

    private external fun nativeSetAutomationSize(width: Int, height: Int)

    private external fun nativeSetSize(width: Int, height: Int)

    private external fun nativeSetViewport(width: Float, height: Float)

    /** cpal/AAudio 需要的 NDK context（JavaVM + Application Context）。 */
    private external fun nativeInitContext(context: android.content.Context, filesDir: String)

    /** Rust 回调：所选音色库（SF2）已复制到私有目录。 */
    private external fun onSoundFontPicked(path: String)

    override fun onCreate(savedInstanceState: android.os.Bundle?) {
        super.onCreate(savedInstanceState)
        ensureStoragePermission()
    }

    /**
     * Android 10 及以下：要读外部真实路径（SFZ 及其采样）就需要这个运行时权限。
     * 鸿蒙 4 的 Android 兼容层可能直接拒绝 —— 拒绝后所有文件都走"复制进私有目录"，
     * 功能不受影响，只有裸 SFZ（采样在用户目录里）不能用，需要压缩包导入。
     */
    private fun ensureStoragePermission() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            return
        }
        if (checkSelfPermission(Manifest.permission.READ_EXTERNAL_STORAGE) ==
            PackageManager.PERMISSION_GRANTED
        ) {
            return
        }
        requestPermissions(arrayOf(Manifest.permission.READ_EXTERNAL_STORAGE), REQ_PERM_STORAGE)
    }

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)

        // cpal/AAudio 需要 NDK context（JavaVM + Application Context）；
        // filesDir 供 Rust 持久化最近文件。
        nativeInitContext(applicationContext, filesDir.absolutePath)
        engine = flutterEngine

        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "yinhe/surface")
            .also { methodChannel = it }
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "getTextureId" -> {
                        val p = canvasSurface.ensure()
                        if (p == null) {
                            result.error("no_engine", "FlutterEngine 未就绪", null)
                        } else {
                            result.success(p.id())
                        }
                    }
                    "getAutomationTextureId" -> {
                        val p = automationSurface.ensure()
                        if (p == null) {
                            result.error("no_engine", "FlutterEngine 未就绪", null)
                        } else {
                            result.success(p.id())
                        }
                    }
                    "setAutomationSize" -> {
                        val w = call.argument<Int>("w")
                        val h = call.argument<Int>("h")
                        if (w == null || h == null) {
                            result.error("bad_args", "缺少 AM w/h", null)
                        } else if (!automationSurface.resize(w, h)) {
                            result.error("no_engine", "FlutterEngine 未就绪", null)
                        } else {
                            result.success(null)
                        }
                    }
                    "ensureAutomationSurface" -> {
                        automationSurface.ensure()
                        result.success(null)
                    }
                    "setSize" -> {
                        val w = call.argument<Int>("w")
                        val h = call.argument<Int>("h")
                        val lw = call.argument<Double>("lw")
                        val lh = call.argument<Double>("lh")
                        if (w == null || h == null || lw == null || lh == null) {
                            result.error("bad_args", "缺少 w/h/lw/lh", null)
                        } else if (!canvasSurface.resize(w, h)) {
                            result.error("no_engine", "FlutterEngine 未就绪", null)
                        } else {
                            Log.i(TAG, "setSize $w x $h (logical $lw x $lh)")
                            // 逻辑视口 = Rust 的视图/手势坐标系。
                            nativeSetViewport(lw.toFloat(), lh.toFloat())
                            result.success(null)
                        }
                    }
                    "openFilePicker" -> {
                        openFilePicker()
                        result.success(null)
                    }
                    "openSoundFontPicker" -> {
                        openSoundFontPicker()
                        result.success(null)
                    }
                    "ensureSurface" -> {
                        canvasSurface.ensure()
                        result.success(null)
                    }
                    "exportTempPath" -> {
                        val ext = call.argument<String>("ext")
                        val file = if (ext.isNullOrBlank()) {
                            File(filesDir, EXPORT_TMP)
                        } else {
                            File(filesDir, "export_tmp.$ext")
                        }
                        result.success(file.absolutePath)
                    }
                    "openSavePicker" -> {
                        val name = call.argument<String>("name") ?: "Untitled"
                        openSavePicker(name)
                        result.success(null)
                    }
                    "rememberProject" -> {
                        val name = call.argument<String>("name")
                        if (name != null) restoreUri(name)
                        result.success(null)
                    }
                    "saveProject" -> {
                        val saveAs = call.argument<Boolean>("saveAs") ?: false
                        val name = call.argument<String>("name") ?: "Untitled"
                        val uri = lastProjectUri
                        if (!saveAs && uri != null) {
                            // 覆盖当前工程文件；失败则退回另存为。
                            if (copyExportTo(uri)) {
                                methodChannel?.invokeMethod("onProjectSaved", "ok")
                            } else {
                                lastProjectUri = null
                                openSavePicker(name)
                            }
                        } else {
                            openSavePicker(name)
                        }
                        result.success(null)
                    }
                    "saveExport" -> {
                        val name = call.argument<String>("name") ?: "export"
                        val mime = call.argument<String>("mime")
                            ?: "application/octet-stream"
                        val ext = call.argument<String>("ext") ?: ""
                        pendingExportExt = ext
                        openExportPicker(name, mime)
                        result.success(null)
                    }
                    else -> result.notImplemented()
                }
            }
    }

    // ── 鼠标右键平移 ──────────────────────────────────────────────
    // 左键选框的同时用右键移动：原生层直接识别 BUTTON_SECONDARY 并消费，
    // 通过 MethodChannel 交给 Dart 走手形平移（同时避免触发系统 BACK）。

    override fun dispatchTouchEvent(ev: MotionEvent): Boolean {
        if (handleRightPan(ev)) {
            return true
        }
        return super.dispatchTouchEvent(ev)
    }

    override fun dispatchGenericMotionEvent(ev: MotionEvent): Boolean {
        if (handleRightPan(ev)) {
            return true
        }
        return super.dispatchGenericMotionEvent(ev)
    }

    private fun handleRightPan(ev: MotionEvent): Boolean {
        if (ev.source and InputDevice.SOURCE_MOUSE != InputDevice.SOURCE_MOUSE) {
            return false
        }
        val secondary =
            (ev.buttonState and MotionEvent.BUTTON_SECONDARY) != 0 ||
                ev.actionButton == MotionEvent.BUTTON_SECONDARY
        val action = ev.actionMasked
        if (action == MotionEvent.ACTION_DOWN) {
            Log.i(TAG, "mouse down buttons=${ev.buttonState} actionButton=${ev.actionButton}")
            if (!secondary) {
                return false
            }
            rightPanActive = true
            sendRightPan(0, ev)
            return true
        }
        if (!rightPanActive) {
            return false
        }
        if (action == MotionEvent.ACTION_MOVE) {
            sendRightPan(1, ev)
            return true
        }
        if (action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_CANCEL) {
            rightPanActive = false
            sendRightPan(2, ev)
            return true
        }
        return false
    }

    /** 原生像素坐标交给 Dart，由 Dart 换算成画布逻辑坐标。 */
    private fun sendRightPan(kind: Int, ev: MotionEvent) {
        methodChannel?.invokeMethod(
            "rightPan",
            mapOf("kind" to kind, "x" to ev.x.toDouble(), "y" to ev.y.toDouble()),
        )
    }

    /**
     * 一个 wgpu 生产端 surface 槽位：惰性创建 SurfaceProducer，处理新旧回调，
     * 且每代 surface 只向 Rust 发送一次（同一窗口重复设置会触发 IN_USE）。
     *
     * 不能在 configureFlutterEngine 里创建：release 包此时 FlutterView 尚未挂载，
     * getSurface() 会走 FlutterJNI.registerImageTexture 并触发引擎断言闪退。
     * Dart 初始化后通过 getTextureId/ensureSurface 调用 [ensure]，此时视图已就绪。
     */
    private inner class SurfaceSlot(
        private val label: String,
        private val setSurface: (Surface) -> Unit,
        private val clearSurface: () -> Unit,
        private val onSize: (Int, Int) -> Unit,
    ) {
        private var producer: TextureRegistry.SurfaceProducer? = null
        private var sent = false

        fun ensure(): TextureRegistry.SurfaceProducer? {
            producer?.let { return it }
            val flutterEngine = engine ?: return null
            val p = flutterEngine.renderer.createSurfaceProducer()
            p.setCallback(
                object : TextureRegistry.SurfaceProducer.Callback {
                    // 引擎（3.47）调用旧入口 onSurfaceCreated/onSurfaceDestroyed；
                    // 新入口 onSurfaceAvailable/onSurfaceCleanup 是给未来版本的。
                    // 两个都接：引擎只会调用其中一个，经同一 helper 幂等处理。
                    @Suppress("OVERRIDE_DEPRECATION")
                    override fun onSurfaceCreated() {
                        Log.i(TAG, "$label onSurfaceCreated")
                        ready(p)
                    }

                    override fun onSurfaceAvailable() {
                        Log.i(TAG, "$label onSurfaceAvailable")
                        ready(p)
                    }

                    @Suppress("OVERRIDE_DEPRECATION")
                    override fun onSurfaceDestroyed() {
                        Log.i(TAG, "$label onSurfaceDestroyed")
                        gone()
                    }

                    override fun onSurfaceCleanup() {
                        Log.i(TAG, "$label onSurfaceCleanup")
                        gone()
                    }
                },
            )
            producer = p
            // 若 surface 已惰性创建则立即发送（回调在部分引擎版本不触发）。
            ready(p)
            return p
        }

        /**
         * resize 只重配置交换链：对同一 ANativeWindow 重建 surface 会触发
         * VK_ERROR_NATIVE_WINDOW_IN_USE_KHR。无引擎时返回 false。
         */
        fun resize(width: Int, height: Int): Boolean {
            val p = ensure() ?: return false
            p.setSize(width, height)
            onSize(width, height)
            ready(p)
            return true
        }

        fun release() {
            producer?.release()
            producer = null
            sent = false
        }

        private fun ready(p: TextureRegistry.SurfaceProducer) {
            if (sent) {
                return
            }
            val s = p.getSurface() ?: return
            sent = true
            setSurface(s)
        }

        private fun gone() {
            sent = false
            clearSurface()
        }
    }

    /** Dart 侧「打开文件」：MIDI、`.yin` 工程与压缩包。 */
    private fun openFilePicker() {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "*/*"
            addFlags(
                Intent.FLAG_GRANT_READ_URI_PERMISSION or
                    Intent.FLAG_GRANT_WRITE_URI_PERMISSION or
                    Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION,
            )
            // 不加 EXTRA_MIME_TYPES：部分机型（如 MIUI）会把 .mid 归到
            // 其它 MIME 而置灰；这里放开为全部文件，回到 onActivityResult
            // 再按扩展名校验（只收 mid/midi/yin/压缩包）。
        }
        startActivityForResult(intent, REQ_OPEN_FILE)
    }

    /**
     * Dart 侧「选择音色库」：只接受裸 SF2 或压缩包。
     *
     * SAF 没有统一的 SoundFont MIME，仍放开选择，回来按扩展名校验；
     * 裸 SFZ 单独拷过来找不到采样，因此不允许，SFZ 只能走压缩包整包导入。
     */
    private fun openSoundFontPicker() {
        // SFZ 需要同目录的采样文件：没有「所有文件访问」时直接跳到系统授权页。
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R &&
            !Environment.isExternalStorageManager()
        ) {
            Toast.makeText(this, "请开启「所有文件访问」后重试", Toast.LENGTH_LONG).show()
            val intent = Intent(
                Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
                Uri.parse("package:$packageName"),
            )
            runCatching { startActivity(intent) }.onFailure {
                runCatching {
                    startActivity(Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION))
                }
            }
            return
        }
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "*/*"
        }
        startActivityForResult(intent, REQ_OPEN_SF)
    }

    private fun hasAllFilesAccess(): Boolean =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            Environment.isExternalStorageManager()
        } else {
            // Android 10 及以下没有"所有文件访问"开关，等价能力是 READ_EXTERNAL_STORAGE；
            // 旧实现直接返回 true，结果把外部真实路径交给 Rust，鸿蒙上 mmap 就 EACCES。
            checkSelfPermission(Manifest.permission.READ_EXTERNAL_STORAGE) ==
                PackageManager.PERMISSION_GRANTED
        }

    /** 文件扩展名（小写、不含点；无扩展名返回空串）。 */
    private fun extensionOf(name: String): String {
        val dot = name.lastIndexOf('.')
        return if (dot >= 0 && dot + 1 < name.length) {
            name.substring(dot + 1).lowercase()
        } else {
            ""
        }
    }

    private fun isArchiveName(name: String): Boolean =
        extensionOf(name) in ARCHIVE_EXTS

    /** 工程选择：MIDI / .yin / 压缩包。 */
    private fun isProjectName(name: String): Boolean =
        extensionOf(name) in PROJECT_EXTS || isArchiveName(name)

    /** 音色库选择：裸 .sf2 或压缩包（包内可含 SF2/SFZ 及采样）。 */
    private fun isSoundfontName(name: String): Boolean {
        val ext = extensionOf(name)
        if (ext in SOUNDFONT_EXTS) {
            return true
        }
        // 裸 SFZ 只有在拿到「所有文件访问」时才允许（采样在用户目录里原地读取）。
        if (ext == "sfz" && hasAllFilesAccess()) {
            return true
        }
        return isArchiveName(name)
    }

    /** 与 Rust/Dart 保持一致的压缩包扩展名。 */
    private fun isArchivePath(path: String): Boolean =
        isArchiveName(File(path).name)

    /** Dart 侧「保存工程」：先由 Rust 写好临时文件，这里复制到用户选择的位置。 */
    private fun openSavePicker(name: String) {
        val safe = name.replace(Regex("[/\\\\:*?\"<>|]"), "_").trim()
        val base = if (safe.isEmpty()) "Untitled" else safe
        val title = if (base.endsWith(".yin", ignoreCase = true)) base else "$base.yin"
        val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "application/octet-stream"
            putExtra(Intent.EXTRA_TITLE, title)
        }
        startActivityForResult(intent, REQ_SAVE_FILE)
    }

    /** 主页导出（WAV/MIDI）：选择保存位置。 */
    private fun openExportPicker(name: String, mime: String) {
        val safe = name.replace(Regex("[/\\:*?\"<>|]"), "_").trim()
        val base = if (safe.isEmpty()) "export" else safe
        val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = mime
            putExtra(Intent.EXTRA_TITLE, base)
        }
        startActivityForResult(intent, REQ_SAVE_EXPORT)
    }

    /** 把指定扩展名的导出临时文件复制到用户选择的 uri。 */
    private fun copyTempTo(uri: android.net.Uri?, ext: String): Boolean {
        if (uri == null) {
            return false
        }
        val fileName = if (ext.isBlank()) EXPORT_TMP else "export_tmp.$ext"
        val src = File(filesDir, fileName)
        if (!src.exists()) {
            Log.e(TAG, "导出临时文件不存在: ${src.absolutePath}")
            return false
        }
        return try {
            val output = contentResolver.openOutputStream(uri)
            if (output == null) {
                Log.e(TAG, "打开目标位置失败: $uri")
                return false
            }
            output.use { dst -> src.inputStream().use { it.copyTo(dst) } }
            true
        } catch (e: Exception) {
            Log.e(TAG, "复制导出文件失败: $e")
            false
        }
    }

    /** 把导出的临时工程复制到用户选择的 uri。 */
    private fun copyExportTo(uri: android.net.Uri?): Boolean {
        if (uri == null) {
            return false
        }
        val src = File(filesDir, EXPORT_TMP)
        if (!src.exists()) {
            Log.e(TAG, "导出临时文件不存在: ${src.absolutePath}")
            return false
        }
        return try {
            val output = contentResolver.openOutputStream(uri)
            if (output == null) {
                Log.e(TAG, "打开目标位置失败: $uri")
                return false
            }
            output.use { dst -> src.inputStream().use { it.copyTo(dst) } }
            true
        } catch (e: Exception) {
            Log.e(TAG, "复制工程失败: $e")
            false
        }
    }

    @Deprecated("Deprecated in Java")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (resultCode != RESULT_OK) {
            if (requestCode == REQ_SAVE_FILE) {
                methodChannel?.invokeMethod("onProjectSaveCancelled", "cancel")
            }
            return
        }
        when (requestCode) {
            REQ_OPEN_FILE -> {
                val uri = data?.data
                val name = uri?.let { displayName(it) }
                if (uri == null || name == null || !isProjectName(name)) {
                    methodChannel?.invokeMethod("onFileRejected", name ?: "")
                    return
                }
                val path = copyPicked(uri) ?: return
                // .yin 工程记住原 uri（含持久权限），供“保存工程”直接覆盖。
                if (path.lowercase().endsWith(".yin")) {
                    runCatching {
                        contentResolver.takePersistableUriPermission(
                            uri,
                            Intent.FLAG_GRANT_READ_URI_PERMISSION or
                                Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
                        )
                    }
                    rememberUri(path.substringAfterLast('/'), uri)
                }
                // 不在这里直接加载：Dart 先检查“未保存修改/同一文件”，
                // 再通过 FFI 发起加载，避免覆盖当前工程时没有确认。
                methodChannel?.invokeMethod("onFilePicked", path)
            }
            REQ_OPEN_SF -> {
                val uri = data?.data
                val name = uri?.let { displayName(it) }
                if (uri == null || name == null || !isSoundfontName(name)) {
                    methodChannel?.invokeMethod("onSoundfontRejected", name ?: "")
                    return
                }
                val path = copyPicked(uri) ?: return
                if (isArchivePath(path)) {
                    // 压缩包交给 Dart/Rust 列 SF2/SFZ，避免把 zip 当音色库直接加载。
                    methodChannel?.invokeMethod("onSoundFontArchivePicked", path)
                } else {
                    onSoundFontPicked(path)
                    methodChannel?.invokeMethod("onSoundFontPicked", path)
                }
            }
            REQ_SAVE_FILE -> {
                val uri = data?.data
                if (copyExportTo(uri)) {
                    if (uri != null) {
                        val name = displayName(uri) ?: "Untitled.yin"
                        rememberUri(name, uri)
                        runCatching {
                            contentResolver.takePersistableUriPermission(
                                uri,
                                Intent.FLAG_GRANT_READ_URI_PERMISSION or
                                    Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
                            )
                        }
                    }
                    methodChannel?.invokeMethod("onProjectSaved", "ok")
                } else {
                    methodChannel?.invokeMethod("onProjectSaveFailed", "copy failed")
                }
            }
            REQ_SAVE_EXPORT -> {
                val uri = data?.data
                if (copyTempTo(uri, pendingExportExt)) {
                    val name = uri?.let { displayName(it) } ?: "export"
                    methodChannel?.invokeMethod("onExportSaved", name)
                }
            }
            else -> {}
        }
    }

    /** SAF uri → 私有目录副本，返回绝对路径（失败返回 null）。 */
    private fun copyPicked(uri: android.net.Uri?): String? {
        if (uri == null) {
            return null
        }
        // 只有 .sfz 允许原地读取（采样是相对 sfz 的路径，复制单个文件会断链），
        // 而且必须真的可读；其余（MIDI/工程/压缩包/SF2）一律复制进私有目录，
        // 避免外部路径在分区存储 / 鸿蒙上被判定为 EACCES。
        externalStoragePath(uri)
            ?.takeIf { it.endsWith(".sfz", ignoreCase = true) && File(it).canRead() }
            ?.let { return it }
        val name = displayName(uri) ?: "picked.bin"
        val dest = File(filesDir, name)
        return try {
            val input = contentResolver.openInputStream(uri)
            if (input == null) {
                Log.e(TAG, "打开所选文件失败: $uri")
                return null
            }
            input.use { src ->
                dest.outputStream().use { dst -> src.copyTo(dst) }
            }
            dest.absolutePath
        } catch (e: Exception) {
            Log.e(TAG, "复制所选文件失败: $e")
            null
        }
    }

    /**
     * `content://com.android.externalstorage.documents/...` → `/storage/...` 真实路径。
     * 仅在已授予「所有文件访问」时可用；其他 provider 返回 null 走复制。
     */
    private fun externalStoragePath(uri: Uri): String? {
        if (!hasAllFilesAccess()) {
            return null
        }
        if (uri.authority != "com.android.externalstorage.documents") {
            return null
        }
        val docId = runCatching { DocumentsContract.getDocumentId(uri) }.getOrNull() ?: return null
        val parts = docId.split(':', limit = 2)
        if (parts.size != 2) {
            return null
        }
        val root = if (parts[0] == "primary") {
            Environment.getExternalStorageDirectory().absolutePath
        } else {
            "/storage/${parts[0]}"
        }
        return "$root/${parts[1]}"
    }

    private fun displayName(uri: android.net.Uri): String? {
        return contentResolver.query(uri, null, null, null, null)?.use { c ->
            if (!c.moveToFirst()) {
                return@use null
            }
            val idx = c.getColumnIndex(OpenableColumns.DISPLAY_NAME)
            if (idx >= 0) c.getString(idx) else null
        }
    }

    override fun onDestroy() {
        canvasSurface.release()
        automationSurface.release()
        super.onDestroy()
    }
}
