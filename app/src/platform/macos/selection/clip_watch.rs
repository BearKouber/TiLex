//! macOS 剪贴板变化被动监听（宽限窗口轮询线程）。
//!
//! macOS NSPasteboard 没有系统级剪贴板变化事件通知（如 Windows 的 WM_CLIPBOARDUPDATE）。
//! 本模块在产生选中手势后，在 CLIP_GRACE_MS（600ms）宽限期内以 20ms 为间隔轮询 changeCount。
//!
//! 关键特征：
//! - 平时线程完全阻塞在 channel 上，零 CPU 与电量开销。
//! - tap 线程产生手势时通过 `notify_gesture` 发送 `PendingGesture` 唤醒轮询。
//! - 等待期间收到新手势时，立刻丢弃旧手势、以新手势为准重新开始。
//! - 一旦发现 changeCount 变化，调用 `capture_clipboard` 捕获候选并向 worker 发送 `Ev::Clip`，随后结束本轮。
//! - 超出 600ms 宽限期仍未变化则放弃本轮，重新进入阻塞等待。

use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use super::tap::cancel_current;
use super::{CURRENT_GESTURE, Ev, current_at_ms, current_clipboard_sequence, foreground_pid, send};
use crate::error::Error;
use crate::platform::selection_state::{CLIP_GRACE_MS, Gesture, PendingGesture};

static CLIP_WATCH_TX: OnceLock<Sender<PendingGesture>> = OnceLock::new();
static CURRENT_PENDING: RwLock<Option<Gesture>> = RwLock::new(None);

pub fn start_clip_watch_thread() -> Result<(), Error> {
    let (tx, rx) = channel();
    CLIP_WATCH_TX
        .set(tx)
        .map_err(|_| Error::Platform("clip_watch thread already started".into()))?;

    std::thread::Builder::new()
        .name("selection-clip".into())
        .spawn(move || run_clip_watch(rx))?;

    Ok(())
}

/// 供 tap 线程在产生选中手势时通知本轮询线程。
pub fn notify_gesture(pending: PendingGesture) {
    if let Ok(mut w) = CURRENT_PENDING.write() {
        *w = Some(pending.gesture);
    }
    if let Some(tx) = CLIP_WATCH_TX.get() {
        let _ = tx.send(pending); // ignore: 接收端退出只会发生在进程退出时
    }
}

/// 获取当前待处理的手势（供前台切换通知核查 interrupted_by）。
pub fn current_pending_gesture() -> Option<Gesture> {
    CURRENT_PENDING.read().ok().and_then(|g| *g)
}

fn run_clip_watch(rx: Receiver<PendingGesture>) {
    while let Ok(mut pending) = rx.recv() {
        // 若 channel 中排队积压了更新的手势，直接取最新的一个
        while let Ok(newer) = rx.try_recv() {
            pending = newer;
        }

        loop {
            let gesture = pending.gesture;
            // 全局手势序号已递增：手势已被取消或被更新手势作废
            if gesture.id != CURRENT_GESTURE.load(Ordering::SeqCst) {
                break;
            }

            let at_ms = current_at_ms();
            let current_seq = current_clipboard_sequence();

            match evaluate_poll(
                gesture.at_ms,
                at_ms,
                gesture.clipboard_sequence,
                current_seq,
                CLIP_GRACE_MS,
            ) {
                PollOutcome::Hit => {
                    let window = foreground_pid();
                    if window != gesture.window {
                        cancel_current();
                        break;
                    }
                    if let Some(candidate) = pending.capture_clipboard(window, at_ms, current_seq) {
                        send(Ev::Clip(candidate));
                    }
                    break;
                }
                PollOutcome::Expired => {
                    break;
                }
                PollOutcome::Pending => {}
            }

            // 等待最多 20ms：若中途来了新手势则立刻切换；超时则继续下一轮比对
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(newer) => {
                    pending = newer;
                    while let Ok(latest) = rx.try_recv() {
                        pending = latest;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PollOutcome {
    Hit,
    Expired,
    Pending,
}

/// 纯函数：判定给定时刻及剪贴板序列号下的轮询结果。
pub fn evaluate_poll(
    gesture_at_ms: u32,
    now_at_ms: u32,
    baseline_seq: u32,
    current_seq: u32,
    grace_ms: u32,
) -> PollOutcome {
    let elapsed = now_at_ms.wrapping_sub(gesture_at_ms);
    if current_seq != baseline_seq && current_seq != 0 {
        if elapsed <= grace_ms {
            PollOutcome::Hit
        } else {
            PollOutcome::Expired
        }
    } else if elapsed > grace_ms {
        PollOutcome::Expired
    } else {
        PollOutcome::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evaluate_poll_pending_when_unchanged() {
        assert_eq!(evaluate_poll(1000, 1200, 10, 10, 600), PollOutcome::Pending);
    }

    #[test]
    fn test_evaluate_poll_hit_when_changed_within_grace() {
        assert_eq!(evaluate_poll(1000, 1200, 10, 11, 600), PollOutcome::Hit);
        assert_eq!(evaluate_poll(1000, 1600, 10, 11, 600), PollOutcome::Hit);
    }

    #[test]
    fn test_evaluate_poll_zero_sequence_pending() {
        // 序列号为 0 代表无效/未就绪，不能误判为 Hit
        assert_eq!(evaluate_poll(1000, 1200, 10, 0, 600), PollOutcome::Pending);
    }

    #[test]
    fn test_evaluate_poll_expired_after_grace() {
        assert_eq!(evaluate_poll(1000, 1601, 10, 10, 600), PollOutcome::Expired);
        // 超出宽限期才发生的变化视为迟到，判作 Expired
        assert_eq!(evaluate_poll(1000, 1601, 10, 11, 600), PollOutcome::Expired);
    }

    #[test]
    fn test_evaluate_poll_timer_wrap_around_u32_boundary() {
        let gesture_at = u32::MAX - 100;
        let now_within = 200; // wrapping_sub 得到 301 <= 600
        assert_eq!(
            evaluate_poll(gesture_at, now_within, 5, 5, 600),
            PollOutcome::Pending
        );
        assert_eq!(
            evaluate_poll(gesture_at, now_within, 5, 6, 600),
            PollOutcome::Hit
        );

        let now_expired = 600; // wrapping_sub 得到 701 > 600
        assert_eq!(
            evaluate_poll(gesture_at, now_expired, 5, 5, 600),
            PollOutcome::Expired
        );
    }
}
