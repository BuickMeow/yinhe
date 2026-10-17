package com.jieneng.yinhe_flutter

import android.view.Surface
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import io.flutter.view.TextureRegistry

/**
 * 纹理桥宿主：把 SurfaceProducer 的 Surface 交给 Rust(wgpu) 渲染。
 *
 * - 引擎通过 BufferQueue 消费 Rust 生产的最新帧，画布动画无需 Dart 侧 setState。
 * - 回调约定（Flutter 3.27+）：onSurfaceAvailable 后调用 getSurface() 取当前
 *   surface（不要缓存：纹理 resize/换格式时 surface 会换）；onSurfaceCleanup
 *   时停止渲染。
 * - Dart 只用 MethodChannel 拿 textureId / 上报尺寸；触摸走 dart:ffi 直连。
 */
class MainActivity : FlutterActivity() {
    companion object {
        init {
            System.loadLibrary("yinhe_flutter")
        }
    }

    private var producer: TextureRegistry.SurfaceProducer? = null

    private external fun nativeSetSurface(surface: Surface)

    private external fun nativeClearSurface()

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)

        val p = flutterEngine.renderer.createSurfaceProducer()
        p.setCallback(
            object : TextureRegistry.SurfaceProducer.Callback {
                override fun onSurfaceAvailable() {
                    // 不要在回调外缓存 surface；创建/重建后重新获取。
                    p.getSurface()?.let { nativeSetSurface(it) }
                }

                override fun onSurfaceCleanup() {
                    nativeClearSurface()
                }
            },
        )
        producer = p

        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "yinhe/surface")
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "getTextureId" -> result.success(p.id())
                    "setSize" -> {
                        val w = call.argument<Int>("w")
                        val h = call.argument<Int>("h")
                        if (w == null || h == null) {
                            result.error("bad_args", "缺少 w/h", null)
                        } else {
                            p.setSize(w, h)
                            // resize 后 buffer 尺寸已变，重发 surface 让 Rust 重建交换链。
                            p.getSurface()?.let { nativeSetSurface(it) }
                            result.success(null)
                        }
                    }
                    else -> result.notImplemented()
                }
            }
    }

    override fun onDestroy() {
        producer?.release()
        producer = null
        super.onDestroy()
    }
}
