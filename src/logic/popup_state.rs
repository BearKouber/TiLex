//! 结果浮窗的失焦隐藏（从旧 `pop_result_lifecycle.js` 重写成同步状态机，时间由调用方传入，测试用假时钟）。
//!
//! 刚显示时会先收到一次交接抖动的失焦：显示后 300ms 内的失焦不直接丢掉，也不直接隐藏，
//! 而是等宽限期结束再查一次真实焦点——真切走了才隐藏（旧版「有时候切窗口它不自动关」的根因）。
//! 每次显示是一个代次；新显示、重新获得焦点、取消都作废旧代次的延时检查。

use std::time::{Duration, Instant};

pub const GRACE: Duration = Duration::from_millis(300);

/// 收到失焦后该做什么。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blur {
    /// 钉住了，或已经作废。
    Ignore,
    /// 过了宽限期：马上隐藏。
    Hide,
    /// 宽限期内：`after` 之后调 [`BlurGuard::check`]，把 `token` 原样带回来。
    CheckLater { after: Duration, token: u64 },
}

#[derive(Debug)]
pub struct BlurGuard {
    generation: u64,
    shown_at: Option<Instant>,
    pinned: bool,
}

impl Default for BlurGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl BlurGuard {
    pub const fn new() -> Self {
        Self {
            generation: 0,
            shown_at: None,
            pinned: false,
        }
    }

    /// 新显示（新划词 / 新截图结果）：作废旧检查，取消钉住，重新计宽限期。
    pub fn begin(&mut self, now: Instant) {
        self.invalidate();
        self.shown_at = Some(now);
        self.pinned = false;
    }

    /// 取消、隐藏、关闭：作废在途的延时检查。
    pub fn invalidate(&mut self) {
        self.generation += 1;
    }

    /// 重新获得焦点：之前的失焦全部作废（但不延长原来的宽限期）。
    pub fn focus(&mut self) {
        self.invalidate();
    }

    /// 钉住后失焦不隐藏；只有关闭角、Esc、取消钉住后的失焦才会让它消失。
    pub fn set_pinned(&mut self, pinned: bool) {
        self.pinned = pinned;
    }

    pub fn pinned(&self) -> bool {
        self.pinned
    }

    pub fn blur(&mut self, now: Instant) -> Blur {
        if self.pinned {
            return Blur::Ignore;
        }
        let Some(shown) = self.shown_at else {
            return Blur::Ignore;
        };
        let elapsed = now.saturating_duration_since(shown);
        if elapsed > GRACE {
            self.invalidate();
            return Blur::Hide;
        }
        // 同一代次里后一次失焦替换前一次：旧 token 作废。
        self.invalidate();
        Blur::CheckLater {
            after: GRACE - elapsed,
            token: self.generation,
        }
    }

    /// 延时到了：`focused` 是此刻查到的真实焦点，查不到传 `None`（查不到不算真失焦）。返回 true 就隐藏。
    pub fn check(&mut self, token: u64, focused: Option<bool>) -> bool {
        if token != self.generation || self.pinned || focused != Some(false) {
            return false;
        }
        self.invalidate();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn later(guard: &mut BlurGuard, now: Instant) -> u64 {
        match guard.blur(now) {
            Blur::CheckLater { token, .. } => token,
            other => panic!("expected a delayed check, got {other:?}"),
        }
    }

    #[test]
    fn early_blur_waits_for_the_grace_period() {
        let t0 = Instant::now();
        let mut g = BlurGuard::new();
        g.begin(t0);
        assert_eq!(
            g.blur(t0),
            Blur::CheckLater {
                after: GRACE,
                token: g.generation
            }
        );
        let token = later(&mut g, t0 + Duration::from_millis(100));
        assert!(!g.check(token, Some(true)), "焦点交接：还有焦点就留着");
        let token = later(&mut g, t0 + Duration::from_millis(200));
        assert!(g.check(token, Some(false)), "宽限期内真切走了照样隐藏");
    }

    #[test]
    fn new_display_invalidates_old_checks() {
        let t0 = Instant::now();
        let mut g = BlurGuard::new();
        g.begin(t0);
        let old = later(&mut g, t0);
        g.begin(t0 + Duration::from_millis(50));
        assert!(!g.check(old, Some(false)), "旧查询不能关掉新显示");
    }

    #[test]
    fn late_blur_hides_immediately() {
        let t0 = Instant::now();
        let mut g = BlurGuard::new();
        g.begin(t0);
        assert_eq!(g.blur(t0 + Duration::from_millis(301)), Blur::Hide);
    }

    #[test]
    fn cancel_and_refocus_invalidate() {
        let t0 = Instant::now();
        let mut g = BlurGuard::new();
        g.begin(t0);
        let token = later(&mut g, t0);
        g.invalidate();
        assert!(!g.check(token, Some(false)), "取消作废在途检查");

        g.begin(t0);
        let token = later(&mut g, t0);
        g.focus();
        assert!(!g.check(token, Some(false)), "重新获得焦点作废更早的失焦");
        // 重新获得焦点不延长原来的宽限期
        assert_eq!(g.blur(t0 + Duration::from_millis(301)), Blur::Hide);
    }

    #[test]
    fn unknown_focus_is_not_a_blur() {
        let t0 = Instant::now();
        let mut g = BlurGuard::new();
        g.begin(t0);
        let token = later(&mut g, t0);
        assert!(!g.check(token, None));
    }

    #[test]
    fn a_second_blur_replaces_the_first_check() {
        let t0 = Instant::now();
        let mut g = BlurGuard::new();
        g.begin(t0);
        let first = later(&mut g, t0);
        let second = later(&mut g, t0 + Duration::from_millis(100));
        assert!(!g.check(first, Some(false)));
        assert!(g.check(second, Some(false)));
    }

    #[test]
    fn pinned_ignores_blur_until_unpinned() {
        let t0 = Instant::now();
        let mut g = BlurGuard::new();
        g.begin(t0);
        g.set_pinned(true);
        assert_eq!(g.blur(t0), Blur::Ignore, "宽限期内钉住");
        assert_eq!(
            g.blur(t0 + Duration::from_secs(5)),
            Blur::Ignore,
            "宽限期后钉住"
        );
        g.set_pinned(false);
        assert_eq!(g.blur(t0 + Duration::from_secs(5)), Blur::Hide);

        // 钉住之前排上的检查，到点时已钉住也不隐藏
        g.begin(t0);
        let token = later(&mut g, t0);
        g.set_pinned(true);
        assert!(!g.check(token, Some(false)));
        // 新显示取消钉住
        g.begin(t0);
        assert!(!g.pinned());
    }

    #[test]
    fn nothing_shown_yet_ignores_blur() {
        assert_eq!(BlurGuard::new().blur(Instant::now()), Blur::Ignore);
    }
}
