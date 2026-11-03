//! 工程封面加载（桌面/移动共用）。
//!
//! 只接受常见位图扩展名；不在导入时重新编码/缩放，保留用户原图质量。
//! 列表缩略图由后续 Library 层生成，`.yin` 里只存最终封面。

use std::path::Path;
use std::sync::Arc;

use yinhe_core::CoverArt;

pub fn cover_mime(path: &Path) -> Option<&'static str> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

pub fn load_cover(path: &Path) -> Result<CoverArt, String> {
    let mime = cover_mime(path).ok_or_else(|| "仅支持 JPG/PNG/WebP 封面".to_string())?;
    let data = std::fs::read(path).map_err(|e| format!("读取封面失败: {e}"))?;
    if data.is_empty() {
        return Err("封面文件为空".into());
    }
    Ok(CoverArt {
        mime: mime.to_string(),
        data: Arc::new(data),
        thumb: Arc::new(Vec::new()),
    })
}
