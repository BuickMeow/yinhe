package com.jieneng.yinhe_flutter

import android.util.Log
import android.view.Surface
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import io.flutter.view.TextureRegistry

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
        init {
            System.loadLibrary("yinhe_flutter")
        }
    }

    private var producer: TextureRegistry.SurfaceProducer? = null

    private external fun nativeSetSurface(surface: Surface)

    private external fun nativeClearSurface()

    private external fun nativeSetViewport(width: Float, height: Float)

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)

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

    override fun onDestroy() {
        producer?.release()
        producer = null
        super.onDestroy()
    }
}
