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
  }

  private var action: Action?
  private var pendingExts: [String] = []

  /// 写入位置书签：文件名 → security-scoped bookmark（UserDefaults 持久化）。
  private let bookmarkKey = "yinhe.fileBookmarks"

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
      do {
        try overwrite(bookmark: bookmark, with: temp)
        channel.invokeMethod("onProjectSaved", arguments: "ok")
      } catch {
        NSLog("[yinhe] 覆盖保存失败：\(error)")
        removeBookmark(for: name)
        presentExport(file: temp, fileName: ensureExt(name, "yin"), action: .saveProject(name: name))
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
        channel.invokeMethod("onProjectSaveFailed", arguments: "temp missing")
      }
      return
    }
    // 导出页用文件 URL 当默认文件名：拷一份改成目标名再交给系统，
    // 原临时文件保留（Rust 下次还是写同一个路径，取消保存后不影响重试）。
    let named = file.deletingLastPathComponent().appendingPathComponent(fileName)
    if named != file {
      try? FileManager.default.removeItem(at: named)
      do {
        try FileManager.default.copyItem(at: file, to: named)
      } catch {
        NSLog("[yinhe] 准备导出文件失败：\(error)")
      }
    }
    let exportURL = FileManager.default.fileExists(atPath: named.path) ? named : file
    self.action = action
    let picker = UIDocumentPickerViewController(forExporting: [exportURL], asCopy: true)
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
      channel.invokeMethod("onProjectSaveCancelled", arguments: "cancel")
    }
  }

  // MARK: - 打开

  private func handleOpen(url: URL, soundfont: Bool) {
    let name = url.lastPathComponent
    let ext = url.pathExtension.lowercased()
    guard !ext.isEmpty, pendingExts.contains(ext) else {
      channel.invokeMethod(soundfont ? "onSoundfontRejected" : "onFileRejected", arguments: name)
      return
    }
    let scoped = url.startAccessingSecurityScopedResource()
    defer { if scoped { url.stopAccessingSecurityScopedResource() } }

    let dest = documents.appendingPathComponent(name.isEmpty ? "picked.bin" : name)
    do {
      if url.standardizedFileURL.path != dest.standardizedFileURL.path {
        // 用户可能就是从「我的 iPad > yinhe」（本应用 Documents）里选的，同一路径不要再拷。
        try copy(from: url, to: dest)
      }
    } catch {
      NSLog("[yinhe] 复制所选文件失败：\(error)")
      channel.invokeMethod(soundfont ? "onSoundfontRejected" : "onFileRejected", arguments: name)
      return
    }

    // .yin 工程记住原位置（bookmark），「保存」时直接覆盖原文件。
    if ext == "yin", scoped, let data = try? url.bookmarkData(
      options: [], includingResourceValuesForKeys: nil, relativeTo: nil)
    {
      saveBookmark(data, for: name)
    }

    channel.invokeMethod(soundfont ? "onSoundFontPicked" : "onFilePicked", arguments: dest.path)
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
      channel.invokeMethod("onProjectSaved", arguments: "ok")
    } else {
      channel.invokeMethod("onExportSaved", arguments: url.lastPathComponent)
    }
  }

  // MARK: - 拷贝 / 覆盖

  private func copy(from src: URL, to dst: URL) throws {
    let fm = FileManager.default
    if fm.fileExists(atPath: dst.path) {
      try fm.removeItem(at: dst)
    }
    try fm.copyItem(at: src, to: dst)
  }

  /// 用书签找回原写入位置并覆盖（流式写，不整文件读进内存）。
  private func overwrite(bookmark: Data, with src: URL) throws {
    var stale = false
    let url = try URL(
      resolvingBookmarkData: bookmark, options: [], relativeTo: nil,
      bookmarkDataIsStale: &stale)
    guard url.startAccessingSecurityScopedResource() else {
      throw NSError(
        domain: "yinhe", code: 1,
        userInfo: [NSLocalizedDescriptionKey: "无法访问上次的保存位置"])
    }
    defer { url.stopAccessingSecurityScopedResource() }
    guard FileManager.default.fileExists(atPath: url.path) else {
      throw NSError(
        domain: "yinhe", code: 2,
        userInfo: [NSLocalizedDescriptionKey: "目标文件已被移动或删除"])
    }
    let input = try FileHandle(forReadingFrom: src)
    defer { try? input.close() }
    let output = try FileHandle(forWritingTo: url)
    defer { try? output.close() }
    try output.truncate(atOffset: 0)
    while let chunk = try input.read(upToCount: 1 << 20), !chunk.isEmpty {
      try output.write(contentsOf: chunk)
    }
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
