package com.jieneng.yinhe

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import android.provider.Settings
import android.util.Log
import android.view.Surface
import android.widget.Toast
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import io.flutter.view.TextureRegistry
import java.io.File
import java.io.InputStream
import java.io.OutputStream
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors

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

        /** 整文件复制的分块大小：1MB，兼顾吞吐与进度粒度。 */
        private const val COPY_BUFFER_SIZE = 1 shl 20

        /** 进度事件节流间隔（毫秒）：GB 级文件不节流会把主线程事件队列淹掉。 */
        private const val COPY_PROGRESS_INTERVAL_MS = 100L

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

    /**
     * 整文件复制专用单线程：工程/音色库动辄几十 MB～GB，放主线程会卡住输入与渲染。
     * 单线程既能保证复制串行（避免同名文件互相覆盖），也省掉并发调度的复杂度。
     * daemon：进程退出时不阻塞，也不需要在 onDestroy 里等复制结束。
     */
    private val fileIo: ExecutorService by lazy {
        Executors.newSingleThreadExecutor { r ->
            Thread(r, "yinhe-file-io").apply { isDaemon = true }
        }
    }

    /**
     * 主线程 Handler：所有 `methodChannel.invokeMethod` 必须在主线程调用
     * （Flutter 的 channel 不能跨线程直接回调），后台复制只能通过它回送事件。
     */
    private val mainHandler = Handler(Looper.getMainLooper())

    private external fun nativeSetSurface(surface: Surface)

    private external fun nativeClearSurface()

    private external fun nativeSetAutomationSurface(surface: Surface)

    private external fun nativeClearAutomationSurface()

    private external fun nativeSetAutomationSize(width: Int, height: Int)

    private external fun nativeSetSize(width: Int, height: Int)

    private external fun nativeSetViewport(width: Float, height: Float)

    /** cpal/AAudio 需要的 NDK context（JavaVM + Application Context）。 */
    private external fun nativeInitContext(context: android.content.Context, filesDir: String)

    /**
     * 本轮选择允许的扩展名（小写、不含点）。
     *
     * 文件类型策略由 Dart 的 `core/platform_files.dart` 统一下发，壳不再各存一份表。
     */
    private var pendingExts: Set<String> = emptySet()

    /** 扩展名提取（小写、不含点；无扩展名返回空串）。 */
    private fun extensionOf(name: String): String {
        val dot = name.lastIndexOf('.')
        return if (dot >= 0 && dot + 1 < name.length) {
            name.substring(dot + 1).lowercase()
        } else {
            ""
        }
    }

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
                        pendingExts = call.argument<List<String>>("exts")
                            ?.map { it.lowercase() }?.toSet() ?: emptySet()
                        openFilePicker()
                        result.success(null)
                    }
                    "openSoundFontPicker" -> {
                        pendingExts = call.argument<List<String>>("exts")
                            ?.map { it.lowercase() }?.toSet() ?: emptySet()
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
                            // 复制在后台：几十 MB 的工程不能阻塞主线程，先立即返回 result。
                            copyTempToAsync(uri, "", name, "saveProject") { ok ->
                                if (ok) {
                                    methodChannel?.invokeMethod("onProjectSaved", "ok")
                                } else {
                                    lastProjectUri = null
                                    openSavePicker(name)
                                }
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

    /** 所选文件是否在 Dart 下发的扩展名白名单里。 */
    private fun isAllowedName(name: String): Boolean =
        extensionOf(name) in pendingExts

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

    /** 后台线程回送 MethodChannel 事件：channel 只能在创建它的主线程使用。 */
    private fun postEvent(method: String, args: Any?) {
        mainHandler.post { methodChannel?.invokeMethod(method, args) }
    }

    /**
     * 约 100ms 节流一次进度事件；复制完成那块一定上报（done == total），
     * 保证 Dart 侧进度条能走到 100%。返回的回调在后台线程调用。
     */
    private fun progressEmitter(kind: String, name: String, total: Long): (Long) -> Unit {
        var lastMs = 0L
        return { done ->
            val now = SystemClock.uptimeMillis()
            if (done == total || now - lastMs >= COPY_PROGRESS_INTERVAL_MS) {
                lastMs = now
                postEvent(
                    "onCopyProgress",
                    mapOf("kind" to kind, "name" to name, "done" to done, "total" to total),
                )
            }
        }
    }

    /**
     * 1MB 分块复制并汇报累计字节。读整文件到内存会在 GB 级文件上 OOM，
     * 所以固定小缓冲循环。只在后台线程调用。
     */
    private fun copyChunked(
        src: InputStream,
        dst: OutputStream,
        onProgress: (Long) -> Unit,
    ) {
        val buf = ByteArray(COPY_BUFFER_SIZE)
        var done = 0L
        while (true) {
            val n = src.read(buf)
            if (n < 0) {
                break
            }
            dst.write(buf, 0, n)
            done += n
            onProgress(done)
        }
    }

    /** SAF 文件大小（OpenableColumns.SIZE）；provider 不提供时返回 -1。 */
    private fun querySize(uri: Uri): Long {
        val size = runCatching {
            contentResolver.query(uri, arrayOf(OpenableColumns.SIZE), null, null, null)?.use { c ->
                val idx = c.getColumnIndex(OpenableColumns.SIZE)
                if (idx >= 0 && c.moveToFirst() && !c.isNull(idx)) c.getLong(idx) else null
            }
        }.getOrNull()
        return size ?: -1L
    }

    /**
     * 把导出的临时文件（ext 决定文件名）复制到用户选择的 uri。
     * 复制必须在后台：工程/导出文件可能几十 MB～GB，同步复制会让主线程 ANR。
     * onDone 已切回主线程，调用方可以安全地发 channel 事件/弹选择器。
     */
    private fun copyTempToAsync(
        uri: Uri,
        ext: String,
        name: String,
        kind: String,
        onDone: (Boolean) -> Unit,
    ) {
        val fileName = if (ext.isBlank()) EXPORT_TMP else "export_tmp.$ext"
        val src = File(filesDir, fileName)
        if (!src.exists()) {
            Log.e(TAG, "导出临时文件不存在: ${src.absolutePath}")
            onDone(false)
            return
        }
        fileIo.execute {
            val total = src.length()
            postEvent("onCopyStart", mapOf("kind" to kind, "name" to name, "total" to total))
            val ok = runCatching {
                // 必须显式用 "wt"：部分 SAF provider 的默认 "w" 不截断旧文件，
                // 新工程比旧工程短时会把旧文件尾部残留成 mixer/audio 伪段。
                val output = contentResolver.openOutputStream(uri, "wt")
                    ?: return@runCatching false
                output.use { dst ->
                    src.inputStream().use { srcStream ->
                        copyChunked(srcStream, dst, progressEmitter(kind, name, total))
                    }
                }
                true
            }.getOrElse { e ->
                Log.e(TAG, "复制临时文件失败: $e")
                false
            }
            mainHandler.post { onDone(ok) }
        }
    }

    /**
     * 把 SAF uri 复制进私有目录（后台）。
     * 主线程只负责取名字/校验；复制完再回主线程做权限记忆与事件上报。
     */
    private fun copyPickedAsync(
        uri: Uri,
        name: String,
        kind: String,
        onDone: (String?) -> Unit,
    ) {
        fileIo.execute {
            val total = querySize(uri)
            postEvent("onCopyStart", mapOf("kind" to kind, "name" to name, "total" to total))
            val dest = File(filesDir, name)
            val ok = runCatching {
                val input = contentResolver.openInputStream(uri)
                    ?: return@runCatching false
                input.use { src ->
                    dest.outputStream().use { dst ->
                        copyChunked(src, dst, progressEmitter(kind, name, total))
                    }
                }
                true
            }.getOrElse { e ->
                Log.e(TAG, "复制所选文件失败: $e")
                false
            }
            // 失败时不删 dest：与旧行为一致（旧实现截断后中途失败也留下部分文件）。
            mainHandler.post { onDone(if (ok) dest.absolutePath else null) }
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
                if (uri == null || name == null || !isAllowedName(name)) {
                    methodChannel?.invokeMethod("onFileRejected", name ?: "")
                    return
                }
                // .sfz 原地读取的判定仍在主线程：要查「所有文件访问」和真实路径，
                // 命中原路径就直接用，不复制（采样是相对 sfz 的路径，复制会断链）。
                inPlaceSfzPath(uri)?.let { path ->
                    methodChannel?.invokeMethod("onFilePicked", path)
                    return
                }
                copyPickedAsync(uri, name, "open") { path ->
                    if (path == null) {
                        // 复制失败要让 Dart 知道，否则 UI 一直等结果。
                        methodChannel?.invokeMethod("onFileRejected", name)
                        return@copyPickedAsync
                    }
                    // .yin 工程记住原 uri（含持久权限），供“保存工程”直接覆盖。
                    // 这些都在复制完成后回主线程执行。
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
            }
            REQ_OPEN_SF -> {
                val uri = data?.data
                val name = uri?.let { displayName(it) }
                if (uri == null || name == null || !isAllowedName(name)) {
                    methodChannel?.invokeMethod("onSoundfontRejected", name ?: "")
                    return
                }
                inPlaceSfzPath(uri)?.let { path ->
                    methodChannel?.invokeMethod("onSoundFontPicked", path)
                    return
                }
                copyPickedAsync(uri, name, "soundfont") { path ->
                    if (path == null) {
                        methodChannel?.invokeMethod("onSoundfontRejected", name)
                        return@copyPickedAsync
                    }
                    // 压缩包还是裸音色库由 Dart 判定（入表也由 Dart 调 FFI）。
                    methodChannel?.invokeMethod("onSoundFontPicked", path)
                }
            }
            REQ_SAVE_FILE -> {
                val uri = data?.data
                if (uri == null) {
                    // 旧实现是 copyExportTo(null) 返回 false，语义保持一致。
                    methodChannel?.invokeMethod("onProjectSaveFailed", "copy failed")
                    return
                }
                // displayName 仍在主线程取：后台只做字节复制。
                val name = displayName(uri) ?: "Untitled.yin"
                copyTempToAsync(uri, "", name, "saveProject") { ok ->
                    if (ok) {
                        rememberUri(name, uri)
                        runCatching {
                            contentResolver.takePersistableUriPermission(
                                uri,
                                Intent.FLAG_GRANT_READ_URI_PERMISSION or
                                    Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
                            )
                        }
                        methodChannel?.invokeMethod("onProjectSaved", "ok")
                    } else {
                        methodChannel?.invokeMethod("onProjectSaveFailed", "copy failed")
                    }
                }
            }
            REQ_SAVE_EXPORT -> {
                val uri = data?.data ?: return
                val name = displayName(uri) ?: "export"
                copyTempToAsync(uri, pendingExportExt, name, "saveExport") { ok ->
                    // 导出复制失败保持旧行为：只 log（在后台已记录）。
                    if (ok) {
                        methodChannel?.invokeMethod("onExportSaved", name)
                    }
                }
            }
            else -> {}
        }
    }

    /**
     * .sfz 允许原地读取：采样是相对 sfz 的路径，复制单个文件会断链。
     * 必须真的可读才用原路径；其余（MIDI/工程/压缩包/SF2）一律复制进私有目录，
     * 避免外部路径在分区存储 / 鸿蒙上被判定为 EACCES。需在主线程调用。
     */
    private fun inPlaceSfzPath(uri: Uri): String? =
        externalStoragePath(uri)
            ?.takeIf { it.endsWith(".sfz", ignoreCase = true) && File(it).canRead() }

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
