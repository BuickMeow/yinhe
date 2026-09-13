//! VST3 factory 枚举：厂商信息与音频处理器类。
//!
//! 基础字段来自 `IPluginFactory::getClassInfo`（`PClassInfo`）；版本/子类别在
//! `IPluginFactory2::getClassInfo2`，Unicode 名称/版本在
//! `IPluginFactory3::getClassInfoUnicode`——插件可能只实现旧接口，逐级回退。

use vst3::ComRef;
use vst3::Steinberg::{
    IPluginFactory, IPluginFactory2, IPluginFactory2Trait, IPluginFactory3, IPluginFactory3Trait,
    IPluginFactoryTrait, PClassInfo, PClassInfo2, PClassInfoW, PFactoryInfo, kResultOk,
};

/// factory 厂商信息。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FactoryInfo {
    pub vendor: String,
    pub url: String,
    pub email: String,
}

/// factory 导出的一个音频处理器类。
#[derive(Clone, Debug, PartialEq)]
pub struct FactoryClass {
    /// canonical 32 位大写十六进制 class id（与 moduleinfo.json 的 CID 同格式）。
    pub class_id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    /// 子类别（"Instrument" / "Fx" / "Synth" 等）；`PClassInfo2` 里由 `|` 分隔。
    pub sub_categories: Vec<String>,
    pub is_instrument: bool,
    pub is_effect: bool,
}

/// 读取 factory 厂商信息。
pub fn factory_info(factory: &ComRef<IPluginFactory>) -> FactoryInfo {
    unsafe {
        let mut info: PFactoryInfo = std::mem::zeroed();
        // 基础接口方法不返回失败语义时也保证 info 有效（zeroed 兜底）。
        let _ = factory.getFactoryInfo(&mut info);
        FactoryInfo {
            vendor: c_str(&info.vendor),
            url: c_str(&info.url),
            email: c_str(&info.email),
        }
    }
}

/// 枚举 factory 的全部**音频处理器类**（过滤 Component Controller 等辅助类）。
pub fn enumerate_classes(factory: &ComRef<IPluginFactory>) -> Vec<FactoryClass> {
    unsafe {
        let count = factory.countClasses();
        let factory2 = factory.cast::<IPluginFactory2>();
        let factory3 = factory.cast::<IPluginFactory3>();
        let mut out = Vec::new();
        for index in 0..count {
            let mut base: PClassInfo = std::mem::zeroed();
            if factory.getClassInfo(index, &mut base) != kResultOk {
                continue;
            }
            let category = c_str(&base.category);
            if !category.contains("Audio Module Class") {
                continue;
            }
            let mut name = c_str(&base.name);
            let mut version = String::new();
            let mut vendor = String::new();
            let mut sub_categories: Vec<String> = Vec::new();

            if let Some(f2) = &factory2 {
                let mut info: PClassInfo2 = std::mem::zeroed();
                if f2.getClassInfo2(index, &mut info) == kResultOk {
                    version = c_str(&info.version);
                    vendor = c_str(&info.vendor);
                    sub_categories = split_sub_categories(&c_str(&info.subCategories));
                }
            }
            if let Some(f3) = &factory3 {
                let mut info: PClassInfoW = std::mem::zeroed();
                if f3.getClassInfoUnicode(index, &mut info) == kResultOk {
                    let unicode_name = utf16_str(&info.name);
                    if !unicode_name.is_empty() {
                        name = unicode_name;
                    }
                    let unicode_version = utf16_str(&info.version);
                    if !unicode_version.is_empty() {
                        version = unicode_version;
                    }
                    let unicode_vendor = utf16_str(&info.vendor);
                    if !unicode_vendor.is_empty() {
                        vendor = unicode_vendor;
                    }
                    let unicode_cats = c_str(&info.subCategories);
                    if !unicode_cats.is_empty() {
                        sub_categories = split_sub_categories(&unicode_cats);
                    }
                }
            }

            out.push(FactoryClass {
                class_id: format_uid(&base.cid),
                name,
                vendor,
                version,
                is_instrument: sub_categories.iter().any(|s| s == "Instrument"),
                is_effect: sub_categories.iter().any(|s| s == "Fx"),
                sub_categories,
            });
        }
        out
    }
}

/// `c_char`（i8）定长缓冲 → String（截断到首个 NUL）。
fn c_str(raw: &[std::ffi::c_char]) -> String {
    let end = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
    let bytes: Vec<u8> = raw[..end].iter().map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// UTF-16 定长缓冲 → String（截断到首个 NUL）。
fn utf16_str(raw: &[u16]) -> String {
    let end = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
    String::from_utf16_lossy(&raw[..end])
}

/// `|` 分隔的子类别列表。
fn split_sub_categories(raw: &str) -> Vec<String> {
    raw.split('|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// TUID 字节 → canonical 32 位大写十六进制文本。
///
/// Windows 上 GUID 前三个字段以 COM 小端序存储，需按规范还原为 canonical 顺序，
/// 否则与其他平台/preset 文本不一致。
fn format_uid(cid: &[std::ffi::c_char; 16]) -> String {
    const COM_TO_CANONICAL: [usize; 16] = [3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15];
    const IDENTITY: [usize; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
    let order = if cfg!(target_os = "windows") {
        COM_TO_CANONICAL
    } else {
        IDENTITY
    };
    order
        .iter()
        .map(|&i| format!("{:02X}", cid[i] as u8))
        .collect()
}

/// canonical 32 位十六进制文本 → TUID 字节（[`format_uid`] 的逆操作）。
///
/// 置换表自逆：Windows 上 COM 顺序与 canonical 顺序互为同一次字段交换。
pub(crate) fn parse_uid(text: &str) -> Option<[std::ffi::c_char; 16]> {
    if text.len() != 32 {
        return None;
    }
    let mut canonical = [0u8; 16];
    for (i, byte) in canonical.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    const COM_TO_CANONICAL: [usize; 16] = [3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15];
    const IDENTITY: [usize; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
    let order = if cfg!(target_os = "windows") {
        COM_TO_CANONICAL
    } else {
        IDENTITY
    };
    let mut tuid = [0i8; 16];
    for (i, &src) in order.iter().enumerate() {
        tuid[i] = canonical[src] as i8;
    }
    Some(tuid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_categories_split_on_pipe() {
        assert_eq!(
            split_sub_categories("Instrument|Synth"),
            vec!["Instrument", "Synth"]
        );
        assert_eq!(split_sub_categories("Fx"), vec!["Fx"]);
        assert!(split_sub_categories("").is_empty());
        assert_eq!(split_sub_categories("Fx| |"), vec!["Fx"]);
    }

    #[test]
    fn uid_format_is_32_uppercase_hex() {
        let cid: [std::ffi::c_char; 16] = [
            0x56, 0x53, 0x45, 0x58, 0x66, 0x73, 0x50, 0x73, 0x65, 0x72, 0x75, 0x6D, 0x20, 0x32,
            0x00, 0x00,
        ];
        let text = format_uid(&cid);
        assert_eq!(text.len(), 32);
        assert!(
            text.chars()
                .all(|c| c.is_ascii_hexdigit() || c.is_ascii_uppercase())
        );
        assert_eq!(&text[0..8], "56534558");
    }

    #[test]
    fn c_str_truncates_at_nul() {
        let raw: [std::ffi::c_char; 6] = [b'h' as i8, b'i' as i8, 0, b'x' as i8, 0, 0];
        assert_eq!(c_str(&raw), "hi");
    }
}
