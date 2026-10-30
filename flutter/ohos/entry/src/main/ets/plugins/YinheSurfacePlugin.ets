/*
 * YINHE 的 ArkTS 侧平台桥（对应 Android 的 MainActivity 方法通道）。
 *
 * 链路：Flutter 外接纹理（Flutter-OH 的 TextureRegistry）
 *       → OHNativeWindow*（bigint）
 *       → NAPI（libyinhe_napi.so）→ libyinhe_flutter.so 的 C ABI → wgpu(GL) 渲染。
 *
 * Dart 只认 `yinhe/surface` 这条 MethodChannel，与 Android 保持一致。
 * 文件选择用系统 DocumentViewPicker，选中的文件先复制到应用沙箱（Rust 只认真实路径）。
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
import common from '@ohos.app.ability.common';
import fs from '@ohos.file.fs';
import picker from '@ohos.file.picker';

const CHANNEL_NAME = 'yinhe/surface';

function baseName(uri: string): string {
  const slash = uri.lastIndexOf('/');
  return slash >= 0 ? uri.substring(slash + 1) : uri;
}

export class YinheSurfacePlugin implements MethodCallHandler {
  private readonly engine: FlutterEngine;
  private readonly renderer: FlutterRenderer;
  private readonly channel: MethodChannel;
  private readonly context: common.UIAbilityContext;

  private mainTextureId: number = -1;
  private amTextureId: number = -1;
  private mainEntry: SurfaceTextureEntry | null = null;
  private amEntry: SurfaceTextureEntry | null = null;

  /** 当前工程的保存位置（name → uri），用于「保存」直接覆盖。 */
  private projectUris: Map<string, string> = new Map();

  constructor(engine: FlutterEngine, context: common.UIAbilityContext) {
    this.engine = engine;
    this.context = context;
    this.renderer = engine.getFlutterRenderer();
    this.channel = new MethodChannel(engine.getDartExecutor().getBinaryMessenger(), CHANNEL_NAME);
    this.channel.setMethodCallHandler(this);
    // Rust 需要沙箱 files 目录：设置/最近文件/音色库列表都落在这里。
    // codeDir 供 NAPI 拼出 libs/arm64/libyinhe_flutter.so 的绝对路径再 dlopen
    // （按 soname 在鸿蒙上不保证能找到）；filesDir 供 Rust 落盘。
    yinhe.init(context.bundleCodeDir, context.filesDir);
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
      case 'openFilePicker':
        this.pickProject(this.readExts(call), result);
        break;
      case 'openSoundFontPicker':
        this.pickSoundfont(this.readExts(call), result);
        break;
      case 'exportTempPath': {
        const ext = (call.argument('ext') as string | null) ?? '';
        const name = ext.length === 0 ? 'export.yin' : `export_tmp.${ext}`;
        result.success(`${this.context.filesDir}/${name}`);
        break;
      }
      case 'saveProject': {
        const name = (call.argument('name') as string | null) ?? 'Untitled';
        const saveAs = (call.argument('saveAs') as boolean | null) ?? false;
        this.saveProject(name, saveAs, result);
        break;
      }
      case 'openSavePicker': {
        const name = (call.argument('name') as string | null) ?? 'Untitled';
        this.saveProject(name, true, result);
        break;
      }
      case 'saveExport': {
        const name = (call.argument('name') as string | null) ?? 'export';
        const ext = (call.argument('ext') as string | null) ?? '';
        this.saveExport(name, ext, result);
        break;
      }
      case 'rememberProject': {
        const name = call.argument('name') as string | null;
        if (name !== null && name.length > 0) {
          // 最近列表恢复：只记录名字，真正覆盖保存仍走「另存为」。
          this.projectUris.delete(name);
        }
        result.success(null);
        break;
      }
      default:
        result.error('unsupported', `鸿蒙暂不支持 ${call.method}`, null);
        break;
    }
  }

  /** Ability 退出时断开纹理。 */
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
    // 接口 `SurfaceTextureEntry` 只暴露已废弃的 getNativeWindowId（number），
    // 真正的 OHNativeWindow* 在 FlutterNapi 上（bigint），这里直接取。
    const ptr = this.engine.getFlutterNapi().getTextureNativeWindowPtr(id);
    yinhe.setSurface(ptr);
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

  // ── 文件选择 ──

  /** 本轮允许的扩展名（小写、不含点），由 Dart `core/platform_files.dart` 下发。 */
  private readExts(call: MethodCall): string[] {
    const raw = call.argument('exts') as string[] | null;
    return (raw ?? []).map((e: string) => e.toLowerCase());
  }

  private async pickProject(exts: string[], result: MethodResult): Promise<void> {
    try {
      const uri = await this.selectFile(exts);
      if (uri === null) {
        result.success(null);
        return;
      }
      const name = baseName(uri);
      if (!this.isAllowed(exts, name)) {
        this.channel.invokeMethod('onFileRejected', name, null);
      } else {
        this.channel.invokeMethod('onFilePicked', this.copyToSandbox(uri), null);
      }
      result.success(null);
    } catch (e) {
      result.error('pick_failed', `${e}`, null);
    }
  }

  private async pickSoundfont(exts: string[], result: MethodResult): Promise<void> {
    try {
      const uri = await this.selectFile(exts);
      if (uri === null) {
        result.success(null);
        return;
      }
      const name = baseName(uri);
      if (!this.isAllowed(exts, name)) {
        this.channel.invokeMethod('onSoundfontRejected', name, null);
      } else {
        // 压缩包还是裸音色库、以及入表，都由 Dart 判定/调用。
        this.channel.invokeMethod('onSoundFontPicked', this.copyToSandbox(uri), null);
      }
      result.success(null);
    } catch (e) {
      result.error('pick_failed', `${e}`, null);
    }
  }

  /** 扩展名是否在白名单里（无扩展名视为不匹配）。 */
  private isAllowed(exts: string[], name: string): boolean {
    const dot = name.lastIndexOf('.');
    if (dot < 0) {
      return false;
    }
    return exts.includes(name.substring(dot + 1).toLowerCase());
  }

  private async saveProject(name: string, saveAs: boolean, result: MethodResult): Promise<void> {
    const tempPath = `${this.context.filesDir}/export.yin`;
    try {
      if (!saveAs) {
        const known = this.projectUris.get(name);
        if (known !== undefined) {
          this.copyToUri(tempPath, known);
          this.channel.invokeMethod('onProjectSaved', 'ok', null);
          result.success(null);
          return;
        }
      }
      const fileName = name.endsWith('.yin') ? name : `${name}.yin`;
      const uri = await this.saveFile(fileName);
      if (uri === null) {
        this.channel.invokeMethod('onProjectSaveCancelled', 'cancel', null);
        result.success(null);
        return;
      }
      this.copyToUri(tempPath, uri);
      this.projectUris.set(name, uri);
      this.channel.invokeMethod('onProjectSaved', 'ok', null);
      result.success(null);
    } catch (e) {
      this.channel.invokeMethod('onProjectSaveFailed', `${e}`, null);
      result.error('save_failed', `${e}`, null);
    }
  }

  private async saveExport(name: string, ext: string, result: MethodResult): Promise<void> {
    const tempPath = ext.length === 0
      ? `${this.context.filesDir}/export.yin`
      : `${this.context.filesDir}/export_tmp.${ext}`;
    try {
      const fileName = ext.length === 0
        ? name
        : (name.toLowerCase().endsWith(`.${ext}`) ? name : `${name}.${ext}`);
      const uri = await this.saveFile(fileName);
      if (uri === null) {
        result.error('cancelled', '用户取消保存', null);
        return;
      }
      this.copyToUri(tempPath, uri);
      this.channel.invokeMethod('onExportSaved', fileName, null);
      result.success(null);
    } catch (e) {
      result.error('save_failed', `${e}`, null);
    }
  }

  private async selectFile(exts: string[]): Promise<string | null> {
    const options = new picker.DocumentSelectOptions();
    options.maxSelectNumber = 1;
    options.fileSuffixFilters = exts.map((e: string) => `.${e}`);
    const viewPicker = new picker.DocumentViewPicker(this.context);
    const uris = await viewPicker.select(options);
    return uris.length > 0 ? uris[0] : null;
  }

  private async saveFile(name: string): Promise<string | null> {
    const options = new picker.DocumentSaveOptions();
    options.newFileNames = [name];
    const viewPicker = new picker.DocumentViewPicker(this.context);
    const uris = await viewPicker.save(options);
    return uris.length > 0 ? uris[0] : null;
  }

  /** 系统选择器给的 uri → 应用沙箱里的真实路径（Rust 只认路径）。 */
  private copyToSandbox(uri: string): string {
    const name = baseName(uri).length > 0 ? baseName(uri) : 'picked.bin';
    const dest = `${this.context.filesDir}/${name}`;
    const src = fs.openSync(uri, fs.OpenMode.READ_ONLY);
    try {
      const dst = fs.openSync(dest, fs.OpenMode.CREATE | fs.OpenMode.READ_WRITE | fs.OpenMode.TRUNC);
      try {
        fs.copyFileSync(src.fd, dst.fd);
      } finally {
        fs.closeSync(dst);
      }
    } finally {
      fs.closeSync(src);
    }
    return dest;
  }

  /** 沙箱文件 → 用户选中的保存位置。 */
  private copyToUri(srcPath: string, uri: string): void {
    const src = fs.openSync(srcPath, fs.OpenMode.READ_ONLY);
    try {
      const dst = fs.openSync(uri, fs.OpenMode.READ_WRITE | fs.OpenMode.TRUNC);
      try {
        fs.copyFileSync(src.fd, dst.fd);
      } finally {
        fs.closeSync(dst);
      }
    } finally {
      fs.closeSync(src);
    }
  }
}
