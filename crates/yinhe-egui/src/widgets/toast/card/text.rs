use eframe::egui;

/// 量文本在给定宽度下的行数（memoized，重复调用便宜）。
pub(super) fn count_rows(
    ctx: &egui::Context,
    text: &str,
    font: &egui::FontId,
    wrap_w: f32,
) -> usize {
    if text.is_empty() {
        return 0;
    }
    ctx.fonts_mut(|f| {
        f.layout(text.to_string(), font.clone(), egui::Color32::WHITE, wrap_w)
            .rows
            .len()
    })
}

/// 文案截到至多 max_lines 行，超出加 …（egui Label 没有行数限制，手动量）。
/// 返回显示文本；空文本返回空串，由调用方决定占位还是跳过。
pub(super) fn clamp_lines(
    ctx: &egui::Context,
    text: &str,
    font: &egui::FontId,
    wrap_w: f32,
    max_lines: usize,
) -> String {
    if text.is_empty() || max_lines == 0 {
        return String::new();
    }
    if count_rows(ctx, text, font, wrap_w) <= max_lines {
        return text.to_string();
    }
    // 二分找最长前缀（+…后仍在行数内）
    let chars: Vec<char> = text.chars().collect();
    let mut lo = 0usize;
    let mut hi = chars.len();
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let cand: String = chars[..mid].iter().collect::<String>() + "…";
        if count_rows(ctx, &cand, font, wrap_w) <= max_lines {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo].iter().collect::<String>() + "…"
}

/// 固定占一行高度的空白：空文案也占位，保证进行中卡片高度不变。
/// 用 allocate（真 widget）而非 add_space，保证与真实行享有同样的 item_spacing。
pub(super) fn blank_line(ui: &mut egui::Ui, font: &egui::FontId, wrap_w: f32) {
    let h = ui.ctx().fonts_mut(|f| f.row_height(font));
    ui.allocate_exact_size(egui::vec2(wrap_w, h), egui::Sense::hover());
}
