#!/bin/sh
# 一键打包 macOS 版 Yinhe：cargo-bundle + 整体重签名 + 清理 xattr + 生成 dmg。
#
# 为什么需要重签名：
# cargo-bundle 生成的 .app 不会整体 codesign，二进制只有链接器(linker)的 adhoc 签名，
# 且 bundle 里可能混入下载文件时留下的 com.apple.quarantine 属性。这两个问题叠加，
# 会让 Gatekeeper 在别人电脑上（下载后带 quarantine 标记）校验失败，报"已损坏，无法打开"。
# 这里打包后强制整体重签名并清掉所有 xattr，让 codesign --verify 通过。
set -e

APP_NAME="Yinhe"
BUNDLE_DIR="target/release/bundle/osx"
APP="$BUNDLE_DIR/$APP_NAME.app"
DMG_DIR="target/release/bundle/dmg"
DMG="$DMG_DIR/$APP_NAME.dmg"
VOLUME="$APP_NAME"
STAGING_DIR="$DMG_DIR/.staging"

cargo build --release -p yinhe-egui
cargo bundle --release --format osx -p yinhe-egui

# 1) 整体重签名（--force 覆盖链接器的部分签名，--deep 递归签嵌套内容），
#    seal 上 Info.plist 和 Resources，使 codesign --verify 通过。
codesign --force --deep --sign - "$APP"

# 2) 清除 bundle 内所有 xattr（quarantine/macl/lastuseddate 等），
#    避免这些属性随 dmg 扩散到其他电脑触发 Gatekeeper。
xattr -cr "$APP"

# 3) 验证签名，失败即退出。
codesign --verify --deep --strict --verbose=2 "$APP"

# 4) 生成 dmg：与 cargo-bundle 的 dmg 布局一致（应用 + Applications 软链）。
rm -rf "$STAGING_DIR"
mkdir -p "$STAGING_DIR"
cp -R "$APP" "$STAGING_DIR/"
ln -s /Applications "$STAGING_DIR/Applications"
rm -f "$DMG"
hdiutil create -volname "$VOLUME" -srcfolder "$STAGING_DIR" -ov -format UDZO "$DMG"
rm -rf "$STAGING_DIR"

echo "打包完成: $DMG"
