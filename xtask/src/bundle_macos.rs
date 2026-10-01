//! 打包 macOS 版 Yinhe：cargo-bundle + 整体重签名 + 清理 xattr + 生成 dmg。
//!
//! 为什么需要重签名：cargo-bundle 生成的 .app 不会整体 codesign，二进制只有
//! 链接器(linker)的 adhoc 签名，且 bundle 里可能混入下载文件时留下的
//! com.apple.quarantine 属性。两者叠加会让 Gatekeeper 在别人电脑上（下载后带
//! quarantine 标记）校验失败，报"已损坏，无法打开"。这里打包后强制整体重签名并
//! 清掉所有 xattr，让 codesign --verify 通过。

use std::fs;
use std::process::ExitCode;

use crate::cmd::{cargo, cmd, status};

const APP_NAME: &str = "Yinhe";
const BUNDLE_DIR: &str = "target/release/bundle/osx";
const DMG_DIR: &str = "target/release/bundle/dmg";
const STAGING_DIR: &str = "target/release/bundle/dmg/.staging";

/// CFBundleDocumentTypes：可打开的文档类型。
/// .yin 工程（自有 UTI，Owner）与 .mid/.midi（系统 UTI public.midi-audio，
/// Alternate——出现在"打开方式"但不抢占默认）。没有它 LaunchServices 不会把文件
/// 路由给本应用，Finder"打开方式"里也不出现 Yinhe，运行时 delegate 收不到事件。
const DOCUMENT_TYPES_JSON: &str = r#"[
  {
    "CFBundleTypeName": "Yinhe Project",
    "CFBundleTypeRole": "Editor",
    "LSHandlerRank": "Owner",
    "CFBundleTypeIconFile": "yinhe",
    "LSItemContentTypes": ["com.jieneng.yinhe.project"]
  },
  {
    "CFBundleTypeName": "MIDI File",
    "CFBundleTypeRole": "Editor",
    "LSHandlerRank": "Alternate",
    "CFBundleTypeIconFile": "yinhe",
    "LSItemContentTypes": ["public.midi-audio"]
  }
]"#;

/// 导出 .yin 的 UTI 声明（压缩容器，conformsTo public.data），
/// 让 LaunchServices 把 .yin 扩展名关联到 com.jieneng.yinhe.project。
const UTI_DECLARATIONS_JSON: &str = r#"[
  {
    "UTTypeIdentifier": "com.jieneng.yinhe.project",
    "UTTypeDescription": "Yinhe Project",
    "UTTypeIconFile": "yinhe",
    "UTTypeConformsTo": ["public.data"],
    "UTTypeTagSpecification": {
      "public.filename-extension": ["yin"]
    }
  }
]"#;

pub fn run() -> ExitCode {
    match run_inner() {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

fn run_inner() -> Result<(), ExitCode> {
    let app = format!("{BUNDLE_DIR}/{APP_NAME}.app");
    let dmg = format!("{DMG_DIR}/{APP_NAME}.dmg");
    let plist = format!("{app}/Contents/Info.plist");

    // 1) 构建 + cargo-bundle。
    status(cargo().args(["build", "--release", "-p", "yinhe-egui"]))?;
    status(cargo().args(["bundle", "--release", "--format", "osx", "-p", "yinhe-egui"]))?;

    // 2) 声明本地化：让系统 UI（打开/保存面板、红绿灯悬停菜单等）跟随系统语言。
    //    不声明的话 AppKit 认为应用只支持英文，中文系统上也显示英文。
    //    CFBundleDevelopmentRegion 作为无法匹配任何本地化时的 fallback。
    status(
        cmd("/usr/libexec/PlistBuddy")
            .args(["-c", "Add :CFBundleLocalizations array"])
            .args(["-c", "Add :CFBundleLocalizations:0 string zh-Hans"])
            .args(["-c", "Add :CFBundleLocalizations:1 string en"])
            .args(["-c", "Add :CFBundleLocalizations:2 string ja"])
            .args(["-c", "Add :CFBundleLocalizations:3 string ko"])
            .args(["-c", "Set :CFBundleDevelopmentRegion zh-Hans"])
            .arg(&plist),
    )?;

    // 3) .lproj 兜底：部分系统组件只认 Resources 下的 .lproj 目录，目录存在即可
    //    让 CFBundle 的 knownLocalizations 包含该语言。
    for lang in ["zh-Hans", "en", "ja", "ko"] {
        let dir = format!("{app}/Contents/Resources/{lang}.lproj");
        fs::create_dir_all(&dir).map_err(|e| report_io("创建 .lproj 目录", &dir, &e))?;
        let strings = format!("{dir}/InfoPlist.strings");
        fs::File::create(&strings)
            .map_err(|e| report_io("创建 InfoPlist.strings", &strings, &e))?;
    }

    // 4) 声明文档类型与导出的 UTI（用 plutil -insert -json：PlistBuddy 批量 -c
    //    指令会 Abort trap）。
    status(
        cmd("plutil")
            .args([
                "-insert",
                "CFBundleDocumentTypes",
                "-json",
                DOCUMENT_TYPES_JSON,
            ])
            .arg(&plist),
    )?;
    status(
        cmd("plutil")
            .args([
                "-insert",
                "UTExportedTypeDeclarations",
                "-json",
                UTI_DECLARATIONS_JSON,
            ])
            .arg(&plist),
    )?;

    // 5) 整体重签名（--force 覆盖链接器的部分签名，--deep 递归签嵌套内容），
    //    seal 上 Info.plist 和 Resources，使 codesign --verify 通过。
    status(
        cmd("codesign")
            .args(["--force", "--deep", "--sign", "-"])
            .arg(&app),
    )?;

    // 6) 清除 bundle 内所有 xattr（quarantine/macl/lastuseddate 等），
    //    避免这些属性随 dmg 扩散到其他电脑触发 Gatekeeper。
    status(cmd("xattr").args(["-cr"]).arg(&app))?;

    // 7) 验证签名，失败即退出。
    status(
        cmd("codesign")
            .args(["--verify", "--deep", "--strict", "--verbose=2"])
            .arg(&app),
    )?;

    // 8) 生成 dmg：与 cargo-bundle 的 dmg 布局一致（应用 + Applications 软链）。
    let staging = std::path::Path::new(STAGING_DIR);
    if staging.exists() {
        fs::remove_dir_all(staging).map_err(|e| report_io("清理 staging", STAGING_DIR, &e))?;
    }
    fs::create_dir_all(staging).map_err(|e| report_io("创建 staging", STAGING_DIR, &e))?;
    let staged_app = format!("{STAGING_DIR}/{APP_NAME}.app");
    status(cmd("cp").args(["-R"]).arg(&app).arg(&staged_app))?;
    let staged_link = format!("{STAGING_DIR}/Applications");
    status(cmd("ln").args(["-s", "/Applications"]).arg(&staged_link))?;

    let _ = fs::remove_file(&dmg);
    status(
        cmd("hdiutil")
            .args([
                "create",
                "-volname",
                APP_NAME,
                "-srcfolder",
                STAGING_DIR,
                "-ov",
                "-format",
                "UDZO",
            ])
            .arg(&dmg),
    )?;
    let _ = fs::remove_dir_all(staging);

    println!("打包完成: {dmg}");
    Ok(())
}

fn report_io(what: &str, path: &str, e: &std::io::Error) -> ExitCode {
    eprintln!("{what} 失败 ({path}): {e}");
    ExitCode::FAILURE
}
