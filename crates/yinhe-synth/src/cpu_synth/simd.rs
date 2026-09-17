//! fearless_simd 基建：运行时 SIMD 等级与跨平台向量数学。
//!
//! 设计要点：
//! - [`level`] 不做额外缓存：fearless_simd 内部在 x86 首次调用时探测 CPU
//!   并缓存（`LazyLock`），aarch64 恒为 NEON 基线，重复调用零探测开销；
//! - 向量数学与标量语义逐位对齐（parity 用）：包络指数曲线用与标量
//!   `powi(8)` 相同的平方求幂乘法链，且不使用 FMA（`mul_add` 只有一次
//!   舍入，会改变末位）。
//!
//! 用法：泛型内核 `fn kernel<S: Simd>(simd: S, ...)`，在渲染入口用
//! `dispatch!(level(), simd => kernel(simd, ...))` 分发到运行时最佳的
//! 指令集实现（SSE2/SSE4.2/AVX2/AVX-512/NEON/Fallback 各单态化一份）。

use fearless_simd::{Level, Simd, SimdBase};

/// 运行时最佳 SIMD 等级（x86 首次探测后缓存；aarch64 恒 NEON 基线）。
#[inline]
pub fn level() -> Level {
    Level::new()
}

/// `(1 - x)^8`：包络 Decay/Release 的指数曲线。
///
/// 平方求幂（x² → x⁴ → x⁸）与标量 `(1 - t).powi(8)` 的乘法链一致。
#[inline(always)]
pub fn powi8_neg<S: Simd>(simd: S, x: S::f32s) -> S::f32s {
    let base = S::f32s::splat(simd, 1.0) - x;
    let sq = base * base;
    let quad = sq * sq;
    quad * quad
}

#[cfg(test)]
mod tests {
    use super::*;
    use fearless_simd::dispatch;

    /// 标量参照：与 [`powi8_neg`] 相同的乘法链。
    fn powi8_neg_scalar(x: f32) -> f32 {
        let base = 1.0 - x;
        let sq = base * base;
        let quad = sq * sq;
        quad * quad
    }

    /// 分块应用（尾部标量），验证任意 runtime level 下与标量参照逐位一致。
    #[inline(always)]
    fn powi8_neg_slice<S: Simd>(simd: S, input: &[f32], out: &mut [f32]) {
        let width = S::f32s::LEN;
        let mut chunks = input.chunks_exact(width);
        let mut out_chunks = out.chunks_exact_mut(width);
        for (i, o) in (&mut chunks).zip(&mut out_chunks) {
            let v = S::f32s::from_slice(simd, i);
            powi8_neg(simd, v).store_slice(o);
        }
        for (i, o) in chunks.remainder().iter().zip(out_chunks.into_remainder()) {
            *o = powi8_neg_scalar(*i);
        }
    }

    #[test]
    fn powi8_matches_scalar_reference() {
        // 长度 65：跨越所有 native 宽度（4/8/16）的整块 + 尾部
        let input: Vec<f32> = (0..=64).map(|i| i as f32 / 64.0).collect();
        let mut out = vec![0.0f32; input.len()];
        let level = level();
        dispatch!(level, simd => powi8_neg_slice(simd, &input, &mut out));
        for (x, y) in input.iter().zip(&out) {
            assert_eq!(*y, powi8_neg_scalar(*x), "x={x}");
        }
    }
}
