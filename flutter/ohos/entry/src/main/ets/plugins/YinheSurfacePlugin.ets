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
const ARCHIVE_EXTS: string[] = ['zip', '7z', 'rar'];
const PROJECT_SUFFIXES: string[] = ['.yin', '.mid', '.midi', '.zip', '.7z', '.rar'];
const SOUNDFONT_SUFFIXES: string[] = ['.sf2', '.sfz', '.zip', '.7z', '.rar'];

function extOf(name: string): string {
  const dot = name.lastIndexOf('.');
  return dot >= 0 ? name.substring(dot + 1).toLowerCase() : '';
}

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
        this.pickProject(result);
        break;
      case 'openSoundFontPicker':
        this.pickSoundfont(result);
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

  private async pickProject(result: MethodResult): Promise<void> {
    try {
      const uri = await this.selectFile(PROJECT_SUFFIXES);
      if (uri === null) {
        result.success(null);
        return;
      }
      const path = this.copyToSandbox(uri);
      this.channel.invokeMethod('onFilePicked', path, null);
      result.success(null);
    } catch (e) {
      result.error('pick_failed', `${e}`, null);
    }
  }

  private async pickSoundfont(result: MethodResult): Promise<void> {
    try {
      const uri = await this.selectFile(SOUNDFONT_SUFFIXES);
      if (uri === null) {
        result.success(null);
        return;
      }
      const path = this.copyToSandbox(uri);
      if (ARCHIVE_EXTS.includes(extOf(baseName(uri)))) {
        this.channel.invokeMethod('onSoundFontArchivePicked', path, null);
      } else {
        yinhe.addSoundFont(path);
        this.channel.invokeMethod('onSoundFontPicked', path, null);
      }
      result.success(null);
    } catch (e) {
      result.error('pick_failed', `${e}`, null);
    }
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

  private async selectFile(suffixes: string[]): Promise<string | null> {
    const options = new picker.DocumentSelectOptions();
    options.maxSelectNumber = 1;
    options.fileSuffixFilters = suffixes;
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
