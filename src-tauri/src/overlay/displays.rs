//! 显示器表（架构 3.1）：编号、物理像素 → 窗口内 CSS 像素、透明窗口放在哪、空闲时挪到哪。纯函数。
//! 编号和挪开的做法移植自 lolPing `src/main/displayNumbers.ts`、`overlayManager.ts`（MIT，© 2026 Danrui Meng）。
//!
//! 坐标都是物理像素（钩子给的就是物理像素）。每块屏幕有自己的缩放，窗口内的 CSS 像素 = 物理像素 ÷ 缩放。

/// 物理像素的矩形。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }

    /// 点到矩形的距离的平方（在里面是 0）。
    fn dist_sq(&self, x: i32, y: i32) -> i64 {
        let dx = i64::from((self.x - x).max(0).max(x - (self.x + self.width - 1)));
        let dy = i64::from((self.y - y).max(0).max(y - (self.y + self.height - 1)));
        dx * dx + dy * dy
    }
}

/// 一块显示器。`key` 是系统给的名字（如 `\\.\DISPLAY1`），用来在显示器变化后认出同一块屏幕。
#[derive(Clone, Debug, PartialEq)]
pub struct Display {
    pub key: String,
    pub rect: Rect,
    pub scale: f64,
    pub primary: bool,
}

/// 队伍里用的编号：主显示器是 1 号，其余从左到右、再从上到下。返回按编号排好的表（下标 + 1 就是编号）。
pub fn numbered(displays: &[Display]) -> Vec<Display> {
    let mut rest: Vec<Display> = displays.iter().filter(|d| !d.primary).cloned().collect();
    rest.sort_by_key(|d| (d.rect.x, d.rect.y));
    displays
        .iter()
        .filter(|d| d.primary)
        .cloned()
        .chain(rest)
        .collect()
}

/// 点所在的显示器；不在任何一块上时取最近的一块。
pub fn display_at(displays: &[Display], x: i32, y: i32) -> Option<usize> {
    displays
        .iter()
        .position(|d| d.rect.contains(x, y))
        .or_else(|| (0..displays.len()).min_by_key(|&i| displays[i].rect.dist_sq(x, y)))
}

/// 物理像素的点在某块显示器的透明窗口里的 CSS 像素位置。轮盘拖出这块屏幕时也按它算，方向不变。
pub fn to_local(d: &Display, x: i32, y: i32) -> (f64, f64) {
    (
        f64::from(x - d.rect.x) / d.scale,
        f64::from(y - d.rect.y) / d.scale,
    )
}

/// 透明窗口盖在显示器上的位置。底边少一个像素：盖满整块屏幕的窗口会被资源管理器当成全屏程序，
/// 任务栏就沉下去、自动隐藏的任务栏也滑不出来了。
pub fn overlay_rect(d: &Display) -> Rect {
    Rect {
        height: d.rect.height - 1,
        ..d.rect
    }
}

/// 空闲时窗口缩成哪：显示器上边一条（一个 CSS 像素对应的物理像素那么高），这样窗口还算在这块屏幕上、缩放不变，
/// 截图工具也不会把它当成要截的窗口。3.2.0 以前是整块屏幕大小挪到屏幕外只露这一条；露出来的一样，
/// 但窗口本身小了，WebView2 不用一直留着整屏大小的显卡缓冲。这一条在自己的显示器里，不会压到别的显示器。
pub fn parked_rect(d: &Display) -> Rect {
    let o = overlay_rect(d);
    Rect {
        height: d.scale.ceil() as i32,
        ..o
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disp(key: &str, x: i32, y: i32, w: i32, h: i32, scale: f64, primary: bool) -> Display {
        Display {
            key: key.into(),
            rect: Rect {
                x,
                y,
                width: w,
                height: h,
            },
            scale,
            primary,
        }
    }

    #[test]
    fn primary_is_one_then_left_to_right_then_top_to_bottom() {
        let list = [
            disp("right", 2560, 0, 1920, 1080, 1.0, false),
            disp("main", 0, 0, 2560, 1440, 1.5, true),
            disp("left-low", -1920, 500, 1920, 1080, 1.0, false),
            disp("left-high", -1920, -580, 1920, 1080, 1.0, false),
        ];
        let keys: Vec<_> = numbered(&list).into_iter().map(|d| d.key).collect();
        assert_eq!(keys, ["main", "left-high", "left-low", "right"]);
    }

    #[test]
    fn point_maps_to_the_display_under_it_or_the_nearest() {
        let list = [
            disp("a", 0, 0, 2560, 1600, 1.5, true),
            disp("b", 2560, 0, 1920, 1080, 1.0, false),
        ];
        assert_eq!(display_at(&list, 100, 100), Some(0));
        assert_eq!(display_at(&list, 2560, 10), Some(1));
        assert_eq!(display_at(&list, 3000, 1500), Some(1)); // b 下方的空白处：最近的是 b
        assert_eq!(display_at(&[], 0, 0), None);
    }

    #[test]
    fn local_coordinates_divide_by_the_display_scale() {
        let d = disp("b", 2560, -200, 1920, 1080, 1.25, false);
        assert_eq!(to_local(&d, 2560 + 500, -200 + 250), (400.0, 200.0));
        assert_eq!(to_local(&d, 2560 - 125, 0), (-100.0, 160.0)); // 拖出了这块屏幕
    }

    #[test]
    fn overlay_stops_one_pixel_short_of_the_bottom() {
        let d = disp("a", 0, 0, 2560, 1600, 1.5, true);
        assert_eq!(
            overlay_rect(&d),
            Rect {
                x: 0,
                y: 0,
                width: 2560,
                height: 1599
            }
        );
    }

    #[test]
    fn parked_overlay_is_a_strip_on_its_own_top_edge() {
        let a = disp("a", 0, 0, 2560, 1600, 1.5, true);
        // 上边一条，2 个物理像素高（1.5 倍向上取整），整个窗口就这么大
        assert_eq!(
            parked_rect(&a),
            Rect {
                x: 0,
                y: 0,
                width: 2560,
                height: 2
            }
        );
        // 左边的副屏是负坐标；1 倍缩放只留 1 个像素
        let b = disp("b", -1920, 200, 1920, 1080, 1.0, false);
        assert_eq!(
            parked_rect(&b),
            Rect {
                x: -1920,
                y: 200,
                width: 1920,
                height: 1
            }
        );
    }
}
