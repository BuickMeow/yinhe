import CoreVideo
import Flutter
import Metal

/// 一帧可渲染的外部纹理。
///
/// iOS 没有 Android `SurfaceProducer` 那种由系统缓冲的生产端 surface：Flutter
/// 的外接纹理就是「宿主提供 CVPixelBuffer，引擎来取」。这里持有 3 个
/// IOSurface 背书的 CVPixelBuffer（轮转，避免改写 Flutter 正在合成的那块），
/// 并把它们的 `MTLTexture` 交给 Rust（wgpu 零拷贝画进去）。
final class YinheTexture: NSObject, FlutterTexture {
  /// 0 = 主画布，1 = AM 面板（与 Rust `Cmd::EnsureExternal` 一致）。
  let kind: Int32

  private let registry: FlutterTextureRegistry
  private(set) var textureId: Int64 = -1

  private let lock = NSLock()
  private var width = 0
  private var height = 0
  private var cache: CVMetalTextureCache?
  private var pixelBuffers: [CVPixelBuffer] = []
  /// 必须持有 CVMetalTexture：它释放后底层 MTLTexture 就没了。
  private var cvTextures: [CVMetalTexture] = []
  private var metalTextures: [MTLTexture] = []
  /// 最近一次提交给 Flutter 的槽位（Flutter 可能还在读，不能改写）。
  private var latest = 0
  private var cursor = 0

  init(kind: Int32, registry: FlutterTextureRegistry) {
    self.kind = kind
    self.registry = registry
    super.init()
  }

  /// 注册到引擎（返回 Flutter 侧的 textureId）。
  func register() -> Int64 {
    if textureId < 0 {
      textureId = registry.register(self)
    }
    return textureId
  }

  // MARK: - Rust 侧回调

  /// 取一个可写槽位：返回它的 MTLTexture 指针，槽位下标写入 `slot`。
  func acquire(
    width w: UInt32,
    height h: UInt32,
    slot: UnsafeMutablePointer<Int32>?
  ) -> UnsafeMutableRawPointer? {
    lock.lock()
    defer { lock.unlock() }

    if Int(w) != width || Int(h) != height || pixelBuffers.isEmpty {
      guard rebuild(width: Int(w), height: Int(h)) else { return nil }
    }
    guard !metalTextures.isEmpty else { return nil }

    // 跳过 Flutter 当前持有的槽位，其余轮转。
    var picked = cursor
    for _ in 0..<metalTextures.count {
      if picked != latest { break }
      picked = (picked + 1) % metalTextures.count
    }
    cursor = (picked + 1) % metalTextures.count
    slot?.pointee = Int32(picked)
    return Unmanaged.passUnretained(metalTextures[picked]).toOpaque()
  }

  /// 提交：标记最新槽位并通知 Flutter 有新帧。
  func submit(slot: Int32) {
    lock.lock()
    if slot >= 0 && Int(slot) < pixelBuffers.count {
      latest = Int(slot)
    }
    lock.unlock()
    registry.textureFrameAvailable(textureId)
  }

  // MARK: - FlutterTexture

  func copyPixelBuffer() -> Unmanaged<CVPixelBuffer>? {
    lock.lock()
    defer { lock.unlock() }
    guard !pixelBuffers.isEmpty else { return nil }
    return Unmanaged.passRetained(pixelBuffers[latest])
  }

  // MARK: - 纹理池

  /// 按尺寸重建池；wgpu 设备还没就绪时返回 false（下一帧重试）。
  private func rebuild(width w: Int, height h: Int) -> Bool {
    guard w > 0, h > 0 else { return false }
    guard let devicePtr = yinhe_ios_metal_device() else { return false }
    let device =
      Unmanaged<AnyObject>.fromOpaque(devicePtr).takeUnretainedValue() as! MTLDevice

    if cache == nil {
      var created: CVMetalTextureCache?
      CVMetalTextureCacheCreate(kCFAllocatorDefault, nil, device, nil, &created)
      cache = created
    }
    guard let cache else { return false }

    let attrs: [CFString: Any] = [
      kCVPixelBufferPixelFormatTypeKey: kCVPixelFormatType_32BGRA,
      kCVPixelBufferWidthKey: w,
      kCVPixelBufferHeightKey: h,
      // IOSurface 背书 + Metal 兼容：CVMetalTextureCache 才能直接映射。
      kCVPixelBufferIOSurfacePropertiesKey: [:] as CFDictionary,
      kCVPixelBufferMetalCompatibilityKey: true,
    ]

    var newBuffers: [CVPixelBuffer] = []
    var newCv: [CVMetalTexture] = []
    var newMetal: [MTLTexture] = []
    for _ in 0..<3 {
      var buffer: CVPixelBuffer?
      let status = CVPixelBufferCreate(
        kCFAllocatorDefault, w, h, kCVPixelFormatType_32BGRA,
        attrs as CFDictionary, &buffer)
      guard status == kCVReturnSuccess, let buffer else { return false }

      var cv: CVMetalTexture?
      let cvStatus = CVMetalTextureCacheCreateTextureFromImage(
        kCFAllocatorDefault, cache, buffer, nil, .bgra8Unorm,
        w, h, 0, &cv)
      guard cvStatus == kCVReturnSuccess, let cv,
        let metal = CVMetalTextureGetTexture(cv)
      else { return false }

      newBuffers.append(buffer)
      newCv.append(cv)
      newMetal.append(metal)
    }

    pixelBuffers = newBuffers
    cvTextures = newCv
    metalTextures = newMetal
    width = w
    height = h
    latest = 0
    cursor = 1
    return true
  }
}
