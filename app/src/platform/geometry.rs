//! 屏幕坐标的纯几何：浮标、结果浮窗、截图结果窗的摆放，和浮标的消失半径。
//! 不碰 Win32，两个平台都编译。坐标一律是物理像素，显示器可以在负坐标上。
//! 放在 platform 里是因为鼠标钩子要用消失半径，而 platform 不能依赖 logic；logic 可以直接用这里。

/// 物理像素矩形。光标、图标中心这种「点」就是 l == r、t == b 的零尺寸矩形。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub l: i32,
    pub t: i32,
    pub r: i32,
    pub b: i32,
}

impl Rect {
    pub fn point(x: i32, y: i32) -> Rect {
        Rect {
            l: x,
            t: y,
            r: x,
            b: y,
        }
    }
}

/// 面板在某个轴上相对基准落在哪。
/// x 轴：Before = 基准左侧外，After = 右侧外，Start = 左边齐平，End = 右边齐平。
/// y 轴同理：Before = 上方，After = 下方，Start = 顶齐平，End = 底齐平。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Before,
    After,
    Start,
    End,
}

impl Side {
    fn mirror(self) -> Side {
        match self {
            Side::Before => Side::After,
            Side::After => Side::Before,
            Side::Start => Side::End,
            Side::End => Side::Start,
        }
    }
}

// 一个轴：按 side 摆；越出 [min, max] 就换镜像那一侧；还放不下就把原来那一侧
// 夹回范围内（比范围还大时贴 min）。返回起点和最终用的 side。
// 两边都放不下时夹的是用户选的那一侧、不是镜像那侧：选了「下方」的面板被推上来
// 贴底；夹镜像那侧会把它甩到屏幕顶上去。返回的 side 也是这一侧，
// 所以 place() 报的「在上方」和面板实际往哪长对得上。
fn place_axis(
    lo: i32,
    hi: i32,
    size: i32,
    side: Side,
    gap: i32,
    min: i32,
    max: i32,
) -> (i32, Side) {
    let at = |s: Side| match s {
        Side::Before => lo - gap - size,
        Side::After => hi + gap,
        Side::Start => lo,
        Side::End => hi - size,
    };
    let fits = |p: i32| p >= min && p + size <= max;
    let p = at(side);
    if fits(p) {
        return (p, side);
    }
    let q = at(side.mirror());
    if fits(q) {
        return (q, side.mirror());
    }
    (p.min(max - size).max(min), side)
}

/// 把 w x h 的窗口按 (sx, sy) 摆在 anchor 旁边，放不下先翻转再贴 bounds。gap 两个轴都留。
/// 返回 (x, y, 是否在基准上方)。第三项取翻转之后的结果：结果浮窗靠它决定内容变高时钉住底边还是顶边。
pub fn place(
    anchor: Rect,
    w: i32,
    h: i32,
    sx: Side,
    sy: Side,
    gap: i32,
    bounds: Rect,
) -> (i32, i32, bool) {
    let (x, _) = place_axis(anchor.l, anchor.r, w, sx, gap, bounds.l, bounds.r);
    let (y, sy) = place_axis(anchor.t, anchor.b, h, sy, gap, bounds.t, bounds.b);
    (x, y, sy == Side::Before)
}

/// 普通间距下浮标的消失半径（物理像素）。
pub const DISMISS_DIST: u64 = 60;

/// 两点距离的平方。钩子里比较用它，不开平方；相距再远也不溢出。
pub fn distance_squared(x: i32, y: i32, cx: i32, cy: i32) -> u64 {
    let dx = (i64::from(x) - i64::from(cx)).unsigned_abs();
    let dy = (i64::from(y) - i64::from(cy)).unsigned_abs();
    (dx * dx).saturating_add(dy * dy)
}

/// 鼠标离浮标中心 (cx, cy) 超过多远（平方）就收起浮标。
/// 在 worker 上、翻转/夹边之后按浮标的真实中心算一次：松开鼠标的位置到浮标的整段路都要在范围内，
/// 再留 10px 余量；普通间距仍是 60px。钩子只拿发布出去的结果比较平方。
pub fn dismissal_limit_squared(anchor: Rect, cx: i32, cy: i32) -> u64 {
    let approach = (distance_squared(anchor.l, anchor.t, cx, cy) as f64)
        .sqrt()
        .ceil() as u64;
    let radius = DISMISS_DIST.max(approach + 10);
    radius.saturating_mul(radius)
}

#[cfg(test)]
mod tests {
    use super::Side::*;
    use super::*;

    const SCREEN: Rect = Rect {
        l: 0,
        t: 0,
        r: 1000,
        b: 800,
    };

    #[test]
    fn each_direction_when_it_fits() {
        let p = Rect::point(500, 400);
        assert_eq!(
            place(p, 100, 50, After, After, 0, SCREEN),
            (500, 400, false)
        );
        assert_eq!(
            place(p, 100, 50, Before, After, 0, SCREEN),
            (400, 400, false)
        );
        assert_eq!(
            place(p, 100, 50, After, Before, 0, SCREEN),
            (500, 350, true)
        );
        assert_eq!(
            place(p, 100, 50, Before, Before, 0, SCREEN),
            (400, 350, true)
        );
        // gap 两个轴都留
        assert_eq!(
            place(p, 10, 10, Before, After, 4, SCREEN),
            (486, 404, false)
        );
    }

    #[test]
    fn overflow_flips_to_the_other_side() {
        // 贴右边：翻到左侧
        assert_eq!(
            place(Rect::point(950, 400), 100, 50, After, After, 0, SCREEN),
            (850, 400, false)
        );
        // 贴底边：翻到上方，而且要报告「在上方」
        assert_eq!(
            place(Rect::point(500, 780), 100, 50, After, After, 0, SCREEN),
            (500, 730, true)
        );
        // 贴顶边选了上方：翻到下方
        assert_eq!(
            place(Rect::point(500, 10), 100, 50, After, Before, 0, SCREEN),
            (500, 10, false)
        );
    }

    #[test]
    fn box_right_top_flips_to_the_left_of_the_box() {
        let sel = Rect {
            l: 700,
            t: 100,
            r: 950,
            b: 300,
        };
        assert_eq!(
            place(sel, 100, 50, After, Start, 4, SCREEN),
            (596, 100, false)
        );
    }

    #[test]
    fn start_overflow_becomes_end() {
        // 选区贴底，顶齐平放不下 → 底齐平
        let sel = Rect {
            l: 100,
            t: 700,
            r: 300,
            b: 790,
        };
        assert_eq!(
            place(sel, 100, 200, After, Start, 4, SCREEN),
            (304, 590, false)
        );
    }

    #[test]
    fn clamps_when_neither_side_fits() {
        // 上下都放不下：原来那一侧夹回屏内
        assert_eq!(
            place(Rect::point(500, 400), 100, 600, After, After, 0, SCREEN),
            (500, 200, false)
        );
        assert_eq!(
            place(Rect::point(500, 400), 100, 600, After, Before, 0, SCREEN),
            (500, 0, true)
        );
        // 比屏幕还大：贴左上
        assert_eq!(
            place(Rect::point(500, 400), 2000, 900, After, After, 0, SCREEN),
            (0, 0, false)
        );
    }

    #[test]
    fn respects_a_bounds_that_does_not_start_at_zero() {
        // 副屏在主屏左边、工作区不含任务栏
        let work = Rect {
            l: -1920,
            t: 0,
            r: 0,
            b: 1040,
        };
        assert_eq!(
            place(Rect::point(-50, 1030), 320, 100, After, After, 0, work),
            (-370, 930, true)
        );
    }

    #[test]
    fn minimum_default_and_maximum_gaps_apply_on_both_axes_in_all_directions() {
        let anchor = Rect::point(500, 400);
        for gap in [0, 10, 20] {
            for (sx, sy, x, y, above) in [
                (After, After, 500 + gap, 400 + gap, false),
                (Before, After, 482 - gap, 400 + gap, false),
                (After, Before, 500 + gap, 382 - gap, true),
                (Before, Before, 482 - gap, 382 - gap, true),
            ] {
                assert_eq!(place(anchor, 18, 18, sx, sy, gap, SCREEN), (x, y, above));
            }
        }
    }

    #[test]
    fn maximum_gap_flips_at_screen_edges_and_negative_monitor_coordinates() {
        for (anchor, sx, sy, expected) in [
            (Rect::point(0, 0), Before, Before, (20, 20, false)),
            (Rect::point(999, 0), After, Before, (961, 20, false)),
            (Rect::point(0, 799), Before, After, (20, 761, true)),
            (Rect::point(999, 799), After, After, (961, 761, true)),
        ] {
            assert_eq!(place(anchor, 18, 18, sx, sy, 20, SCREEN), expected);
        }
        let left_monitor = Rect {
            l: -1920,
            t: -200,
            r: 0,
            b: 880,
        };
        assert_eq!(
            place(
                Rect::point(-1, -199),
                18,
                18,
                After,
                Before,
                20,
                left_monitor
            ),
            (-39, -179, false),
        );
    }

    #[test]
    fn normal_spacing_keeps_the_existing_dismissal_boundary() {
        for size in [18, 23, 27, 36] {
            for gap in [0, 4, 10] {
                for (sx, sy) in [
                    (Before, Before),
                    (Before, After),
                    (After, Before),
                    (After, After),
                ] {
                    let anchor = Rect::point(500, 400);
                    let (x, y, _) = place(anchor, size, size, sx, sy, gap, SCREEN);
                    let (cx, cy) = (x + size / 2, y + size / 2);
                    let limit = dismissal_limit_squared(anchor, cx, cy);
                    assert_eq!(limit, 60 * 60);
                    assert!(distance_squared(cx + 60, cy, cx, cy) <= limit);
                    assert!(distance_squared(cx + 61, cy, cx, cy) > limit);
                }
            }
        }
    }

    #[test]
    fn maximum_gap_allows_approach_but_still_dismisses_movement_away() {
        for anchor in [
            Rect::point(500, 400),
            Rect::point(0, 0),
            Rect::point(999, 799),
        ] {
            for size in [18, 23, 36] {
                for (sx, sy) in [
                    (Before, Before),
                    (Before, After),
                    (After, Before),
                    (After, After),
                ] {
                    let (x, y, _) = place(anchor, size, size, sx, sy, 20, SCREEN);
                    let (cx, cy) = (x + size / 2, y + size / 2);
                    let limit = dismissal_limit_squared(anchor, cx, cy);
                    // 从松开鼠标的位置到浮标中心，每一点都在范围内（含第一下移动和悬停目标）。
                    for step in 0..=100 {
                        let px = anchor.l + (cx - anchor.l) * step / 100;
                        let py = anchor.t + (cy - anchor.t) * step / 100;
                        assert!(distance_squared(px, py, cx, cy) <= limit);
                    }
                    let away_x = anchor.l - (cx - anchor.l).signum() * 20;
                    let away_y = anchor.t - (cy - anchor.t).signum() * 20;
                    assert!(distance_squared(away_x, away_y, cx, cy) > limit);
                }
            }
        }
    }

    #[test]
    fn dismissal_limit_uses_the_actual_clamped_icon_center() {
        let bounds = Rect {
            l: 0,
            t: 0,
            r: 50,
            b: 50,
        };
        let anchor = Rect::point(25, 25);
        let (x, y, _) = place(anchor, 36, 36, After, After, 20, bounds);
        assert_eq!((x, y), (14, 14));
        // 夹边后中心离得近，用标准 60px，而不是 36px 浮标、20px 间距需要的加大半径。
        assert_eq!(dismissal_limit_squared(anchor, x + 18, y + 18), 60 * 60);
    }

    #[test]
    fn distant_monitor_coordinates_do_not_overflow_the_hook_comparison() {
        assert_eq!(
            distance_squared(-40000, -40000, 40000, 40000),
            12_800_000_000
        );
        assert_eq!(
            distance_squared(i32::MIN, i32::MIN, i32::MAX, i32::MAX),
            u64::MAX
        );
    }
}
