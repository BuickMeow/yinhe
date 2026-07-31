//! 数值表达式解析器：批量编辑输入框共用。
//!
//! 支持一行链式运算，例如：
//! - `100`    → 赋值（Set 100）
//! - `+2`     → 加 2
//! - `-2`     → 减 2（等价于加 -2）
//! - `x2` / `*2` / `×2` → 乘以 2
//! - `/2` / `÷2` → 除以 2
//! - `20%` / `x.2` → 乘以 0.2（百分比是乘法因子）
//! - `x3/7`   → 先乘 3 再除 7（链式）
//! - `3x2`    → 先赋值 3，再乘 2
//!
//! 语法规则：
//! - 第一个 token 以数字开头 → 赋值（Set）；带 `%` 后缀 → 乘法因子
//! - 后续 token 必须以运算符开头（`+` `-` `x` `*` `×` `/` `÷`）
//! - `%` 后缀仅对空运算符或乘号合法（`20%` = ×0.2，`+20%` 无效）
//! - 除以 0 无效

/// 数值运算操作，按输入顺序链式应用。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NumOp {
    /// 赋值（忽略当前值）。
    Set(f64),
    /// 加（负数即减）。
    Add(f64),
    /// 乘。
    Mul(f64),
    /// 除（除数在解析时已保证非 0）。
    Div(f64),
}

impl NumOp {
    /// 应用单个操作。
    pub fn apply(&self, v: f64) -> f64 {
        match self {
            NumOp::Set(n) => *n,
            NumOp::Add(n) => v + n,
            NumOp::Mul(n) => v * n,
            NumOp::Div(n) => v / n,
        }
    }
}

/// 把 ops 链式应用到 `v` 上。
pub fn apply_ops(ops: &[NumOp], v: f64) -> f64 {
    ops.iter().fold(v, |acc, op| op.apply(acc))
}

/// 链式应用后四舍五入到整数。
pub fn apply_ops_round(ops: &[NumOp], v: f64) -> f64 {
    apply_ops(ops, v).round()
}

/// 解析表达式字符串。
///
/// 返回 `None` 表示表达式无效（空串、缺数字、除 0、`%` 位置非法等）。
pub fn parse_num_expr(input: &str) -> Option<Vec<NumOp>> {
    // 中文输入法句号「。」折算为小数点，与 numeric_input 一致。
    let s: String = input.trim().replace('。', ".");
    if s.is_empty() {
        return None;
    }
    let chars: Vec<char> = s.chars().collect();
    let mut ops: Vec<NumOp> = Vec::new();
    let mut i = 0;
    let mut first = true;
    while i < chars.len() {
        // ── 读运算符（可选） ──
        // 用 Option<u8> 区分「无运算符」与「运算符种类」：
        // 0 = 加，1 = 减，2 = 乘，3 = 除。
        let op_kind: Option<u8> = match chars[i] {
            '+' => Some(0),
            '-' => Some(1),
            'x' | 'X' | '*' | '×' => Some(2),
            '/' | '÷' => Some(3),
            _ => None,
        };
        if op_kind.is_some() {
            i += 1;
        }

        // ── 读数字 ──
        let start = i;
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
            i += 1;
        }
        if start == i {
            return None; // 运算符后缺数字
        }
        let num: f64 = chars[start..i].iter().collect::<String>().parse().ok()?;

        // ── 读 % 后缀 ──
        let is_pct = i < chars.len() && chars[i] == '%';
        if is_pct {
            i += 1;
        }

        let op = match op_kind {
            Some(0) => {
                if is_pct {
                    return None;
                }
                NumOp::Add(num)
            }
            Some(1) => {
                if is_pct {
                    return None;
                }
                NumOp::Add(-num)
            }
            Some(2) => NumOp::Mul(if is_pct { num / 100.0 } else { num }),
            Some(3) => {
                if is_pct || num == 0.0 {
                    return None; // 除 0 无效
                }
                NumOp::Div(num)
            }
            Some(_) => unreachable!(), // 运算符种类只有 0..=3
            None => {
                if first {
                    // 首 token：数字 = 赋值；带 % = 乘法因子
                    if is_pct {
                        NumOp::Mul(num / 100.0)
                    } else {
                        NumOp::Set(num)
                    }
                } else {
                    return None; // 后续 token 必须以运算符开头
                }
            }
        };
        ops.push(op);
        first = false;
    }
    Some(ops)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_assign() {
        assert_eq!(parse_num_expr("100"), Some(vec![NumOp::Set(100.0)]));
        assert_eq!(parse_num_expr("0"), Some(vec![NumOp::Set(0.0)]));
        assert_eq!(parse_num_expr("-0"), Some(vec![NumOp::Add(-0.0)])); // 减号开头是加减法
    }

    #[test]
    fn parse_add_sub() {
        assert_eq!(parse_num_expr("+2"), Some(vec![NumOp::Add(2.0)]));
        assert_eq!(parse_num_expr("-2"), Some(vec![NumOp::Add(-2.0)]));
        assert_eq!(
            parse_num_expr("+5-2"),
            Some(vec![NumOp::Add(5.0), NumOp::Add(-2.0)])
        );
    }

    #[test]
    fn parse_mul_div() {
        assert_eq!(parse_num_expr("x2"), Some(vec![NumOp::Mul(2.0)]));
        assert_eq!(parse_num_expr("*2"), Some(vec![NumOp::Mul(2.0)]));
        assert_eq!(parse_num_expr("×2"), Some(vec![NumOp::Mul(2.0)]));
        assert_eq!(parse_num_expr("/2"), Some(vec![NumOp::Div(2.0)]));
        assert_eq!(parse_num_expr("÷2"), Some(vec![NumOp::Div(2.0)]));
        assert_eq!(
            parse_num_expr("x3/7"),
            Some(vec![NumOp::Mul(3.0), NumOp::Div(7.0)])
        );
    }

    #[test]
    fn parse_percent() {
        assert_eq!(parse_num_expr("20%"), Some(vec![NumOp::Mul(0.2)]));
        assert_eq!(parse_num_expr("x.2"), Some(vec![NumOp::Mul(0.2)]));
        assert_eq!(parse_num_expr("x50%"), Some(vec![NumOp::Mul(0.5)]));
    }

    #[test]
    fn parse_assign_then_chain() {
        assert_eq!(
            parse_num_expr("3x2"),
            Some(vec![NumOp::Set(3.0), NumOp::Mul(2.0)])
        );
        assert_eq!(
            parse_num_expr("100+10"),
            Some(vec![NumOp::Set(100.0), NumOp::Add(10.0)])
        );
    }

    #[test]
    fn parse_cn_decimal_point() {
        assert_eq!(parse_num_expr("x3。5"), Some(vec![NumOp::Mul(3.5)]));
        assert_eq!(parse_num_expr("1。5"), Some(vec![NumOp::Set(1.5)]));
    }

    #[test]
    fn parse_invalid() {
        assert_eq!(parse_num_expr(""), None);
        assert_eq!(parse_num_expr("  "), None);
        assert_eq!(parse_num_expr("+"), None);
        assert_eq!(parse_num_expr("x"), None);
        assert_eq!(parse_num_expr("/0"), None);
        assert_eq!(parse_num_expr("+20%"), None);
        assert_eq!(parse_num_expr("-20%"), None);
        assert_eq!(parse_num_expr("3 2"), None);
        assert_eq!(parse_num_expr("abc"), None);
        assert_eq!(parse_num_expr("2x"), None); // 数字后缺数字
    }

    #[test]
    fn apply_chain() {
        let ops = parse_num_expr("x3/7").unwrap();
        assert!((apply_ops(&ops, 14.0) - 6.0).abs() < 1e-9);
        let ops = parse_num_expr("+2x3").unwrap();
        assert!((apply_ops(&ops, 1.0) - 9.0).abs() < 1e-9);
    }
}
