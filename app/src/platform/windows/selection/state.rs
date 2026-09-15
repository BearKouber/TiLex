//! 手势与剪贴板候选的归属规则（从旧版 `pop_button/selection.rs` 原样搬来）。
//! 取词 worker 和测试共用；这里不许出现 Win32、剪贴板、配置、窗口调用。

/// 剪贴板更新离手势多久以内还算"这次选中自己复制的"（毫秒，按产生时刻算）。
pub const CLIP_GRACE_MS: u32 = 600;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gesture {
    pub id: u64,
    pub window: isize,
    pub x: i32,
    pub y: i32,
    pub at_ms: u32,
    pub clipboard_sequence: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipboardCandidate {
    pub gesture: Gesture,
    pub window: isize,
    pub at_ms: u32,
    pub sequence: u32,
}

#[derive(Clone, Copy)]
pub struct PendingGesture {
    pub gesture: Gesture,
    clipboard_sent: bool,
}

impl PendingGesture {
    pub fn new(gesture: Gesture) -> Self {
        Self {
            gesture,
            clipboard_sent: false,
        }
    }

    pub fn capture_clipboard(
        &mut self,
        window: isize,
        at_ms: u32,
        sequence: u32,
    ) -> Option<ClipboardCandidate> {
        let clip = ClipboardCandidate {
            gesture: self.gesture,
            window,
            at_ms,
            sequence,
        };
        if self.clipboard_sent
            || !clip.is_current(ReadContext {
                gesture_id: self.gesture.id,
                window,
                clipboard_sequence: sequence,
                armed: true,
            })
        {
            return None;
        }
        // 一个手势只认第一次剪贴板更新。之后的变化只让这个候选失效，
        // 不会把 600ms 内无关的新内容再算到这个旧手势头上。
        self.clipboard_sent = true;
        Some(clip)
    }
}

#[derive(Clone, Copy)]
pub struct ReadContext {
    pub gesture_id: u64,
    pub window: isize,
    pub clipboard_sequence: u32,
    pub armed: bool,
}

impl Gesture {
    pub fn is_current(self, context: ReadContext) -> bool {
        self.id != 0
            && self.id == context.gesture_id
            && self.window != 0
            && self.window == context.window
            && context.armed
    }

    pub fn interrupted_by(self, window: isize, at_ms: u32) -> bool {
        // WinEvent 可能比后来的鼠标事件晚到：早于本手势的事件不算（含 tick 计数回绕）。
        window != self.window && at_ms.wrapping_sub(self.at_ms) < (1 << 31)
    }
}

impl ClipboardCandidate {
    pub fn is_current(self, context: ReadContext) -> bool {
        self.gesture.is_current(context)
            && self.window == self.gesture.window
            && self.at_ms.wrapping_sub(self.gesture.at_ms) <= CLIP_GRACE_MS
            && self.sequence != 0
            && self.sequence != self.gesture.clipboard_sequence
            && self.sequence == context.clipboard_sequence
    }
}

pub struct Offer {
    pub gesture: Gesture,
    pub text: String,
    pub clipboard: Option<ClipboardCandidate>,
}

impl Offer {
    pub fn is_current(&self, context: ReadContext) -> bool {
        self.clipboard.map_or_else(
            || self.gesture.is_current(context),
            |clip| clip.is_current(context),
        )
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    WaitingUia,
    NoUia,
    Consumed,
    Cancelled,
}

pub struct Candidates {
    gesture: Option<Gesture>,
    phase: Phase,
    clipboard: Option<ClipboardCandidate>,
}

impl Default for Candidates {
    fn default() -> Self {
        Self {
            gesture: None,
            phase: Phase::Cancelled,
            clipboard: None,
        }
    }
}

impl Candidates {
    pub fn begin(&mut self, gesture: Gesture) {
        self.gesture = Some(gesture);
        self.phase = Phase::WaitingUia;
        self.clipboard = None;
    }

    pub fn cancel(&mut self, id: u64) {
        if self.gesture.map(|g| g.id) == Some(id) {
            self.phase = Phase::Cancelled;
            self.clipboard = None;
        }
    }

    pub fn complete_uia(
        &mut self,
        gesture: Gesture,
        text: String,
        context: ReadContext,
        mut offer: impl FnMut(Offer),
    ) -> Option<ClipboardCandidate> {
        if self.gesture != Some(gesture) || self.phase != Phase::WaitingUia {
            return None;
        }
        if !gesture.is_current(context) {
            self.cancel(gesture.id);
            return None;
        }
        if text.trim().is_empty() {
            self.phase = Phase::NoUia;
            return self.clipboard.take();
        }
        // 先消费再交给过滤（长度、母语）：UIA 答案被过滤掉时，也不能让无关的剪贴板文字绕过过滤。
        self.phase = Phase::Consumed;
        self.clipboard = None;
        offer(Offer {
            gesture,
            text,
            clipboard: None,
        });
        None
    }

    pub fn clipboard_ready(&mut self, clip: ClipboardCandidate, context: ReadContext) -> bool {
        if self.gesture != Some(clip.gesture) || !clip.is_current(context) {
            return false;
        }
        match self.phase {
            Phase::WaitingUia => {
                self.clipboard = Some(clip);
                false
            }
            Phase::NoUia => true,
            Phase::Consumed | Phase::Cancelled => false,
        }
    }

    pub fn complete_clipboard(
        &mut self,
        clip: ClipboardCandidate,
        text: String,
        context: ReadContext,
        mut offer: impl FnMut(Offer),
    ) {
        if self.phase != Phase::NoUia
            || self.gesture != Some(clip.gesture)
            || !clip.is_current(context)
            || text.trim().is_empty()
        {
            return;
        }
        self.phase = Phase::Consumed;
        self.clipboard = None;
        offer(Offer {
            gesture: clip.gesture,
            text,
            clipboard: Some(clip),
        });
    }
}

pub struct Displayed {
    pub gesture: Gesture,
    pub text: String,
    pub x: i32,
    pub y: i32,
}

#[derive(Default)]
pub struct Display {
    shown: Option<Displayed>,
}

impl Display {
    pub fn show(&mut self, shown: Displayed) {
        self.shown = Some(shown);
    }

    pub fn take(&mut self, owner: u64) -> Option<Displayed> {
        if self.shown.as_ref().map(|shown| shown.gesture.id) == Some(owner) {
            self.shown.take()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gesture(id: u64) -> Gesture {
        Gesture {
            id,
            window: 10,
            x: 100,
            y: 200,
            at_ms: 1000,
            clipboard_sequence: 20,
        }
    }

    fn context(g: Gesture) -> ReadContext {
        ReadContext {
            gesture_id: g.id,
            window: g.window,
            clipboard_sequence: 21,
            armed: true,
        }
    }

    fn clip(g: Gesture) -> ClipboardCandidate {
        ClipboardCandidate {
            gesture: g,
            window: g.window,
            at_ms: 1100,
            sequence: 21,
        }
    }

    #[test]
    fn uia_consumes_once_for_both_clipboard_arrival_orders() {
        for early_clip in [false, true] {
            let g = gesture(1);
            let mut state = Candidates::default();
            let mut offered = Vec::new();
            state.begin(g);
            if early_clip {
                assert!(!state.clipboard_ready(clip(g), context(g)));
            }
            assert!(
                state
                    .complete_uia(g, "selected".into(), context(g), |v| offered.push(v.text))
                    .is_none()
            );
            assert!(!state.clipboard_ready(clip(g), context(g)));
            state.complete_clipboard(clip(g), "other text".into(), context(g), |v| {
                offered.push(v.text)
            });
            state.complete_uia(g, "duplicate".into(), context(g), |v| offered.push(v.text));
            assert_eq!(offered, ["selected"]);
        }
    }

    #[test]
    fn filtered_uia_still_consumes_the_gesture() {
        let g = gesture(1);
        let mut state = Candidates::default();
        let mut calls = 0;
        state.begin(g);
        state.complete_uia(g, "x".into(), context(g), |_| calls += 1);
        assert!(!state.clipboard_ready(clip(g), context(g)));
        state.complete_clipboard(clip(g), "unrelated".into(), context(g), |_| calls += 1);
        assert_eq!(calls, 1);
    }

    #[test]
    fn automatic_copy_is_available_only_after_empty_uia_in_both_orders() {
        for early_clip in [false, true] {
            let g = gesture(1);
            let c = clip(g);
            let mut state = Candidates::default();
            let mut offered = Vec::new();
            state.begin(g);
            if early_clip {
                assert!(!state.clipboard_ready(c, context(g)));
            }
            let pending = state.complete_uia(g, String::new(), context(g), |_| panic!("empty UIA"));
            assert_eq!(pending, early_clip.then_some(c));
            assert!(state.clipboard_ready(c, context(g)));
            state.complete_clipboard(c, "terminal selection".into(), context(g), |v| {
                offered.push(v.text)
            });
            assert!(!state.clipboard_ready(c, context(g)));
            state.complete_clipboard(c, "duplicate".into(), context(g), |v| offered.push(v.text));
            assert_eq!(offered, ["terminal selection"]);
        }
    }

    #[test]
    fn stale_context_before_and_after_reads_never_offers_text() {
        let g = gesture(1);
        let valid = context(g);
        for invalid in [
            ReadContext {
                gesture_id: 2,
                ..valid
            },
            ReadContext {
                window: 11,
                ..valid
            },
            ReadContext {
                armed: false,
                ..valid
            },
        ] {
            let mut state = Candidates::default();
            state.begin(g);
            state.complete_uia(g, "stale UIA".into(), invalid, |_| {
                panic!("stale UIA offered")
            });
            assert!(!state.clipboard_ready(clip(g), valid));

            state.begin(g);
            state.complete_uia(g, String::new(), valid, |_| panic!("empty UIA"));
            assert!(!state.clipboard_ready(clip(g), invalid));
            assert!(state.clipboard_ready(clip(g), valid));
            state.complete_clipboard(clip(g), "stale clipboard".into(), invalid, |_| {
                panic!("stale clip offered")
            });
        }
    }

    #[test]
    fn clipboard_sequence_is_checked_before_and_after_the_read() {
        let g = gesture(1);
        let valid = context(g);
        let changed = ReadContext {
            clipboard_sequence: 22,
            ..valid
        };
        let mut state = Candidates::default();
        state.begin(g);
        state.complete_uia(g, String::new(), valid, |_| panic!("empty UIA"));
        assert!(!state.clipboard_ready(clip(g), changed));
        assert!(state.clipboard_ready(clip(g), valid));
        state.complete_clipboard(clip(g), "new clipboard".into(), changed, |_| {
            panic!("changed clip offered")
        });
    }

    #[test]
    fn producer_never_reassigns_later_clipboard_content_to_an_old_gesture() {
        let g = gesture(1);
        let mut pending = PendingGesture::new(g);
        assert!(
            pending
                .capture_clipboard(g.window, 1100, g.clipboard_sequence)
                .is_none()
        );
        let original = pending.capture_clipboard(g.window, 1100, 21).unwrap();
        assert_eq!(original.gesture, g);
        assert!(pending.capture_clipboard(g.window, 1101, 22).is_none());
        assert!(!original.is_current(ReadContext {
            clipboard_sequence: 22,
            ..context(g)
        }));
        let mut next = PendingGesture::new(gesture(2));
        assert_eq!(
            next.capture_clipboard(g.window, 1102, 22)
                .unwrap()
                .gesture
                .id,
            2
        );
    }

    #[test]
    fn clipboard_metadata_preserves_source_window_time_and_sequence() {
        let g = gesture(1);
        for invalid in [
            ClipboardCandidate {
                window: 11,
                ..clip(g)
            },
            ClipboardCandidate {
                at_ms: 999,
                ..clip(g)
            },
            ClipboardCandidate {
                at_ms: 1601,
                ..clip(g)
            },
            ClipboardCandidate {
                sequence: 20,
                ..clip(g)
            },
            ClipboardCandidate {
                sequence: 0,
                ..clip(g)
            },
        ] {
            assert!(!invalid.is_current(context(g)));
        }
        assert!(
            ClipboardCandidate {
                at_ms: 1600,
                ..clip(g)
            }
            .is_current(context(g))
        );
        let wrapped = Gesture {
            at_ms: u32::MAX - 10,
            ..g
        };
        assert!(
            ClipboardCandidate {
                at_ms: 20,
                ..clip(wrapped)
            }
            .is_current(context(wrapped))
        );
    }

    #[test]
    fn cancel_and_new_gesture_reject_queued_answers_without_text_deduplication() {
        let old = gesture(1);
        let new = gesture(2);
        let mut state = Candidates::default();
        let mut offered = Vec::new();
        state.begin(old);
        state.cancel(old.id);
        state.complete_uia(old, "stale".into(), context(old), |_| panic!("cancelled"));
        assert!(!state.clipboard_ready(clip(old), context(old)));
        for g in [old, new] {
            state.begin(g);
            state.cancel(999); // 排队中的旧 Cancel 不能取消更新的手势
            state.complete_uia(g, "same text".into(), context(g), |v| offered.push(v.text));
        }
        state.complete_uia(old, "old".into(), context(new), |_| panic!("old UIA"));
        assert!(!state.clipboard_ready(clip(old), context(new)));
        assert_eq!(offered, ["same text", "same text"]);
    }

    #[test]
    fn old_hide_and_duplicate_engage_do_not_consume_a_new_button() {
        let mut display = Display::default();
        display.show(Displayed {
            gesture: gesture(2),
            text: "same text".into(),
            x: 1,
            y: 2,
        });
        assert!(display.take(1).is_none()); // 迟到的 Hide 或 Engage
        let shown = display.take(2).unwrap();
        assert_eq!(shown.text, "same text");
        assert_eq!((shown.x, shown.y), (1, 2));
        assert!(display.take(2).is_none());
    }

    #[test]
    fn foreground_changes_invalidate_even_if_the_user_returns_to_the_window() {
        let g = gesture(1);
        assert!(g.interrupted_by(11, 1100));
        assert!(!g.interrupted_by(11, 900)); // 早于本手势的迟到事件
        assert!(!g.interrupted_by(10, 1100));
        let g = Gesture {
            at_ms: u32::MAX - 10,
            ..g
        };
        assert!(g.interrupted_by(11, 20));
    }
}
