use std::time::{Instant, SystemTime};

/// 历史条目年龄文案（纯函数，方便单测）：5 秒内“刚刚”，之后秒/分/时/天。
/// 无 chrono/time 依赖，超 7 天也显示“{n}天前”一直下去，不做 MM-DD。
pub(super) fn format_age(created: Instant, now: Instant) -> String {
    let secs = now.saturating_duration_since(created).as_secs();
    if secs <= 5 {
        "刚刚".to_string()
    } else if secs < 60 {
        format!("{}秒前", secs)
    } else if secs < 3600 {
        format!("{}分钟前", secs / 60)
    } else if secs < 86_400 {
        format!("{}小时前", secs / 3600)
    } else {
        format!("{}天前", secs / 86_400)
    }
}

/// 墙钟绝对时间（纯函数，本地时区）：`%Y-%m-%d %H:%M:%S`。
pub(super) fn format_wall(wall: SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Local> = wall.into();
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 统一时间戳行（纯函数）：`{相对} · {绝对}`，相对复用 `format_age`。
/// `created` 是单调钟，只做相对；墙钟由调用方按
/// `SystemTime::now().checked_sub(created.elapsed())` 反推后以 `now_wall` 传入，
/// 未来时间钳制到 `now_wall`（`saturating` + `checked_sub` 回退）。
pub(super) fn format_timestamp(
    created: Instant,
    now_steady: Instant,
    now_wall: SystemTime,
) -> String {
    let elapsed = now_steady.saturating_duration_since(created);
    let wall_created = now_wall.checked_sub(elapsed).unwrap_or(now_wall);
    format!(
        "{} · {}",
        format_age(created, now_steady),
        format_wall(wall_created)
    )
}
#[cfg(test)]
mod tests {
    use super::*;

    /// format_age 全分支：刚刚/秒/分钟/小时/天（含超 7 天一直显示天数）。
    #[test]
    fn format_age_branches() {
        use std::time::{Duration, Instant};
        let now = Instant::now();
        let ago = |d: Duration| format_age(now - d, now);
        // 5 秒内“刚刚”（含边界）
        assert_eq!(ago(Duration::from_secs(0)), "刚刚");
        assert_eq!(ago(Duration::from_secs(5)), "刚刚");
        // 60 秒内秒
        assert_eq!(ago(Duration::from_secs(6)), "6秒前");
        assert_eq!(ago(Duration::from_secs(59)), "59秒前");
        // 60 分钟内分钟
        assert_eq!(ago(Duration::from_secs(60)), "1分钟前");
        assert_eq!(ago(Duration::from_secs(3599)), "59分钟前");
        // 24 小时内小时
        assert_eq!(ago(Duration::from_secs(3600)), "1小时前");
        assert_eq!(ago(Duration::from_secs(86399)), "23小时前");
        // 天（含 7 天边界与更早：无 chrono 依赖，一直显示天数）
        assert_eq!(ago(Duration::from_secs(86_400)), "1天前");
        assert_eq!(ago(Duration::from_secs(6 * 86_400)), "6天前");
        assert_eq!(ago(Duration::from_secs(7 * 86_400)), "7天前");
        assert_eq!(ago(Duration::from_secs(30 * 86_400)), "30天前");
        // 未来时间（created 晚于 now）钳制为“刚刚”
        assert_eq!(format_age(now, now - Duration::from_secs(10)), "刚刚");
    }

    /// format_wall 全确定性：UNIX_EPOCH + 已知秒数，不读真实时钟，不 hardcode 时区偏移。
    #[test]
    fn format_wall_known_epoch() {
        use std::time::{Duration, UNIX_EPOCH};
        let wall = UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        let s = format_wall(wall);
        // 形如 2001-09-09 01:46:40（本地时区日期可能不同，只断言格式骨架）
        assert_eq!(s.len(), 19);
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[7..8], "-");
        assert_eq!(&s[10..11], " ");
        assert_eq!(&s[13..14], ":");
        assert_eq!(&s[16..17], ":");
        // 同口径期望（chrono Local 同一转换不断言具体偏移）
        let expected = chrono::DateTime::<chrono::Local>::from(wall)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        assert_eq!(s, expected);
    }

    /// format_timestamp 组合 `{age} · {wall}`：age 复用 format_age，wall 由 now_wall 反推。
    #[test]
    fn format_timestamp_combines_age_and_wall() {
        use std::time::{Duration, UNIX_EPOCH};
        let now_steady = Instant::now();
        let created = now_steady - Duration::from_secs(12);
        let now_wall = UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        let s = format_timestamp(created, now_steady, now_wall);
        let wall_created = now_wall - Duration::from_secs(12);
        let expected = format!(
            "{} · {}",
            format_age(created, now_steady),
            format_wall(wall_created)
        );
        assert_eq!(s, expected);
        assert!(s.starts_with("12秒前 · "));
        // 未来 created 钳制：相对“刚刚”，绝对即 now_wall 本身
        let future = now_steady + Duration::from_secs(10);
        let s2 = format_timestamp(future, now_steady, now_wall);
        assert_eq!(s2, format!("刚刚 · {}", format_wall(now_wall)));
    }
}
