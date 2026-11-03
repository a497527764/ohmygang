//! macOS 和 X11 的按键翻译成 Windows 虚拟键码（手势状态机和设置都用 Windows 键码）。纯函数，每个平台都测。
//! 移植自 lolPing `native/hook-helper/src/macinput.cpp`、`linuxinput.cpp`（MIT，© 2026 Danrui Meng）。
//!
//! 实现流程：
//! 1. macOS：系统给的是 kVK_* 键码，查表换成 Windows 键码；回放时反查。修饰键不单独来按键事件，
//!    只来「修饰键状态变了」，所以按新旧状态算出哪些修饰键松开、哪些按下（先松后按），当成按键交给状态机。
//!    状态里只有「有修饰键按着」没有左右时（别的软件合成的事件），算左边那个。
//! 2. X11：先按物理位置的名字（AE01、AD01……，和键盘布局无关）认字母数字和符号键，认不出再按 keysym。

#![cfg_attr(windows, allow(dead_code))]

// ---------- macOS ----------

/// kVK_*（Carbon Events.h）→ Windows 键码。修饰键也列上，回放时能反查。
const MAC_KEYS: &[(u16, u32)] = &[
    (0x00, 0x41),
    (0x0B, 0x42),
    (0x08, 0x43),
    (0x02, 0x44),
    (0x0E, 0x45),
    (0x03, 0x46),
    (0x05, 0x47),
    (0x04, 0x48),
    (0x22, 0x49),
    (0x26, 0x4A),
    (0x28, 0x4B),
    (0x25, 0x4C),
    (0x2E, 0x4D),
    (0x2D, 0x4E),
    (0x1F, 0x4F),
    (0x23, 0x50),
    (0x0C, 0x51),
    (0x0F, 0x52),
    (0x01, 0x53),
    (0x11, 0x54),
    (0x20, 0x55),
    (0x09, 0x56),
    (0x0D, 0x57),
    (0x07, 0x58),
    (0x10, 0x59),
    (0x06, 0x5A),
    (0x1D, 0x30),
    (0x12, 0x31),
    (0x13, 0x32),
    (0x14, 0x33),
    (0x15, 0x34),
    (0x17, 0x35),
    (0x16, 0x36),
    (0x1A, 0x37),
    (0x1C, 0x38),
    (0x19, 0x39),
    // F1–F20
    (0x7A, 0x70),
    (0x78, 0x71),
    (0x63, 0x72),
    (0x76, 0x73),
    (0x60, 0x74),
    (0x61, 0x75),
    (0x62, 0x76),
    (0x64, 0x77),
    (0x65, 0x78),
    (0x6D, 0x79),
    (0x67, 0x7A),
    (0x6F, 0x7B),
    (0x69, 0x7C),
    (0x6B, 0x7D),
    (0x71, 0x7E),
    (0x6A, 0x7F),
    (0x40, 0x80),
    (0x4F, 0x81),
    (0x50, 0x82),
    (0x5A, 0x83),
    // 回车、小键盘回车、Tab、空格、退格、Esc、Delete、Help（当 Insert）
    (0x24, 0x0D),
    (0x4C, 0x0D),
    (0x30, 0x09),
    (0x31, 0x20),
    (0x33, 0x08),
    (0x35, 0x1B),
    (0x75, 0x2E),
    (0x72, 0x2D),
    // Home End PgUp PgDn，方向键
    (0x73, 0x24),
    (0x77, 0x23),
    (0x74, 0x21),
    (0x79, 0x22),
    (0x7B, 0x25),
    (0x7E, 0x26),
    (0x7C, 0x27),
    (0x7D, 0x28),
    // ` - = [ ] \ ; ' , . /
    (0x32, 0xC0),
    (0x1B, 0xBD),
    (0x18, 0xBB),
    (0x21, 0xDB),
    (0x1E, 0xDD),
    (0x2A, 0xDC),
    (0x29, 0xBA),
    (0x27, 0xDE),
    (0x2B, 0xBC),
    (0x2F, 0xBE),
    (0x2C, 0xBF),
    // 小键盘 0–9 和 * + - . /
    (0x52, 0x60),
    (0x53, 0x61),
    (0x54, 0x62),
    (0x55, 0x63),
    (0x56, 0x64),
    (0x57, 0x65),
    (0x58, 0x66),
    (0x59, 0x67),
    (0x5B, 0x68),
    (0x5C, 0x69),
    (0x43, 0x6A),
    (0x45, 0x6B),
    (0x4E, 0x6D),
    (0x41, 0x6E),
    (0x4B, 0x6F),
    // Caps Lock，左右 Shift、Control、Option、Command
    (0x39, 0x14),
    (0x38, 0xA0),
    (0x3C, 0xA1),
    (0x3B, 0xA2),
    (0x3E, 0xA3),
    (0x3A, 0xA4),
    (0x3D, 0xA5),
    (0x37, 0x5B),
    (0x36, 0x5C),
];

pub fn mac_key_to_vk(code: u16) -> u32 {
    MAC_KEYS
        .iter()
        .find(|(c, _)| *c == code)
        .map_or(0, |(_, vk)| *vk)
}

pub fn vk_to_mac_key(vk: u32) -> Option<u16> {
    MAC_KEYS.iter().find(|(_, v)| *v == vk).map(|(c, _)| *c)
}

/// CGEventFlags 的位：不分左右的四个，和分左右的 NX_DEVICE* 位。
pub const FLAG_SHIFT: u64 = 0x0002_0000;
pub const FLAG_CONTROL: u64 = 0x0004_0000;
pub const FLAG_OPTION: u64 = 0x0008_0000;
pub const FLAG_COMMAND: u64 = 0x0010_0000;
const DEV_LCTRL: u64 = 0x0001;
const DEV_LSHIFT: u64 = 0x0002;
const DEV_RSHIFT: u64 = 0x0004;
const DEV_LCMD: u64 = 0x0008;
const DEV_RCMD: u64 = 0x0010;
const DEV_LALT: u64 = 0x0020;
const DEV_RALT: u64 = 0x0040;
const DEV_RCTRL: u64 = 0x2000;
/// 和修饰键有关的全部位。
pub const MODIFIER_FLAGS: u64 = FLAG_SHIFT
    | FLAG_CONTROL
    | FLAG_OPTION
    | FLAG_COMMAND
    | DEV_LCTRL
    | DEV_LSHIFT
    | DEV_RSHIFT
    | DEV_LCMD
    | DEV_RCMD
    | DEV_LALT
    | DEV_RALT
    | DEV_RCTRL;

/// (键码, 不分左右的位, 自己这边的位, 另一边的位)
const MAC_MODS: [(u32, u64, u64, u64); 8] = [
    (0xA0, FLAG_SHIFT, DEV_LSHIFT, DEV_RSHIFT),
    (0xA1, FLAG_SHIFT, DEV_RSHIFT, DEV_LSHIFT),
    (0xA2, FLAG_CONTROL, DEV_LCTRL, DEV_RCTRL),
    (0xA3, FLAG_CONTROL, DEV_RCTRL, DEV_LCTRL),
    (0xA4, FLAG_OPTION, DEV_LALT, DEV_RALT),
    (0xA5, FLAG_OPTION, DEV_RALT, DEV_LALT),
    (0x5B, FLAG_COMMAND, DEV_LCMD, DEV_RCMD),
    (0x5C, FLAG_COMMAND, DEV_RCMD, DEV_LCMD),
];

pub fn is_modifier_vk(vk: u32) -> bool {
    (0xA0..=0xA5).contains(&vk) || vk == 0x5B || vk == 0x5C
}

/// 按 `flags`，修饰键 `vk` 是不是按着（见文件头第 1 条）。
pub fn mac_modifier_held(flags: u64, vk: u32) -> bool {
    let Some(&(_, group, own, other)) = MAC_MODS.iter().find(|m| m.0 == vk) else {
        return false;
    };
    if flags & group == 0 {
        return false;
    }
    if flags & own != 0 {
        return true;
    }
    matches!(vk, 0xA0 | 0xA2 | 0xA4 | 0x5B) && flags & other == 0
}

/// 从 `old` 变到 `new` 的修饰键：先全部松开的，再全部按下的。(键码, 是不是按下)
pub fn mac_modifier_transitions(old: u64, new: u64) -> Vec<(u32, bool)> {
    let mut ups = Vec::new();
    let mut downs = Vec::new();
    for &(vk, ..) in &MAC_MODS {
        match (mac_modifier_held(old, vk), mac_modifier_held(new, vk)) {
            (true, false) => ups.push((vk, false)),
            (false, true) => downs.push((vk, true)),
            _ => {}
        }
    }
    ups.extend(downs);
    ups
}

// ---------- X11 ----------

/// XKB 的物理键名（四个字符，比如 AE01、AC05、TLDE）→ Windows 键码；不是字母数字和符号键的返回 0。
pub fn x11_key_name_to_vk(name: &[u8]) -> u32 {
    const ROWS: [(&[u8; 2], &[u8]); 4] = [
        (b"AE", b"1234567890"),
        (b"AD", b"QWERTYUIOP"),
        (b"AC", b"ASDFGHJKL"),
        (b"AB", b"ZXCVBNM"),
    ];
    if name.len() >= 4 && name[2].is_ascii_digit() && name[3].is_ascii_digit() {
        let col = usize::from(name[2] - b'0') * 10 + usize::from(name[3] - b'0');
        for (row, keys) in ROWS {
            if name[..2] == row[..] && (1..=keys.len()).contains(&col) {
                return u32::from(keys[col - 1]);
            }
        }
    }
    const NAMED: [(&[u8; 4], u32); 11] = [
        (b"TLDE", 0xC0),
        (b"AE11", 0xBD),
        (b"AE12", 0xBB),
        (b"AD11", 0xDB),
        (b"AD12", 0xDD),
        (b"BKSL", 0xDC),
        (b"AC10", 0xBA),
        (b"AC11", 0xDE),
        (b"AB08", 0xBC),
        (b"AB09", 0xBE),
        (b"AB10", 0xBF),
    ];
    NAMED
        .iter()
        .find(|(n, _)| name.starts_with(&n[..]))
        .map_or(0, |(_, vk)| *vk)
}

/// X keysym → Windows 键码；认不出返回 0。
pub fn x11_keysym_to_vk(s: u32) -> u32 {
    match s {
        0x61..=0x7A => 0x41 + s - 0x61,       // a–z
        0x41..=0x5A => s,                     // A–Z
        0x30..=0x39 => s,                     // 0–9
        0xFFBE..=0xFFD5 => 0x70 + s - 0xFFBE, // F1–F24
        0xFFB0..=0xFFB9 => 0x60 + s - 0xFFB0, // 小键盘 0–9
        0xFFE1 => 0xA0,                       // Shift_L
        0xFFE2 => 0xA1,                       // Shift_R
        0xFFE3 => 0xA2,                       // Control_L
        0xFFE4 => 0xA3,                       // Control_R
        0xFFE9 => 0xA4,                       // Alt_L
        0xFFEA => 0xA5,                       // Alt_R
        0xFFEB => 0x5B,                       // Super_L
        0xFFEC => 0x5C,                       // Super_R
        0xFF1B => 0x1B,                       // Escape
        0xFF09 | 0xFE20 => 0x09,              // Tab、ISO_Left_Tab
        0xFF0D | 0xFF8D => 0x0D,              // Return、KP_Enter
        0x20 => 0x20,                         // space
        0xFF08 => 0x08,                       // BackSpace
        0xFFFF | 0xFF9F => 0x2E,              // Delete
        0xFF63 | 0xFF9E => 0x2D,              // Insert
        0xFF50 | 0xFF95 => 0x24,              // Home
        0xFF57 | 0xFF9C => 0x23,              // End
        0xFF55 | 0xFF9A => 0x21,              // Page_Up
        0xFF56 | 0xFF9B => 0x22,              // Page_Down
        0xFF51 | 0xFF96 => 0x25,              // Left
        0xFF52 | 0xFF97 => 0x26,              // Up
        0xFF53 | 0xFF98 => 0x27,              // Right
        0xFF54 | 0xFF99 => 0x28,              // Down
        0xFFE5 => 0x14,                       // Caps_Lock
        0xFF61 => 0x2C,                       // Print
        0xFF14 => 0x91,                       // Scroll_Lock
        0xFF13 => 0x13,                       // Pause
        0xFF7F => 0x90,                       // Num_Lock
        0xFF67 => 0x5D,                       // Menu
        0xFFAA => 0x6A,                       // KP_Multiply
        0xFFAB => 0x6B,                       // KP_Add
        0xFFAD => 0x6D,                       // KP_Subtract
        0xFFAE => 0x6E,                       // KP_Decimal
        0xFFAF => 0x6F,                       // KP_Divide
        0x3B => 0xBA,                         // ;
        0x3D => 0xBB,                         // =
        0x2C => 0xBC,                         // ,
        0x2D => 0xBD,                         // -
        0x2E => 0xBE,                         // .
        0x2F => 0xBF,                         // /
        0x60 => 0xC0,                         // `
        0x5B => 0xDB,                         // [
        0x5C => 0xDC,                         // \
        0x5D => 0xDD,                         // ]
        0x27 => 0xDE,                         // '
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_keys_round_trip() {
        assert_eq!(mac_key_to_vk(0x11), 0x54, "T");
        assert_eq!(mac_key_to_vk(0x30), 0x09, "Tab");
        assert_eq!(mac_key_to_vk(0x3A), 0xA4, "左 Option 是 Alt");
        assert_eq!(mac_key_to_vk(0xFF), 0);
        for &(code, vk) in MAC_KEYS {
            if vk != 0x0D {
                assert_eq!(vk_to_mac_key(vk), Some(code), "{vk:#x}");
            }
        }
        assert_eq!(vk_to_mac_key(0x0D), Some(0x24), "回车反查到主键盘的回车");
        assert_eq!(vk_to_mac_key(0x07), None);
    }

    #[test]
    fn mac_modifier_flags() {
        assert!(mac_modifier_held(FLAG_OPTION | DEV_LALT, 0xA4));
        assert!(!mac_modifier_held(FLAG_OPTION | DEV_LALT, 0xA5));
        assert!(
            mac_modifier_held(FLAG_OPTION, 0xA4),
            "只有不分左右的位：算左边"
        );
        assert!(!mac_modifier_held(FLAG_OPTION, 0xA5));
        assert!(
            !mac_modifier_held(DEV_LALT, 0xA4),
            "不分左右的位没了就是松开"
        );
        assert!(mac_modifier_held(FLAG_COMMAND | DEV_RCMD, 0x5C));
        let t = mac_modifier_transitions(FLAG_SHIFT | DEV_LSHIFT, FLAG_CONTROL | DEV_LCTRL);
        assert_eq!(t, vec![(0xA0, false), (0xA2, true)], "先松后按");
        assert!(mac_modifier_transitions(FLAG_SHIFT, FLAG_SHIFT).is_empty());
    }

    #[test]
    fn x11_names_and_keysyms() {
        assert_eq!(x11_key_name_to_vk(b"AE01"), 0x31);
        assert_eq!(x11_key_name_to_vk(b"AC05"), 0x47, "G 的物理位置");
        assert_eq!(x11_key_name_to_vk(b"AB07"), 0x4D);
        assert_eq!(x11_key_name_to_vk(b"AB08"), 0xBC, "逗号");
        assert_eq!(x11_key_name_to_vk(b"TLDE"), 0xC0);
        assert_eq!(x11_key_name_to_vk(b"RTRN"), 0);
        assert_eq!(x11_keysym_to_vk(0x74), 0x54, "t");
        assert_eq!(x11_keysym_to_vk(0x54), 0x54, "T");
        assert_eq!(x11_keysym_to_vk(0xFF09), 0x09);
        assert_eq!(x11_keysym_to_vk(0xFFBF), 0x71, "F2");
        assert_eq!(x11_keysym_to_vk(0xFFE9), 0xA4);
        assert_eq!(x11_keysym_to_vk(0x1234), 0);
    }
}
