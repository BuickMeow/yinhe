//! 非规格化数（denormal）抑制：x86/x86_64 上打开 MXCSR 的 FTZ/DAZ 位。
//!
//! 黑乐谱数千 voice 同时衰减到 -100dB 以下时，浮点运算结果落在非规格化
//! 范围会让 x86 标量/SSE 路径慢 10~100 倍（Arm 硬件无此惩罚，故不设置，
//! 也避免改变输出）。MXCSR 是 **per-thread** 状态，rayon worker 线程需要
//! 各自调用一次（幂等：已置位则直接返回，只读一次寄存器）。

/// 打开当前线程的 flush-to-zero / denormals-are-zero（x86/x86_64）。
/// 其他架构为 no-op（Arm 处理非规格化数接近全速，且改 FPCR 会轻微
/// 改变输出，不做）。
#[inline]
pub fn enable_flush_denormals() {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        #[cfg(target_arch = "x86")]
        use core::arch::x86::{_mm_getcsr, _mm_setcsr};
        #[cfg(target_arch = "x86_64")]
        use core::arch::x86_64::{_mm_getcsr, _mm_setcsr};
        // 15 = FTZ，6 = DAZ
        const WANT: u32 = (1 << 15) | (1 << 6);
        // SAFETY: 读写 MXCSR 无语义风险（仅影响浮点舍入/非规格化处理）；
        // 位 mask 均为合法保留位组合。
        unsafe {
            let csr = _mm_getcsr();
            if csr & WANT != WANT {
                _mm_setcsr(csr | WANT);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn sets_ftz_and_daz_bits() {
        #[cfg(target_arch = "x86")]
        use core::arch::x86::_mm_getcsr;
        #[cfg(target_arch = "x86_64")]
        use core::arch::x86_64::_mm_getcsr;
        super::enable_flush_denormals();
        // SAFETY: 只读 MXCSR。
        let csr = unsafe { _mm_getcsr() };
        assert_ne!(csr & (1 << 15), 0, "FTZ 应已置位");
        assert_ne!(csr & (1 << 6), 0, "DAZ 应已置位");
    }
}
