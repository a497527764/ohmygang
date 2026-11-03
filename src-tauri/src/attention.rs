//! 新消息提醒（V1.1）：窗口不在眼前时，让用户知道来了新消息。
//!
//! 实现流程：
//! 1. 节点通知里有「要提醒」的新消息（规则在 `chat::should_alert`）时，在主线程上调用 `alert`：
//!    - 窗口在前台（可见、没最小化、有焦点）：什么都不做，界面自己会显示；
//!    - 窗口可见但在别的窗口后面，或者最小化了：任务栏按钮闪烁（Tauri 的 request_user_attention）；
//!    - 窗口收进了托盘：托盘图标右下角加一个红点，提示文字改成「钢一下：有新消息」。
//! 2. 窗口获得焦点（包括从托盘点开）时调用 `clear`：托盘图标和提示文字恢复，任务栏停止闪烁。
//! 3. 红点图标在运行时由应用图标画出：复制像素，在右下角画一个白边红点（`with_dot`，纯函数，有单元测试）。
//!
//! 不用系统通知插件，也不给前端开任何权限。

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::image::Image;
use tauri::{AppHandle, Manager, UserAttentionType};

pub const TRAY_ID: &str = "main";
pub const TOOLTIP: &str = "钢一下";
const TOOLTIP_UNREAD: &str = "钢一下：有新消息";
/// 红点颜色，和界面里的未读角标一样。
const DOT: [u8; 4] = [0xef, 0x4f, 0x6c, 0xff];
const RING: [u8; 4] = [0xff, 0xff, 0xff, 0xff];

/// 托盘图标现在是不是带红点，避免每条消息都重画一次。
static DOTTED: AtomicBool = AtomicBool::new(false);

/// 在 RGBA 图标的右下角画一个白边红点，返回新的像素（宽高不变）。
pub fn with_dot(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut out = rgba.to_vec();
    let size = width.min(height) as f32;
    let r = size * 0.24;
    let ring = r + size * 0.06;
    let (cx, cy) = (width as f32 - ring, height as f32 - ring);
    for y in 0..height {
        for x in 0..width {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let d = (dx * dx + dy * dy).sqrt();
            let color = if d <= r {
                DOT
            } else if d <= ring {
                RING
            } else {
                continue;
            };
            let i = ((y * width + x) * 4) as usize;
            out[i..i + 4].copy_from_slice(&color);
        }
    }
    out
}

/// 来了要提醒的新消息。要在主线程上调用（窗口状态查询在那里不会卡住节点）。
pub fn alert(app: &AppHandle) {
    let Some(w) = app.get_webview_window("main") else {
        return;
    };
    let visible = w.is_visible().unwrap_or(false);
    let minimized = w.is_minimized().unwrap_or(false);
    let focused = w.is_focused().unwrap_or(false);
    if visible && !minimized && focused {
        return;
    }
    if visible {
        let _ = w.request_user_attention(Some(UserAttentionType::Informational));
        return;
    }
    if DOTTED.swap(true, Ordering::Relaxed) {
        return;
    }
    if let (Some(tray), Some(icon)) = (app.tray_by_id(TRAY_ID), app.default_window_icon()) {
        let (width, height) = (icon.width(), icon.height());
        let dotted = Image::new_owned(with_dot(icon.rgba(), width, height), width, height);
        let _ = tray.set_icon(Some(dotted));
        let _ = tray.set_tooltip(Some(TOOLTIP_UNREAD));
    }
}

/// 用户回到窗口：恢复托盘图标，停止闪烁。
pub fn clear(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.request_user_attention(None);
    }
    if !DOTTED.swap(false, Ordering::Relaxed) {
        return;
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_icon(app.default_window_icon().cloned());
        let _ = tray.set_tooltip(Some(TOOLTIP));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * width + x) * 4) as usize;
        rgba[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn dot_is_drawn_in_the_bottom_right_corner_only() {
        let (w, h) = (32u32, 32u32);
        let base: Vec<u8> = [10u8, 20, 30, 255].repeat((w * h) as usize);
        let out = with_dot(&base, w, h);
        assert_eq!(out.len(), base.len());
        // 红点中心在右下角
        assert_eq!(pixel(&out, w, 24, 24), DOT);
        // 左上角不变
        assert_eq!(pixel(&out, w, 2, 2), [10, 20, 30, 255]);
        // 红点外圈是白边
        assert!(out.chunks(4).any(|p| p == RING));
    }
}
