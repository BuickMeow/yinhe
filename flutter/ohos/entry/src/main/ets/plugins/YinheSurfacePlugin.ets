/*
 * YINHE 的 ArkTS 侧平台桥（对应 Android 的 MainActivity 方法通道）。
 *
 * 链路：Flutter 外接纹理（Flutter-OH 的 TextureRegistry）
 *       → SurfaceTextureEntry.getNativeWindowPtr()（OHNativeWindow*）
 *       → NAPI（libyinhe_napi.so）→ libyinhe_flutter.so 的 C ABI → wgpu(GL) 渲染。
 *
 * Dart 只认 `yinhe/surface` 这条 MethodChannel，与 Android 保持一致。
 */

import yinhe from 'libyinhe_napi.so';
import {
  FlutterEngine,
  FlutterRenderer,
  MethodCall,
  MethodCallHandler,
  MethodChannel,
  MethodResult,
  SurfaceTextureEntry,
} from '@ohos/flutter_ohos';

const CHANNEL_NAME = 'yinhe/surface';

export class YinheSurfacePlugin implements MethodCallHandler {
  private readonly engine: FlutterEngine;
  private readonly renderer: FlutterRenderer;
  private readonly channel: MethodChannel;

  private mainTextureId: number = -1;
  private amTextureId: number = -1;
  private mainEntry: SurfaceTextureEntry | null = null;
  private amEntry: SurfaceTextureEntry | null = null;

  constructor(engine: FlutterEngine) {
    this.engine = engine;
    this.renderer = engine.getFlutterRenderer();
    this.channel = new MethodChannel(engine.getDartExecutor().getBinaryMessenger(), CHANNEL_NAME);
    this.channel.setMethodCallHandler(this);
    yinhe.init();
  }

  onMethodCall(call: MethodCall, result: MethodResult): void {
    switch (call.method) {
      case 'getTextureId':
        result.success(this.ensureMain());
        break;
      case 'ensureSurface':
        this.ensureMain();
        result.success(null);
        break;
      case 'getAutomationTextureId':
        result.success(this.ensureAutomation());
        break;
      case 'ensureAutomationSurface':
        this.ensureAutomation();
        result.success(null);
        break;
      case 'setSize': {
        const w = call.argument('w') as number;
        const h = call.argument('h') as number;
        const lw = call.argument('lw') as number;
        const lh = call.argument('lh') as number;
        const id = this.ensureMain();
        this.renderer.setTextureBufferSize(id, w, h);
        yinhe.setSize(w, h);
        yinhe.setViewport(lw, lh);
        result.success(null);
        break;
      }
      case 'setAutomationSize': {
        const w = call.argument('w') as number;
        const h = call.argument('h') as number;
        const id = this.ensureAutomation();
        this.renderer.setTextureBufferSize(id, w, h);
        yinhe.setAutomationSize(w, h);
        result.success(null);
        break;
      }
      default:
        // 文件选择 / 导出等能力尚未在鸿蒙侧实现；返回错误让 Dart 走降级分支。
        result.error('unsupported', `鸿蒙暂不支持 ${call.method}`, null);
        break;
    }
  }

  /** 销毁时断开纹理与原生 surface（Ability 退出）。 */
  dispose(): void {
    this.channel.setMethodCallHandler(null);
    if (this.mainEntry !== null) {
      this.renderer.unregisterTexture(this.mainTextureId);
      this.mainEntry = null;
      this.mainTextureId = -1;
    }
    if (this.amEntry !== null) {
      this.renderer.unregisterTexture(this.amTextureId);
      this.amEntry = null;
      this.amTextureId = -1;
    }
    yinhe.clearSurface();
  }

  private ensureMain(): number {
    if (this.mainEntry !== null) {
      return this.mainTextureId;
    }
    const id = this.renderer.getTextureId();
    const entry = this.renderer.registerTexture(id);
    // 接口 `SurfaceTextureEntry` 只暴露了已废弃的 getNativeWindowId（number），
    // 真正的 OHNativeWindow* 在 FlutterNapi 上（bigint），这里直接取。
    yinhe.setSurface(this.engine.getFlutterNapi().getTextureNativeWindowPtr(id));
    this.mainEntry = entry;
    this.mainTextureId = id;
    return id;
  }

  private ensureAutomation(): number {
    if (this.amEntry !== null) {
      return this.amTextureId;
    }
    const id = this.renderer.getTextureId();
    const entry = this.renderer.registerTexture(id);
    yinhe.setAutomationSurface(this.engine.getFlutterNapi().getTextureNativeWindowPtr(id));
    this.amEntry = entry;
    this.amTextureId = id;
    return id;
  }
}
