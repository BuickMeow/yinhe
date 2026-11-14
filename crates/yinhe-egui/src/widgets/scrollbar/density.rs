//! 滚动条密度预览：纹理缓存与像素填充。

use eframe::egui;
use yinhe_editor_core::density::{NO_TRACK, NoteDensityView};

use crate::theme;

/// 密度预览纹理缓存（每滚动条实例一份，存于 egui Context 持久数据）。
///
/// 失效判定四要素：数据版本 / 纹理尺寸 / 轨道配色。任一变化即重建像素
/// 并更新纹理——切显隐（版本变）、改颜色（配色变）、窗口 resize 与多窗口
/// 不同尺寸（w/h 变）都被覆盖。稳态（无变化）每帧只画 1 个 image。
#[derive(Clone, Default)]
struct DensityTextureCache {
    handle: Option<egui::TextureHandle>,
    version: u64,
    w: usize,
    h: usize,
    /// 创建纹理时的缩放因子：HiDPI 下纹理必须按物理像素建，否则会糊。
    ppp: f32,
    colors: Vec<[f32; 4]>,
}

/// 密度预览（纹理化）：把像素网格渲染进纹理，只在失效判定命中时重建。
///
/// 纹理尺寸 = 该滚动条当前的实际像素网格（`ceil(主轴长) × 副轴行数`，
/// 纵向转置），不存在"缓存了过期尺寸"的情况；resize 拖动期间每帧重建
/// 一次（约 1ms 构建），松手后归零。
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_density_image(
    ui: &egui::Ui,
    rect: egui::Rect,
    vertical: bool,
    sb_w: f32,
    px_rows: usize,
    scale: f64,
    d: &NoteDensityView<'_>,
    track_colors: &[[f32; 4]],
) {
    // HiDPI：纹理尺寸按**物理像素**建。egui 的 painter.image 会把纹理铺到
    // 逻辑尺寸 rect 上；若纹理仍按逻辑像素建，2x 屏会直接把图放大 2 倍变糊。
    let ppp = ui.ctx().pixels_per_point().max(0.5);
    let physical_main = (sb_w * ppp).round().max(1.0) as usize;
    let physical_rows = (px_rows as f32 * ppp).round().max(1.0) as usize;
    let (w, h) = if vertical {
        (physical_rows, physical_main)
    } else {
        (physical_main, physical_rows)
    };
    // 主轴物理像素 → tick 的比例（用取整后的 physical_main 反推，
    // 避免 ppp 非整数时最后一列错位）。
    let scale_px = scale * (physical_main as f64 / sb_w.max(1.0) as f64);
    let id = ui.scope_id().with("__sb_density_tex__");
    let need_rebuild = ui.ctx().data_mut(|data| {
        let cache = data.get_persisted_mut_or(id, DensityTextureCache::default());
        if cache.version != d.version
            || cache.w != w
            || cache.h != h
            || cache.ppp != ppp
            || cache.colors != track_colors
        {
            cache.version = d.version;
            cache.w = w;
            cache.h = h;
            cache.ppp = ppp;
            cache.colors.clear();
            cache.colors.extend_from_slice(track_colors);
            true
        } else {
            false
        }
    });
    if need_rebuild {
        let mut pixels = vec![egui::Color32::TRANSPARENT; w * h];
        fill_density_pixels(
            &mut pixels,
            w,
            h,
            vertical,
            sb_w,
            physical_rows,
            scale_px,
            d,
            track_colors,
        );
        let image = egui::ColorImage::new([w, h], pixels);
        // 已有纹理就地更新（避免反复创建/销毁 GPU 纹理），否则新建。
        let existing = ui.ctx().data_mut(|data| {
            data.get_persisted_mut_or(id, DensityTextureCache::default())
                .handle
                .clone()
        });
        match existing {
            Some(mut handle) => handle.set(image, egui::TextureOptions::LINEAR),
            None => {
                let handle =
                    ui.ctx()
                        .load_texture("density_preview", image, egui::TextureOptions::NEAREST);
                ui.ctx().data_mut(|data| {
                    data.get_persisted_mut_or(id, DensityTextureCache::default())
                        .handle = Some(handle)
                });
            }
        }
    }
    let tex = ui.ctx().data_mut(|data| {
        data.get_persisted_mut_or(id, DensityTextureCache::default())
            .handle
            .as_ref()
            .map(|h| h.id())
    });
    if let Some(tex) = tex {
        ui.painter().image(
            tex,
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }
}

/// 填充密度纹理像素（premultiplied RGBA，`w × h` 对应**物理**像素网格）。
///
/// 横向：x = 主轴列（左→右），y = 副轴行（顶 = 高音，底 = 低音）。
/// 纵向：x = 副轴行（左 = 低音），y = 主轴列（时间轴镜像：内容起点在底部）。
///
/// 每列按与桶的重叠权重累加计数：桶比列宽时该桶铺满其覆盖的所有列
/// （旧实现把桶只画到"跨越桶边界"的那一列，其余列留空 → 周期性虚线），
/// 列比桶宽时列内所有桶按重叠比例累加。权重除以两者中较小的跨度，
/// 保证整桶/整列都按 1 计，亮度不随缩放变化。
///
/// 每格取格子内计数最大的 `(key, 桶)` 的主导轨色（与 AR 画布轨色一致），
/// 计数经 ln 压缩映射到 alpha；0 计数留透明。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub(super) fn fill_density_pixels(
    out: &mut [egui::Color32],
    w: usize,
    h: usize,
    vertical: bool,
    sb_w: f32,
    px_rows: usize,
    scale: f64,
    d: &NoteDensityView<'_>,
    track_colors: &[[f32; 4]],
) {
    let n = d.n_buckets;
    if n == 0
        || d.n_keys == 0
        || px_rows == 0
        || scale <= 0.0
        || sb_w <= 0.0
        || d.rows.len() < d.n_keys * n
        || d.dominant.len() < d.n_keys * n
    {
        return;
    }
    let bucket = d.bucket_ticks.max(1) as f64;
    let inv_scale = 1.0 / scale; // 主轴像素 → tick
    let max_ln = (d.max_count as f32).ln_1p().max(f32::EPSILON);
    let fallback = theme::accent_active();
    let cols = if vertical { h } else { w }; // 主轴像素数
    // 每列覆盖的桶区间与重叠权重（复用缓冲）。
    let mut weights: Vec<f64> = Vec::new();
    for col in 0..cols {
        let t0 = col as f64 * inv_scale;
        let t1 = (col + 1) as f64 * inv_scale;
        let b0 = (t0 / bucket) as usize;
        if b0 >= n {
            break; // 后续列的 tick 更大，全部在桶覆盖范围外
        }
        let b1 = ((t1 / bucket).ceil() as usize).clamp(b0 + 1, n);
        // 桶宽与列宽取较小者作分母：整桶或整列都按 1 计。
        let unit = bucket.min(t1 - t0).max(f64::EPSILON);
        weights.clear();
        weights.extend((b0..b1).map(|i| {
            let (bs, be) = (i as f64 * bucket, (i + 1) as f64 * bucket);
            ((t1.min(be) - t0.max(bs)) / unit).clamp(0.0, 1.0)
        }));
        for r in 0..px_rows {
            let k0 = r * d.n_keys / px_rows;
            let k1 = ((r + 1) * d.n_keys / px_rows).min(d.n_keys);
            if k1 <= k0 {
                continue;
            }
            let mut count = 0.0f64;
            let mut best_cell = 0u32;
            let mut best_track = NO_TRACK;
            for k in k0..k1 {
                let base = k * n;
                for (wi, &weight) in weights.iter().enumerate() {
                    let c = d.rows[base + b0 + wi];
                    count += c as f64 * weight;
                    if c > best_cell {
                        best_cell = c;
                        best_track = d.dominant[base + b0 + wi];
                    }
                }
            }
            if count <= 0.0 {
                continue;
            }
            let color = track_color(best_track, track_colors).unwrap_or(fallback);
            // ln 压缩；一个像素行聚合多个 key，允许超过单格 max（clamp 上限）。
            let a = (count as f32).ln_1p() / max_ln;
            let alpha = (0.28 + 0.62 * a).min(0.95);
            let px = color.gamma_multiply(alpha);
            let idx = if vertical {
                // 内容 col=0 在屏幕底部（时间轴镜像），低音行 r 在左端。
                (h - 1 - col) * w + r
            } else {
                (px_rows - 1 - r) * w + col
            };
            out[idx] = px;
        }
    }
}

/// 主导轨对应的 egui 颜色（`[f32;4]` RGBA → rgb 部分）。
fn track_color(track: u32, colors: &[[f32; 4]]) -> Option<egui::Color32> {
    if track == NO_TRACK {
        return None;
    }
    let c = colors.get(track as usize)?;
    Some(egui::Color32::from_rgb(
        (c[0].clamp(0.0, 1.0) * 255.0) as u8,
        (c[1].clamp(0.0, 1.0) * 255.0) as u8,
        (c[2].clamp(0.0, 1.0) * 255.0) as u8,
    ))
}
