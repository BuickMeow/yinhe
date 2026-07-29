//! 数值输入组件 wrapper：中文句号「。」自动折算为小数点「.」。
//!
//! 用户在中文输入法下输入小数时，常误触句号「。」而非小数点「.」。
//! egui 的 `DragValue` 默认 parser 会把「。」当作非数字字符过滤掉，
//! 导致「3。14」被解析为「314」。本模块的 `decimal_parser` 在解析前
//! 先把「。」替换为「.」，再走 egui 默认的数值过滤逻辑。
//!
//! 用法：把 `egui::DragValue::new(value)` 替换为
//! `crate::widgets::numeric_input::decimal_drag_value(value)`，
//! 后续的 `.range(...)` / `.speed(...)` 等链式调用不变。

use eframe::egui;

/// 带中文句号折算的数值 parser，用于 `DragValue::custom_parser`。
///
/// 先把「。」替换为「.」，再过滤掉非数字字符（与 egui 默认 parser 一致），
/// 最后 `parse::<f64>`。
pub fn decimal_parser(s: &str) -> Option<f64> {
    let s: String = s.replace('。', ".");
    let s: String = s
        .chars()
        .filter(|c| {
            *c == '-' || *c == '+' || *c == '.' || *c == 'e' || *c == 'E' || c.is_ascii_digit()
        })
        .collect();
    s.parse().ok()
}

/// 创建带中文句号折算的 `DragValue`。
///
/// 等价于 `egui::DragValue::new(value).custom_parser(decimal_parser)`，
/// 后续可继续链式 `.range(...)` / `.speed(...)` / `.suffix(...)` 等。
pub fn decimal_drag_value<'a, Num: egui::emath::Numeric>(
    value: &'a mut Num,
) -> egui::DragValue<'a> {
    egui::DragValue::new(value).custom_parser(decimal_parser)
}
