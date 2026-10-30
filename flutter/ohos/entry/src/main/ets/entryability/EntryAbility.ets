import { FlutterAbility, FlutterEngine } from '@ohos/flutter_ohos';
import { GeneratedPluginRegistrant } from '../plugins/GeneratedPluginRegistrant';
import { YinheSurfacePlugin } from '../plugins/YinheSurfacePlugin';

export default class EntryAbility extends FlutterAbility {
  private surfacePlugin: YinheSurfacePlugin | null = null;

  configureFlutterEngine(flutterEngine: FlutterEngine) {
    super.configureFlutterEngine(flutterEngine)
    GeneratedPluginRegistrant.registerWith(flutterEngine)
    // YINHE：wgpu 渲染走外接纹理，Dart 侧仍用 `yinhe/surface` 通道拿 textureId。
    this.surfacePlugin = new YinheSurfacePlugin(flutterEngine, this.context);
  }

  onDestroy(): void {
    this.surfacePlugin?.dispose();
    this.surfacePlugin = null;
    super.onDestroy();
  }
}
