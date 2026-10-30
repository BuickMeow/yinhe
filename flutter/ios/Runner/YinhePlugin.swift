import Flutter
import UIKit

/// `yinhe/surface` 通道的 iOS 实现。
///
/// 与 Android 的 `MainActivity` / 鸿蒙的 `YinheSurfacePlugin` 对齐：Dart 只认
/// 这一条通道，平台壳负责外接纹理、尺寸上报与文件选择。
final class YinhePlugin: NSObject, FlutterPlugin {
  private static let channelName = "yinhe/surface"
  private static let canvasKind: Int32 = 0
  private static let automationKind: Int32 = 1

  private let registrar: FlutterPluginRegistrar
  private let canvas: YinheTexture
  private let automation: YinheTexture

  private init(registrar: FlutterPluginRegistrar) {
    self.registrar = registrar
    let textures = registrar.textures()
    canvas = YinheTexture(kind: Self.canvasKind, registry: textures)
    automation = YinheTexture(kind: Self.automationKind, registry: textures)
    super.init()
  }

  static func register(with registrar: FlutterPluginRegistrar) {
    let plugin = YinhePlugin(registrar: registrar)
    let channel = FlutterMethodChannel(
      name: channelName, binaryMessenger: registrar.messenger())
    channel.setMethodCallHandler(plugin.handleCall)

    // 宿主回调先登记，再初始化引擎：Rust 建好 wgpu 设备后宿主才能建纹理池。
    yinhe_ios_set_host(yinheAcquireTexture, yinheSubmitTexture)
    let dir = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
    yinhe_ios_init(dir.path)
  }

  // MARK: - MethodChannel

  private func handleCall(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
    let args = call.arguments as? [String: Any]
    switch call.method {
    case "getTextureId":
      YinheTextures.shared.set(canvas)
      result(canvas.register())
    case "getAutomationTextureId":
      YinheTextures.shared.set(automation)
      result(automation.register())
    case "ensureSurface":
      yinhe_ios_ensure(Self.canvasKind)
      result(nil)
    case "ensureAutomationSurface":
      yinhe_ios_ensure(Self.automationKind)
      result(nil)
    case "setSize":
      let w = (args?["w"] as? NSNumber)?.int32Value ?? 0
      let h = (args?["h"] as? NSNumber)?.int32Value ?? 0
      let lw = (args?["lw"] as? NSNumber)?.floatValue ?? 0
      let lh = (args?["lh"] as? NSNumber)?.floatValue ?? 0
      yinhe_ios_set_size(Self.canvasKind, w, h)
      yinhe_ios_set_viewport(lw, lh)
      result(nil)
    case "setAutomationSize":
      let w = (args?["w"] as? NSNumber)?.int32Value ?? 0
      let h = (args?["h"] as? NSNumber)?.int32Value ?? 0
      yinhe_ios_set_size(Self.automationKind, w, h)
      result(nil)
    case "exportTempPath":
      // Rust 写临时文件，宿主再交给用户选位置（Phase 2 接 UIDocumentPicker）。
      let ext = call.arguments as? String ?? ""
      let name = ext.isEmpty ? "export.yin" : "export_tmp.\(ext)"
      result(FileManager.default.temporaryDirectory.appendingPathComponent(name).path)
    case "openFilePicker", "openSoundFontPicker", "saveProject", "saveExport",
      "rememberProject":
      // 文件选择/保存：Phase 2（UIDocumentPicker + 沙箱拷贝 + bookmark）。
      result(nil)
    default:
      result(FlutterMethodNotImplemented)
    }
  }
}

// MARK: - 纹理登记表（C 回调按 kind 找宿主对象）

final class YinheTextures {
  static let shared = YinheTextures()
  private let lock = NSLock()
  private var byKind: [Int32: YinheTexture] = [:]

  func set(_ texture: YinheTexture) {
    lock.lock()
    byKind[texture.kind] = texture
    lock.unlock()
  }

  func get(_ kind: Int32) -> YinheTexture? {
    lock.lock()
    defer { lock.unlock() }
    return byKind[kind]
  }
}

/// C 回调：向宿主申请一帧可写纹理。
private func yinheAcquireTexture(
  kind: Int32, width: UInt32, height: UInt32, slot: UnsafeMutablePointer<Int32>?
) -> UnsafeMutableRawPointer? {
  guard let texture = YinheTextures.shared.get(kind) else { return nil }
  return texture.acquire(width: width, height: height, slot: slot)
}

/// C 回调：一帧画完。
private func yinheSubmitTexture(kind: Int32, slot: Int32) {
  YinheTextures.shared.get(kind)?.submit(slot: slot)
}
