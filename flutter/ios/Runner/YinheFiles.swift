import Foundation
import UIKit
import UniformTypeIdentifiers

/// 文件选择 / 保存的 iOS 实现。
///
/// 只做三件事：弹系统选择器、把文件拷进/拷出沙箱、记住写入位置（security-scoped
/// bookmark，等价于 Android 的持久化 URI 权限）。文件类型策略不在这里——扩展名
/// 白名单由 Dart 的 `core/platform_files.dart` 下发。
final class YinheFileBridge: NSObject, UIDocumentPickerDelegate {
  /// 用户可见的 Documents（导入的文件放这里，Files App 里能看到）。
  static let documentsDirectory: URL = FileManager.default.urls(
    for: .documentDirectory, in: .userDomainMask)[0]
  private let documents: URL
  /// 应用私有状态目录（最近文件/设置/音色库列表，别让用户看到）。
  static let stateDirectory: URL = {
    let fm = FileManager.default
    let base = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
    try? fm.createDirectory(at: base, withIntermediateDirectories: true)
    return base
  }()
  private let channel: FlutterMethodChannel

  /// 当前进行中的动作（delegate 回调据此决定回哪个通道消息）。
  private enum Action {
    case openProject
    case openSoundfont
    case saveProject(name: String)
    case saveExport(name: String)

    /// 大文件复制事件里的 `kind`。
    var copyKind: String {
      switch self {
      case .openProject: return "open"
      case .openSoundfont: return "soundfont"
      case .saveProject: return "saveProject"
      case .saveExport: return "saveExport"
      }
    }
  }

  private var action: Action?
  private var pendingExts: [String] = []

  /// 写入位置书签：文件名 → security-scoped bookmark（UserDefaults 持久化）。
  private let bookmarkKey = "yinhe.fileBookmarks"

  /// 大文件复制分块大小（1MB）与进度上报节流间隔（~100ms）。
  private static let copyChunkSize = 1 << 20
  private static let progressIntervalNanos: UInt64 = 100_000_000

  init(channel: FlutterMethodChannel) {
    self.channel = channel
    documents = Self.documentsDirectory
    super.init()
  }

  // MARK: - 通道入口

  func pickProject(exts: [String]) {
    pendingExts = exts
    action = .openProject
    presentOpen(allowed: exts)
  }

  func pickSoundfont(exts: [String]) {
    pendingExts = exts
    action = .openSoundfont
    presentOpen(allowed: exts)
  }

  func saveProject(name: String, saveAs: Bool) {
    let temp = tempURL(ext: "")
    // 不是「另存为」且记得上次位置：直接覆盖，不弹选择器。
    if !saveAs, let bookmark = bookmark(for: name) {
      let fileName = ensureExt(name, "yin")
      overwriteInBackground(
        bookmark: bookmark, with: temp, kind: "saveProject", name: fileName
      ) { result in
        switch result {
        case .success:
          self.send("onProjectSaved", "ok")
        case .failure(let error):
          NSLog("[yinhe] 覆盖保存失败：\(error)")
          self.removeBookmark(for: name)
          self.presentExport(file: temp, fileName: fileName, action: .saveProject(name: name))
        }
      }
      return
    }
    presentExport(
      file: temp, fileName: ensureExt(name, "yin"), action: .saveProject(name: name))
  }

  func saveExport(name: String, ext: String) {
    let temp = tempURL(ext: ext)
    let fileName = ext.isEmpty || name.lowercased().hasSuffix(".\(ext.lowercased())")
      ? name
      : "\(name).\(ext)"
    presentExport(file: temp, fileName: fileName, action: .saveExport(name: fileName))
  }

  func rememberProject(name: String) {
    // 书签已按文件名存好，重开最近文件后「保存」能直接覆盖；无需额外动作。
    _ = name
  }

  /// Rust 写临时文件用的路径（保存工程的没有扩展名，导出带扩展名）。
  func tempURL(ext: String) -> URL {
    let name = ext.isEmpty ? "export.yin" : "export_tmp.\(ext)"
    return FileManager.default.temporaryDirectory.appendingPathComponent(name)
  }

  // MARK: - 选择器

  private func presentOpen(allowed: [String]) {
    // 直接收全部数据文件，扩展名由我们自己按 Dart 下发的白名单校验
    // （系统 UTType 对 .mid/.yin 覆盖不全）。
    let picker = UIDocumentPickerViewController(forOpeningContentTypes: [.data], asCopy: false)
    picker.allowsMultipleSelection = false
    picker.delegate = self
    present(picker)
  }

  private func presentExport(file: URL, fileName: String, action: Action) {
    guard FileManager.default.fileExists(atPath: file.path) else {
      NSLog("[yinhe] 待保存的临时文件不存在：\(file.path)")
      if case .saveProject = action {
        send("onProjectSaveFailed", "temp missing")
      }
      return
    }
    // 导出页用文件 URL 当默认文件名：拷一份改成目标名再交给系统，
    // 原临时文件保留（Rust 下次还是写同一个路径，取消保存后不影响重试）。
    let named = file.deletingLastPathComponent().appendingPathComponent(fileName)
    if named == file {
      presentExportPicker(file: file, action: action)
      return
    }
    copyInBackground(from: file, to: named, kind: action.copyKind, name: fileName) { ok in
      if !ok {
        NSLog("[yinhe] 准备导出文件失败，改用原临时文件：\(file.path)")
      }
      let exportURL = FileManager.default.fileExists(atPath: named.path) ? named : file
      self.presentExportPicker(file: exportURL, action: action)
    }
  }

  private func presentExportPicker(file: URL, action: Action) {
    self.action = action
    let picker = UIDocumentPickerViewController(forExporting: [file], asCopy: true)
    picker.delegate = self
    present(picker)
  }

  private func present(_ picker: UIViewController) {
    DispatchQueue.main.async {
      guard let host = Self.topViewController() else {
        NSLog("[yinhe] 找不到可用的 UIViewController，选择器无法弹出")
        return
      }
      host.present(picker, animated: true)
    }
  }

  private static func topViewController() -> UIViewController? {
    let scene = UIApplication.shared.connectedScenes
      .compactMap { $0 as? UIWindowScene }
      .first { $0.activationState == .foregroundActive } ?? UIApplication.shared
      .connectedScenes.compactMap { $0 as? UIWindowScene }.first
    var vc = scene?.windows.first(where: { $0.isKeyWindow })?.rootViewController
      ?? scene?.windows.first?.rootViewController
    while let presented = vc?.presentedViewController {
      vc = presented
    }
    return vc
  }

  // MARK: - UIDocumentPickerDelegate

  func documentPicker(
    _ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]
  ) {
    guard let url = urls.first, let action else { return }
    self.action = nil
    switch action {
    case .openProject:
      handleOpen(url: url, soundfont: false)
    case .openSoundfont:
      handleOpen(url: url, soundfont: true)
    case .saveProject(let name):
      handleSavePicked(url: url, name: name, isProject: true)
    case .saveExport:
      handleSavePicked(url: url, name: url.lastPathComponent, isProject: false)
    }
  }

  func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
    guard let action else { return }
    self.action = nil
    if case .saveProject = action {
      send("onProjectSaveCancelled", "cancel")
    }
  }

  // MARK: - 打开

  private func handleOpen(url: URL, soundfont: Bool) {
    let name = url.lastPathComponent
    let ext = url.pathExtension.lowercased()
    guard !ext.isEmpty, pendingExts.contains(ext) else {
      send(soundfont ? "onSoundfontRejected" : "onFileRejected", name)
      return
    }
    // security scope 要一直持有到复制完成（以及 .yin 书签创建完），
    // 所以不能像以前那样用完就 defer stop。
    let scoped = url.startAccessingSecurityScopedResource()
    let dest = documents.appendingPathComponent(name.isEmpty ? "picked.bin" : name)
    let kind = soundfont ? "soundfont" : "open"

    if url.standardizedFileURL.path == dest.standardizedFileURL.path {
      // 用户可能就是从「我的 iPad > yinhe」（本应用 Documents）里选的，同一路径不要再拷。
      finishOpen(
        url: url, dest: dest, ext: ext, name: name, soundfont: soundfont, scoped: scoped)
      return
    }

    copyInBackground(from: url, to: dest, kind: kind, name: name) { ok in
      guard ok else {
        if scoped { url.stopAccessingSecurityScopedResource() }
        self.send(soundfont ? "onSoundfontRejected" : "onFileRejected", name)
        return
      }
      self.finishOpen(
        url: url, dest: dest, ext: ext, name: name, soundfont: soundfont, scoped: scoped)
    }
  }

  /// 复制完成后收尾（主线程）：记书签、释放 security scope、通知 Dart。
  private func finishOpen(
    url: URL, dest: URL, ext: String, name: String, soundfont: Bool, scoped: Bool
  ) {
    // .yin 工程记住原位置（bookmark），「保存」时直接覆盖原文件。
    if ext == "yin", scoped,
      let data = try? url.bookmarkData(
        options: [], includingResourceValuesForKeys: nil, relativeTo: nil)
    {
      saveBookmark(data, for: name)
    }
    if scoped { url.stopAccessingSecurityScopedResource() }
    send(soundfont ? "onSoundFontPicked" : "onFilePicked", dest.path)
  }

  // MARK: - 保存

  private func handleSavePicked(url: URL, name: String, isProject: Bool) {
    let scoped = url.startAccessingSecurityScopedResource()
    defer { if scoped { url.stopAccessingSecurityScopedResource() } }
    // 目标已存在时系统会提示替换；这里仍要保证能写。
    if scoped, let data = try? url.bookmarkData(
      options: [], includingResourceValuesForKeys: nil, relativeTo: nil)
    {
      saveBookmark(data, for: isProject ? name : url.lastPathComponent)
    }
    if isProject {
      send("onProjectSaved", "ok")
    } else {
      send("onExportSaved", url.lastPathComponent)
    }
  }

  // MARK: - 拷贝 / 覆盖

  /// 1MB 分块搬运，按 ~100ms 节流上报进度（在后台队列里跑）。
  private func streamCopy(
    from input: FileHandle, to output: FileHandle, total: Int64, kind: String, name: String
  ) throws {
    var done: Int64 = 0
    var reported: Int64 = -1
    var lastReport = DispatchTime.now()
    while let chunk = try input.read(upToCount: Self.copyChunkSize), !chunk.isEmpty {
      try output.write(contentsOf: chunk)
      done += Int64(chunk.count)
      let now = DispatchTime.now()
      if now.uptimeNanoseconds - lastReport.uptimeNanoseconds >= Self.progressIntervalNanos {
        lastReport = now
        reported = done
        emitCopyProgress(kind: kind, name: name, total: total, done: done)
      }
    }
    if done != reported {
      emitCopyProgress(kind: kind, name: name, total: total, done: done)
    }
  }

  /// 后台整文件复制（导入到沙箱 / 准备导出文件）。
  private func copyInBackground(
    from src: URL, to dst: URL, kind: String, name: String,
    completion: @escaping (Bool) -> Void
  ) {
    let total = fileSize(src)
    emitCopyStart(kind: kind, name: name, total: total)
    DispatchQueue.global(qos: .userInitiated).async {
      let fm = FileManager.default
      var ok = true
      do {
        if fm.fileExists(atPath: dst.path) {
          try fm.removeItem(at: dst)
        }
        guard fm.createFile(atPath: dst.path, contents: nil) else {
          throw NSError(
            domain: "yinhe", code: 3,
            userInfo: [NSLocalizedDescriptionKey: "无法创建目标文件"])
        }
        let input = try FileHandle(forReadingFrom: src)
        defer { try? input.close() }
        let output = try FileHandle(forWritingTo: dst)
        defer { try? output.close() }
        try self.streamCopy(from: input, to: output, total: total, kind: kind, name: name)
      } catch {
        NSLog("[yinhe] 复制文件失败：\(error)")
        ok = false
        try? fm.removeItem(at: dst)
      }
      DispatchQueue.main.async { completion(ok) }
    }
  }

  /// 用书签找回原写入位置并在后台覆盖（流式写，不整文件读进内存）。
  private func overwriteInBackground(
    bookmark: Data, with src: URL, kind: String, name: String,
    completion: @escaping (Result<Void, Error>) -> Void
  ) {
    let target: URL
    do {
      var stale = false
      target = try URL(
        resolvingBookmarkData: bookmark, options: [], relativeTo: nil,
        bookmarkDataIsStale: &stale)
    } catch {
      completion(.failure(error))
      return
    }
    guard target.startAccessingSecurityScopedResource() else {
      completion(
        .failure(
          NSError(
            domain: "yinhe", code: 1,
            userInfo: [NSLocalizedDescriptionKey: "无法访问上次的保存位置"])))
      return
    }
    guard FileManager.default.fileExists(atPath: target.path) else {
      target.stopAccessingSecurityScopedResource()
      completion(
        .failure(
          NSError(
            domain: "yinhe", code: 2,
            userInfo: [NSLocalizedDescriptionKey: "目标文件已被移动或删除"])))
      return
    }
    guard FileManager.default.fileExists(atPath: src.path) else {
      target.stopAccessingSecurityScopedResource()
      completion(
        .failure(
          NSError(
            domain: "yinhe", code: 4,
            userInfo: [NSLocalizedDescriptionKey: "待保存的临时文件不存在"])))
      return
    }
    let total = fileSize(src)
    emitCopyStart(kind: kind, name: name, total: total)
    DispatchQueue.global(qos: .userInitiated).async {
      var failure: Error?
      do {
        let input = try FileHandle(forReadingFrom: src)
        defer { try? input.close() }
        let output = try FileHandle(forWritingTo: target)
        defer { try? output.close() }
        try output.truncate(atOffset: 0)
        try self.streamCopy(from: input, to: output, total: total, kind: kind, name: name)
      } catch {
        failure = error
      }
      DispatchQueue.main.async {
        target.stopAccessingSecurityScopedResource()
        if let error = failure {
          completion(.failure(error))
        } else {
          completion(.success(()))
        }
      }
    }
  }

  private func fileSize(_ url: URL) -> Int64 {
    guard let attrs = try? FileManager.default.attributesOfItem(atPath: url.path),
      let size = attrs[.size] as? NSNumber
    else {
      return -1
    }
    return size.int64Value
  }

  // MARK: - 通道回调（统一切主线程）

  private func send(_ method: String, _ arguments: Any?) {
    if Thread.isMainThread {
      channel.invokeMethod(method, arguments: arguments)
    } else {
      DispatchQueue.main.async { [weak self] in
        self?.channel.invokeMethod(method, arguments: arguments)
      }
    }
  }

  private func emitCopyStart(kind: String, name: String, total: Int64) {
    send("onCopyStart", ["kind": kind, "name": name, "total": total])
  }

  private func emitCopyProgress(kind: String, name: String, total: Int64, done: Int64) {
    send("onCopyProgress", ["kind": kind, "name": name, "total": total, "done": done])
  }

  // MARK: - 书签存储

  private func bookmark(for name: String) -> Data? {
    let map = UserDefaults.standard.dictionary(forKey: bookmarkKey) as? [String: Data]
    return map?[name]
  }

  private func saveBookmark(_ data: Data, for name: String) {
    var map = (UserDefaults.standard.dictionary(forKey: bookmarkKey) as? [String: Data]) ?? [:]
    map[name] = data
    UserDefaults.standard.set(map, forKey: bookmarkKey)
  }

  private func removeBookmark(for name: String) {
    var map = (UserDefaults.standard.dictionary(forKey: bookmarkKey) as? [String: Data]) ?? [:]
    map.removeValue(forKey: name)
    UserDefaults.standard.set(map, forKey: bookmarkKey)
  }

  private func ensureExt(_ name: String, _ ext: String) -> String {
    name.lowercased().hasSuffix(".\(ext)") ? name : "\(name).\(ext)"
  }
}
