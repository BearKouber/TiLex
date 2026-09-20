//! 结果浮窗摆放（design §2.7，从旧 `pop_result_sizing.js` 重写成纯函数）。
//! Slint 给内容高度（逻辑像素），这里算出物理像素矩形，界面层一次 `SetWindowPos` 同时设位置和尺寸。
//! 旧版为 WebView 出帧做的串行合并、1px 显示、repaint 全部不需要。

use crate::platform::geometry::{Rect, Side, place};

/// 浮窗宽度和最大高度（逻辑像素）。内容超过最大高度时在窗口里滚动。
pub const RESULT_WIDTH: f32 = 320.0;
pub const RESULT_MAX_HEIGHT: f32 = 400.0;

/// 内容变高时哪条边不动。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pin {
    /// 浮窗在基准上方：底边钉在这个物理 y 上，往上长（划词处不被盖住）。
    Bottom(i32),
    /// 在下方，或用户拖动过：顶边不动，往下长，出屏就贴底往上推。
    Top,
}

fn size(content_height: f32, scale: f32) -> Option<(i32, i32)> {
    if !content_height.is_finite() || content_height <= 0.0 || !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let logical = content_height.ceil().min(RESULT_MAX_HEIGHT);
    Some((
        (RESULT_WIDTH * scale).round() as i32,
        (logical * scale).round() as i32,
    ))
}

/// 第一次显示：按用户选的方位摆在基准（选区 / 光标）旁边，放不下翻到另一侧再贴工作区边。
/// `bounds` 是基准所在显示器的工作区（物理像素，可以是负坐标）。量不出高度返回 `None`。
pub fn first_placement(
    anchor: Rect,
    sx: Side,
    sy: Side,
    gap: i32,
    content_height: f32,
    scale: f32,
    bounds: Rect,
) -> Option<(Rect, Pin)> {
    let (w, h) = size(content_height, scale)?;
    let (x, y, above) = place(anchor, w, h, sx, sy, gap, bounds);
    let pin = if above { Pin::Bottom(y + h) } else { Pin::Top };
    Some((
        Rect {
            l: x,
            t: y,
            r: x + w,
            b: y + h,
        },
        pin,
    ))
}

/// 内容高度变了：钉底边的往上长（顶部出屏就贴顶），钉顶边的顶边不动（底部出屏就贴底上推）。
/// `current` 是窗口此刻的物理矩形（用户可能拖过）。量不出高度（0、负数、NaN）返回 `None`，窗口不动。
/// 用户按下顶栏开始拖动时把 pin 换成 `Pin::Top`，之后变高不再跳回原来的底边。
pub fn place_result(
    current: Rect,
    pin: Pin,
    content_height: f32,
    scale: f32,
    bounds: Rect,
) -> Option<Rect> {
    let (w, h) = size(content_height, scale)?;
    let y = match pin {
        Pin::Bottom(bottom) => (bottom - h).max(bounds.t),
        Pin::Top => current.t.min(bounds.b - h).max(bounds.t),
    };
    Some(Rect {
        l: current.l,
        t: y,
        r: current.l + w,
        b: y + h,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect {
        l: 0,
        t: 0,
        r: 1000,
        b: 1000,
    };

    fn at(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect {
            l: x,
            t: y,
            r: x + w,
            b: y + h,
        }
    }

    #[test]
    fn grows_upwards_with_the_bottom_pinned() {
        let current = at(30, 620, 320, 80);
        let r = place_result(current, Pin::Bottom(700), 240.0, 1.0, SCREEN).unwrap();
        assert_eq!(r, at(30, 460, 320, 240));
        let r = place_result(r, Pin::Bottom(500), 240.0, 1.0, SCREEN).unwrap();
        assert_eq!(r.t, 260, "锚点变了按新底边");
    }

    #[test]
    fn grows_downwards_and_clamps_at_the_bottom() {
        let r = place_result(at(30, 30, 320, 80), Pin::Top, 200.0, 1.0, SCREEN).unwrap();
        assert_eq!(r.t, 30, "顶边不动");
        let r = place_result(at(30, 900, 320, 80), Pin::Top, 200.0, 1.0, SCREEN).unwrap();
        assert_eq!(r.t, 800, "出屏贴底上推");
        let r = place_result(at(30, 30, 320, 240), Pin::Top, 80.0, 1.0, SCREEN).unwrap();
        assert_eq!(r, at(30, 30, 320, 80), "变矮也是顶边不动");
    }

    #[test]
    fn physical_pixels_and_negative_monitors() {
        // 上方的副屏，150% 缩放：顶部出屏就贴顶
        let upper = Rect {
            l: 0,
            t: -500,
            r: 1000,
            b: 0,
        };
        let r = place_result(at(30, -400, 480, 90), Pin::Bottom(-200), 240.0, 1.5, upper).unwrap();
        assert_eq!(r, at(30, -500, 480, 360));
        let r = place_result(at(30, 400, 480, 90), Pin::Top, 240.0, 1.5, SCREEN).unwrap();
        assert_eq!(r.t, 400);
        assert_eq!(r.b - r.t, 360);
    }

    #[test]
    fn height_is_capped_and_rounded_up() {
        let r = place_result(at(0, 0, 320, 80), Pin::Top, 999.0, 1.0, SCREEN).unwrap();
        assert_eq!(r.b - r.t, 400, "超过最大高度在窗口里滚动");
        let r = place_result(at(0, 0, 320, 80), Pin::Top, 80.2, 1.0, SCREEN).unwrap();
        assert_eq!(r.b - r.t, 81);
    }

    #[test]
    fn unmeasurable_heights_are_skipped() {
        for h in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(
                place_result(at(0, 0, 320, 80), Pin::Top, h, 1.0, SCREEN),
                None,
                "{h}"
            );
        }
        assert_eq!(
            place_result(at(0, 0, 320, 80), Pin::Top, 80.0, 0.0, SCREEN),
            None
        );
    }

    #[test]
    fn dragging_clears_the_anchor() {
        let r = place_result(at(30, 620, 320, 80), Pin::Bottom(700), 200.0, 1.0, SCREEN).unwrap();
        assert_eq!(r.t, 500);
        // 用户拖到 y = 300，界面把 pin 换成 Top
        let dragged = at(30, 300, 320, 200);
        let r = place_result(dragged, Pin::Top, 250.0, 1.0, SCREEN).unwrap();
        assert_eq!(r.t, 300, "不跳回原来的底边");
    }

    #[test]
    fn first_show_flips_above_near_the_bottom_edge() {
        // 光标靠近屏幕底：选了「右下」放不下，翻到上方，之后钉底边往上长
        let (rect, pin) = first_placement(
            Rect::point(500, 950),
            Side::After,
            Side::After,
            0,
            100.0,
            1.0,
            SCREEN,
        )
        .unwrap();
        assert_eq!(rect, at(500, 850, 320, 100));
        assert_eq!(pin, Pin::Bottom(950));
        let grown = place_result(rect, pin, 300.0, 1.0, SCREEN).unwrap();
        assert_eq!(grown.b, 950);
        // 放得下就在下方，顶边不动
        let (rect, pin) = first_placement(
            Rect::point(500, 100),
            Side::After,
            Side::After,
            0,
            100.0,
            1.0,
            SCREEN,
        )
        .unwrap();
        assert_eq!((rect.t, pin), (100, Pin::Top));
        // 左侧负坐标副屏，右边放不下翻到左边
        let left = Rect {
            l: -1920,
            t: 0,
            r: 0,
            b: 1040,
        };
        let (rect, _) = first_placement(
            Rect::point(-50, 500),
            Side::After,
            Side::After,
            0,
            100.0,
            1.0,
            left,
        )
        .unwrap();
        assert_eq!(rect.l, -370);
        assert_eq!(
            first_placement(
                Rect::point(0, 0),
                Side::After,
                Side::After,
                0,
                0.0,
                1.0,
                SCREEN
            ),
            None
        );
    }
}
