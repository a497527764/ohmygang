//! 轮盘几何：选格。移植自 lolPing `src/shared/geometry.ts`（MIT，© 2026 Danrui Meng）。
//! 画轮盘的 SVG（`src/overlay/wheel.ts`）用同一组尺寸，所以这里算出的格子和屏幕上高亮的格子一致。

/// 轮盘 SVG 的设计尺寸（CSS 像素）：中心圆半径 86，环 86–188，正方向的四格延伸到 288。
pub const WHEEL_SIZE: f64 = 600.0;
pub const INNER_R: f64 = 86.0;
pub const SLOTS: usize = 8;

/// 相对轮盘中心的偏移（屏幕坐标，y 向下）落在哪一格：在死区内返回 None，否则 0..8，0 是正上方，顺时针数。
/// 每格 45°，以正方向为中心。
pub fn pick_slice(dx: f64, dy: f64, dead_zone: f64) -> Option<usize> {
    if dx.hypot(dy) < dead_zone {
        return None;
    }
    let deg = (dy.atan2(dx).to_degrees() + 90.0 + 22.5 + 360.0) % 360.0;
    Some((deg / 45.0) as usize % SLOTS)
}

/// 轮盘按设置的大小缩放时，死区跟着缩放。
pub fn dead_zone(wheel_size_px: f64) -> f64 {
    INNER_R * wheel_size_px / WHEEL_SIZE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_the_dead_zone_is_no_slice() {
        assert_eq!(pick_slice(0.0, 0.0, INNER_R), None);
        assert_eq!(pick_slice(60.0, 60.0, INNER_R), None); // 84.9 < 86
        assert_eq!(pick_slice(0.0, -86.0, INNER_R), Some(0)); // 正好在边上算进第一格
    }

    #[test]
    fn compass_directions_go_clockwise_from_north() {
        let r = 120.0;
        let d = r / 2f64.sqrt();
        assert_eq!(pick_slice(0.0, -r, INNER_R), Some(0)); // 上
        assert_eq!(pick_slice(d, -d, INNER_R), Some(1)); // 右上
        assert_eq!(pick_slice(r, 0.0, INNER_R), Some(2)); // 右
        assert_eq!(pick_slice(d, d, INNER_R), Some(3)); // 右下
        assert_eq!(pick_slice(0.0, r, INNER_R), Some(4)); // 下
        assert_eq!(pick_slice(-d, d, INNER_R), Some(5)); // 左下
        assert_eq!(pick_slice(-r, 0.0, INNER_R), Some(6)); // 左
        assert_eq!(pick_slice(-d, -d, INNER_R), Some(7)); // 左上
    }

    #[test]
    fn slices_are_centred_on_their_direction() {
        // 正上方左右各 22.5° 以内都是第 0 格。
        let at = |deg: f64| {
            let rad = (deg - 90.0).to_radians();
            pick_slice(200.0 * rad.cos(), 200.0 * rad.sin(), INNER_R)
        };
        assert_eq!(at(-22.0), Some(0));
        assert_eq!(at(22.0), Some(0));
        assert_eq!(at(23.0), Some(1));
        assert_eq!(at(-23.0), Some(7));
    }

    #[test]
    fn dead_zone_scales_with_the_wheel() {
        assert_eq!(dead_zone(600.0), 86.0);
        assert_eq!(dead_zone(300.0), 43.0);
    }
}
