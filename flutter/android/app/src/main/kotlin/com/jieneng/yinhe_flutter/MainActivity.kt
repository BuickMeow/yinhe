package com.jieneng.yinhe_flutter

import android.content.Intent
import android.provider.OpenableColumns
import android.util.Log
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

        init {
            System.loadLibrary("yinhe_flutter")
        }
    }

    private var producer: TextureRegistry.SurfaceProducer? = null

    /** 与 Dart 的桥（双向：Dart 请求 + 文件选择结果回传）。 */
    private var methodChannel: MethodChannel? = null

    private external fun nativeSetSurface(surface: Surface)

    private external fun nativeClearSurface()

    private external fun nativeSetViewport(width: Float, height: Float)

    /** cpal/AAudio 需要的 NDK context（JavaVM + Application Context）。 */
    private external fun nativeInitContext(context: android.content.Context)

    /** Rust 回调：所选文件已复制到私有目录，path 为绝对路径。 */
    private external fun onFilePicked(path: String)

    /** Rust 回调：所选音色库（SF2/SF3）已复制到私有目录。 */
    private external fun onSoundFontPicked(path: String)

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)

        // cpal/AAudio 需要 NDK context（JavaVM + Application Context）。
        nativeInitContext(applicationContext)

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

        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "yinhe/surface")
            .also { methodChannel = it }
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "getTextureId" -> result.success(p.id())
                    "setSize" -> {
                        val w = call.argument<Int>("w")
                        val h = call.argument<Int>("h")
                        val lw = call.argument<Double>("lw")
                        val lh = call.argument<Double>("lh")
                        if (w == null || h == null || lw == null || lh == null) {
                            result.error("bad_args", "缺少 w/h/lw/lh", null)
                        } else {
                            Log.i(TAG, "setSize $w x $h (logical $lw x $lh)")
                            p.setSize(w, h)
                            // resize 后 buffer 尺寸已变，重发 surface 让 Rust 重建交换链。
                            nativeSetSurface(p.getSurface())
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
                    else -> result.notImplemented()
                }
            }
    }

    /** surface 可用/重建：`getSurface()` 不缓存，每次重新获取。 */
    private fun surfaceReady(p: TextureRegistry.SurfaceProducer) {
        p.getSurface()?.let { nativeSetSurface(it) }
    }

    private fun onSurfaceGone() {
        nativeClearSurface()
    }

    /** Dart 侧「打开 MIDI」：拉系统文件选择器（ACTION_OPEN_DOCUMENT，免权限）。 */
    private fun openFilePicker() {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "*/*"
            putExtra(
                Intent.EXTRA_MIME_TYPES,
                arrayOf("audio/midi", "audio/x-midi", "audio/mid"),
            )
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

    @Deprecated("Deprecated in Java")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (resultCode != RESULT_OK) {
            return
        }
        when (requestCode) {
            REQ_OPEN_FILE -> {
                val path = copyPicked(data?.data) ?: return
                onFilePicked(path)
                methodChannel?.invokeMethod("onFilePicked", path)
            }
            REQ_OPEN_SF -> {
                val path = copyPicked(data?.data) ?: return
                onSoundFontPicked(path)
                methodChannel?.invokeMethod("onSoundFontPicked", path)
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
