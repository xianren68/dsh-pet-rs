//! 数工资：按上班 / 下班 / 午休时间和月薪，折算今天到此刻赚了多少。
//!
//! **不落盘**。每次现算，跨天自然归零，不需要清缓存也不用担心跨天残留。
//! 时间口径：午休完全不计薪（12:00~13:00 这段时间不产生钱），
//! 上班前为 0，下班后封顶到全天日薪。

use crate::app::UserSettings;
use crate::event::{local_time, parse_hm};
use crate::holiday::FALLBACK_WORKDAYS;

pub struct Progress {
    /// 今天到此刻已赚（元）
    pub earned: f32,
    /// 全天日薪（元）
    pub daily: f32,
    /// 已计薪分钟
    pub worked: u32,
    /// 应计薪分钟（扣掉午休）
    pub total: u32,
    /// 本次用的当月工作日数（拿不到节假日数据时是 21.75）
    pub workdays: f32,
}

/// 算今天到此刻的工资进度。
///
/// 上下班时间不合法（格式错、下班早于上班）时返回 `None`，调用方据此静默跳过。
/// 午休填错则不会让整个功能失效 —— 只当没有午休，见下。
///
/// `workdays` 是当月实际工作日数（来自节假日表）。传 0 或负数会用 21.75 兜底。
pub fn progress(s: &UserSettings, workdays: f32) -> Option<Progress> {
    let start = parse_hm(&s.work_start)?;
    let end = parse_hm(&s.work_end)?;

    if end <= start {
        return None;
    }
    // 午休宽容处理：留空、填错、起止相等、或者不在上下班区间内，
    // 一律当作「没有午休」而不是整个功能哑掉。有些公司确实没午休。
    let (lunch_from, lunch_to) = match (parse_hm(&s.lunch_start), parse_hm(&s.lunch_end)) {
        (Some(a), Some(b)) if b > a && a >= start && b <= end => (a, b),
        _ => (0, 0),
    };
    let lunch = lunch_to - lunch_from;

    let total = end - start - lunch;
    if total == 0 {
        return None;
    }

    let (h, m) = local_time();
    let now = h * 60 + m;

    // 没午休时 lunch_from/lunch_to 都是 0，下面两个比较自然落空
    let worked = if now <= start {
        0
    } else if now < lunch_from {
        now - start
    } else if now < lunch_to {
        // 午休中：停在午休开始的那一刻
        lunch_from - start
    } else if now < end {
        (now - start) - lunch
    } else {
        total
    };

    let days = if workdays > 0.0 {
        workdays
    } else {
        FALLBACK_WORKDAYS
    };
    let daily = s.salary_month as f32 / days;
    let earned = daily * (worked as f32 / total as f32);

    Some(Progress {
        earned,
        daily,
        worked,
        total,
        workdays: days,
    })
}

/// 拼成气泡文案。金额按习惯保留到分，但整数就不显示 `.00`。
///
/// 三个模板来自 events.json 的 `salary.texts`，`{amount}` 会被替换成金额。
pub fn bubble_text(p: &Progress, texts: &crate::event::SalaryTexts) -> String {
    let yen = |v: f32| {
        if (v - v.round()).abs() < 0.005 {
            format!("{:.0}", v)
        } else {
            format!("{:.2}", v)
        }
    };
    let (tpl, amount) = if p.worked == 0 {
        (&texts.before, "0".to_string())
    } else if p.worked >= p.total {
        (&texts.after, yen(p.daily))
    } else {
        (&texts.during, yen(p.earned))
    };
    let days = if (p.workdays - p.workdays.round()).abs() < 0.05 {
        format!("{:.0}", p.workdays)
    } else {
        format!("{:.1}", p.workdays)
    };
    tpl.replace("{amount}", &amount)
        .replace("{days}", &days)
        .replace("{daily}", &yen(p.daily))
}
