package com.jieneng.yinhe

import android.content.Intent
import android.provider.OpenableColumns
import android.util.Log
import android.view.InputDevice
import android.view.MotionEvent
import android.view.Surface
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import io.flutter.view.TextureRegistry
import java.io.File

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

        /** SAF 保存工程请求码。 */
        private const val REQ_SAVE_FILE = 1003
        private const val REQ_SAVE_EXPORT = 1004

        /** SAF 目录选择（SFZ 音色库连同采样一起导入）。 */
        /** 导出工程的临时文件（Rust 写入，Kotlin 复制到用户选择的位置）。 */
        private const val EXPORT_TMP = "export.ying"

        init {
            // 压缩包支持（unarc-rs）带 C++ 代码：先加载 NDK 的 C++ 运行时，
            // 否则 libyinhe_flutter.so 里的 std::length_error 等符号无法解析。
            System.loadLibrary("c++_shared")
            System.loadLibrary("yinhe_flutter")
        }
    }

    private var producer: TextureRegistry.SurfaceProducer? = null

    /** 延迟到 Dart 请求纹理时才创建 SurfaceProducer（see ensureProducer）。 */
    private var engine: FlutterEngine? = null

    /** 原生层已拦截的鼠标右键拖拽（避免 Android 把右键当 BACK）。 */
    private var rightPanActive = false

    /** 本代 surface 是否已发送给 Rust（同一窗口重复设置会触发 IN_USE）。 */
    private var surfaceSent = false

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

    private external fun nativeSetSize(width: Int, height: Int)

    private external fun nativeSetViewport(width: Float, height: Float)

    /** cpal/AAudio 需要的 NDK context（JavaVM + Application Context）。 */
    private external fun nativeInitContext(context: android.content.Context, filesDir: String)

    /** Rust 回调：所选文件已复制到私有目录，path 为绝对路径。 */
    private external fun onFilePicked(path: String)

    /** Rust 回调：所选音色库（SF2/SF3）已复制到私有目录。 */
    private external fun onSoundFontPicked(path: String)

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
                        val p = ensureProducer()
                        if (p == null) {
                            result.error("no_engine", "FlutterEngine 未就绪", null)
                        } else {
                            result.success(p.id())
                        }
                    }
                    "setSize" -> {
                        val w = call.argument<Int>("w")
                        val h = call.argument<Int>("h")
                        val lw = call.argument<Double>("lw")
                        val lh = call.argument<Double>("lh")
                        val p = ensureProducer()
                        if (w == null || h == null || lw == null || lh == null) {
                            result.error("bad_args", "缺少 w/h/lw/lh", null)
                        } else if (p == null) {
                            result.error("no_engine", "FlutterEngine 未就绪", null)
                        } else {
                            Log.i(TAG, "setSize $w x $h (logical $lw x $lh)")
                            p.setSize(w, h)
                            // resize 只重配置交换链：对同一 ANativeWindow 重建
                            // surface 会触发 VK_ERROR_NATIVE_WINDOW_IN_USE_KHR。
                            nativeSetSize(w, h)
                            nativeSetViewport(lw.toFloat(), lh.toFloat())
                            // 引擎在 setSize 时 surface 必已创建：确保已发送。
                            surfaceReady(p)
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
                        ensureProducer()?.let { surfaceReady(it) }
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

    /**
     * 惰性创建 SurfaceProducer。
     *
     * 不能在 configureFlutterEngine 里创建：release 包此时 FlutterView 尚未挂载，
     * getSurface() 会走 FlutterJNI.registerImageTexture 并触发引擎断言闪退。
     * Dart 初始化后通过 getTextureId/ensureSurface 调用本方法，此时视图已就绪。
     */
    private fun ensureProducer(): TextureRegistry.SurfaceProducer? {
        producer?.let { return it }
        val flutterEngine = engine ?: return null
        val p = flutterEngine.renderer.createSurfaceProducer()
        p.setCallback(
            object : TextureRegistry.SurfaceProducer.Callback {
                // 引擎（3.47）调用旧入口 onSurfaceCreated/onSurfaceDestroyed；
                // 新入口 onSurfaceAvailable/onSurfaceCleanup 是给未来版本的。
                // 两个都接：引擎只会调用其中一个，经同一 helper 幂等处理。
                override fun onSurfaceCreated() {
                    Log.i(TAG, "onSurfaceCreated")
                    surfaceReady(p)
                }

                override fun onSurfaceAvailable() {
                    Log.i(TAG, "onSurfaceAvailable")
                    surfaceReady(p)
                }

                override fun onSurfaceDestroyed() {
                    Log.i(TAG, "onSurfaceDestroyed")
                    onSurfaceGone()
                }

                override fun onSurfaceCleanup() {
                    Log.i(TAG, "onSurfaceCleanup")
                    onSurfaceGone()
                }
            },
        )
        producer = p
        // 若 surface 已惰性创建则立即发送（回调在部分引擎版本不触发）。
        surfaceReady(p)
        return p
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

    /** surface 可用/重建：每代只发送一次（created/available 可能都回调）。 */
    private fun surfaceReady(p: TextureRegistry.SurfaceProducer) {
        if (surfaceSent) {
            return
        }
        val s = p.getSurface() ?: return
        surfaceSent = true
        nativeSetSurface(s)
    }

    private fun onSurfaceGone() {
        surfaceSent = false
        nativeClearSurface()
    }

    /** Dart 侧「打开文件」：MIDI 与 .ying 工程（未知扩展名走 octet-stream）。 */
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
            // 其它 MIME 而置灰；这里放开为全部文件，压缩包（zip/7z…）也要能选。
        }
        startActivityForResult(intent, REQ_OPEN_FILE)
    }

    /** Dart 侧「选择音色库」：SF2/SF3 无统一 MIME，交给用户选择。 */
    private fun openSoundFontPicker() {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "*/*"
        }
        startActivityForResult(intent, REQ_OPEN_SF)
    }

    /** 与 Rust/Dart 保持一致的压缩包扩展名。 */
    private fun isArchivePath(path: String): Boolean {
        val lower = path.lowercase()
        return listOf(
            ".zip", ".7z", ".rar", ".lzh", ".lha", ".tar",
            ".gz", ".xz", ".tgz", ".txz", ".tbz", ".bz2",
        ).any { lower.endsWith(it) }
    }

    /** Dart 侧「保存工程」：先由 Rust 写好临时文件，这里复制到用户选择的位置。 */
    private fun openSavePicker(name: String) {
        val safe = name.replace(Regex("[/\\\\:*?\"<>|]"), "_").trim()
        val base = if (safe.isEmpty()) "Untitled" else safe
        val title = if (base.endsWith(".ying", ignoreCase = true)) base else "$base.ying"
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
            return
        }
        when (requestCode) {
            REQ_OPEN_FILE -> {
                val uri = data?.data
                val path = copyPicked(uri) ?: return
                // .ying 工程记住原 uri（含持久权限），供“保存工程”直接覆盖。
                if (path.lowercase().endsWith(".ying") && uri != null) {
                    runCatching {
                        contentResolver.takePersistableUriPermission(
                            uri,
                            Intent.FLAG_GRANT_READ_URI_PERMISSION or
                                Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
                        )
                    }
                    rememberUri(path.substringAfterLast('/'), uri)
                }
                onFilePicked(path)
                methodChannel?.invokeMethod("onFilePicked", path)
            }
            REQ_OPEN_SF -> {
                val path = copyPicked(data?.data) ?: return
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
                        val name = displayName(uri) ?: "Untitled.ying"
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
        producer?.release()
        producer = null
        super.onDestroy()
    }
}
