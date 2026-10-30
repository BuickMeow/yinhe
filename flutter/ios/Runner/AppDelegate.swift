import AVFAudio
import Flutter
import UIKit

@main
@objc class AppDelegate: FlutterAppDelegate, FlutterImplicitEngineDelegate {
  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    // cpal 在 iOS 上需要显式把 AVAudioSession 设为 playback 并激活；
    // 默认 SoloAmbient 在部分设备/模拟器上不出声。
    let session = AVAudioSession.sharedInstance()
    try? session.setCategory(.playback, mode: .default)
    try? session.setActive(true)
    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }

  func didInitializeImplicitFlutterEngine(_ engineBridge: FlutterImplicitEngineBridge) {
    GeneratedPluginRegistrant.register(with: engineBridge.pluginRegistry)
    // 本仓库自带的平台壳（纹理 / 文件选择），不经过 pub 包。
    if let registrar = engineBridge.pluginRegistry.registrar(forPlugin: "YinhePlugin") {
      YinhePlugin.register(with: registrar)
    }
  }
}
