//! 表情（2.0 架构第 7 节）：内置表情清单、导入的图片表情、表情轮盘设置的合法性。规则移植自 lolPing
//! `src/shared/emotes.ts`（MIT，© 2026 Danrui Meng）。
//!
//! 实现流程：
//! 1. 表情的引用（ref）：内置表情是 slug（`facepalm`），导入的是 `c:` 加 32 位十六进制（静态图 SHA-256 的前 16 字节）。
//!    队友发来的引用先按这个形状检查，再去找。
//! 2. 导入的表情最多 24 个，每个记名字（最多 24 个字）、有没有动画、有没有音效、出场效果（0–5）。文件在 `store.rs`。
//! 3. 轮盘是 8 个不同的、认识的表情；不认识的格子换回那一格的默认值（默认值已在轮盘上就换成清单里第一个没用到的）。
//! 4. 在线状态里带上自己轮盘和单击表情里的导入表情哈希（最多 9 个），没有这张图的队友用 Want 来要（`transfer.rs`）。

pub mod catalog;
pub mod net;
pub mod store;
pub mod transfer;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use catalog::SLUGS;

pub const DEFAULT_WHEEL: [&str; 8] = [
    "unworthy_cn",
    "poro_snax",
    "sad_kitten",
    "cheeky_poro",
    "angry_kitty",
    "gg_heart",
    "bee_happy",
    "bee_mad",
];
pub const DEFAULT_CLICK: &str = "facepalm";
pub const MAX_CUSTOM: usize = 24;
pub const NAME_MAX_CHARS: usize = 24;
/// 出场效果的种数：0 是普通弹出，另外 5 种。
pub const FX_VARIANTS: u8 = 6;
/// 在线状态里最多报几个哈希：轮盘 8 个加单击的 1 个。
pub const ANNOUNCED_MAX: usize = 9;

/// 一个导入的表情。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomEmote {
    pub id: String,
    pub name: String,
    /// 另存了原动画。
    pub animated: bool,
    /// 有音效（只在本机放，不传给队友）。
    pub sound: bool,
    pub fx: u8,
}

fn is_slug_shape(s: &str) -> bool {
    (1..=40).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

pub fn is_hash(s: &str) -> bool {
    s.len() == 32
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `c:` 加哈希。
pub fn custom_hash(r: &str) -> Option<&str> {
    r.strip_prefix("c:").filter(|h| is_hash(h))
}

/// 形状对（内置 slug 或导入的哈希），不管存不存在。
pub fn is_ref_shape(r: &str) -> bool {
    is_slug_shape(r) || custom_hash(r).is_some()
}

pub fn is_bundled(r: &str) -> bool {
    SLUGS.contains(&r)
}

fn is_known(r: &str, custom: &[CustomEmote]) -> bool {
    is_bundled(r) || (custom_hash(r).is_some() && custom.iter().any(|c| c.id == r))
}

/// 去掉控制字符和首尾空白，截到 24 个字；空了就用哈希的前 6 位。
fn clean_name(raw: &str, hash: &str) -> String {
    let name: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(NAME_MAX_CHARS)
        .collect();
    if name.is_empty() {
        hash[..6].to_string()
    } else {
        name
    }
}

/// 导入的表情：逐项检查，坏的丢掉，重复的只留第一个，最多 24 个。
pub fn normalize_custom(raw: Option<&Value>) -> Vec<CustomEmote> {
    let list = raw.and_then(Value::as_array).cloned().unwrap_or_default();
    let mut out: Vec<CustomEmote> = Vec::new();
    for v in list {
        let Some(id) = v.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(hash) = custom_hash(id) else {
            continue;
        };
        if out.iter().any(|c| c.id == id) {
            continue;
        }
        out.push(CustomEmote {
            id: id.to_string(),
            name: clean_name(v.get("name").and_then(Value::as_str).unwrap_or(""), hash),
            animated: v.get("animated").and_then(Value::as_bool).unwrap_or(false),
            sound: v.get("sound").and_then(Value::as_bool).unwrap_or(false),
            fx: v
                .get("fx")
                .and_then(Value::as_u64)
                .filter(|f| *f < u64::from(FX_VARIANTS))
                .unwrap_or(0) as u8,
        });
        if out.len() >= MAX_CUSTOM {
            break;
        }
    }
    out
}

/// 轮盘：8 个形状对、互不相同的引用，否则用默认轮盘；不认识的格子换掉（见文件头第 3 条）。
pub fn normalize_wheel(raw: Option<&Value>, custom: &[CustomEmote]) -> Vec<String> {
    let default: Vec<String> = DEFAULT_WHEEL.map(String::from).to_vec();
    let Some(list) = raw.and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok()) else {
        return default;
    };
    let distinct = list.iter().enumerate().all(|(i, r)| !list[..i].contains(r));
    if list.len() != DEFAULT_WHEEL.len() || !distinct || !list.iter().all(|r| is_ref_shape(r)) {
        return default;
    }
    let mut wheel = list;
    let mut used: Vec<String> = wheel
        .iter()
        .filter(|r| is_known(r, custom))
        .cloned()
        .collect();
    for i in 0..wheel.len() {
        if is_known(&wheel[i], custom) {
            continue;
        }
        let pick = if used.iter().any(|u| u == DEFAULT_WHEEL[i]) {
            SLUGS
                .iter()
                .find(|s| !used.iter().any(|u| u == *s))
                .copied()
                .unwrap_or(DEFAULT_WHEEL[i])
        } else {
            DEFAULT_WHEEL[i]
        };
        used.push(pick.to_string());
        wheel[i] = pick.to_string();
    }
    wheel
}

pub fn normalize_click(raw: Option<&Value>, custom: &[CustomEmote]) -> String {
    raw.and_then(Value::as_str)
        .filter(|r| is_ref_shape(r) && is_known(r, custom))
        .unwrap_or(DEFAULT_CLICK)
        .to_string()
}

/// 十六进制哈希 → 线上的 16 字节。
pub fn hash_bytes(h: &str) -> Option<[u8; 16]> {
    if !is_hash(h) {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// 在线状态里的哈希（16 字节连在一起）↔ 十六进制列表。
pub fn split_announced(b: &[u8]) -> Vec<String> {
    b.as_chunks::<16>()
        .0
        .iter()
        .take(ANNOUNCED_MAX)
        .map(|h| crate::crypto::hex(h))
        .collect()
}

pub fn join_announced(list: &[String]) -> Vec<u8> {
    list.iter()
        .filter_map(|h| hash_bytes(h))
        .flatten()
        .collect()
}

/// 在线状态里报的哈希：单击表情和轮盘上的导入表情，不重复，最多 9 个。
pub fn announced(wheel: &[String], click: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in std::iter::once(click).chain(wheel.iter().map(String::as_str)) {
        if let Some(h) = custom_hash(r)
            && !out.iter().any(|o| o == h)
        {
            out.push(h.to_string());
        }
        if out.len() >= ANNOUNCED_MAX {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const H: &str = "0123456789abcdef0123456789abcdef";

    fn custom(h: &str) -> CustomEmote {
        CustomEmote {
            id: format!("c:{h}"),
            name: "图".into(),
            animated: false,
            sound: false,
            fx: 0,
        }
    }

    #[test]
    fn ref_shapes() {
        assert!(is_ref_shape("facepalm") && is_ref_shape(&format!("c:{H}")));
        assert!(
            !is_ref_shape("Facepalm")
                && !is_ref_shape("c:xyz")
                && !is_ref_shape("")
                && !is_ref_shape(&"a".repeat(41))
        );
        assert!(!is_ref_shape(&format!("c:{}", H.to_uppercase())));
        assert!(is_bundled("facepalm") && !is_bundled("nope"));
        assert_eq!(SLUGS.len(), 44);
        for d in DEFAULT_WHEEL.iter().chain([&DEFAULT_CLICK]) {
            assert!(is_bundled(d), "{d}");
        }
    }

    #[test]
    fn custom_list_is_cleaned() {
        let h2 = "f".repeat(32);
        let raw = json!([
            { "id": format!("c:{H}"), "name": "  阿钢\u{7}的图  ", "animated": true, "sound": true, "fx": 3 },
            { "id": format!("c:{H}"), "name": "重复" },
            { "id": "c:bad", "name": "坏的" },
            { "id": format!("c:{h2}"), "name": "", "fx": 9 },
        ]);
        let list = normalize_custom(Some(&raw));
        assert_eq!(list.len(), 2);
        assert_eq!(
            (
                list[0].name.as_str(),
                list[0].animated,
                list[0].sound,
                list[0].fx
            ),
            ("阿钢的图", true, true, 3)
        );
        assert_eq!((list[1].name.as_str(), list[1].fx), ("ffffff", 0));
        let many: Vec<Value> = (0..30)
            .map(|i| json!({ "id": format!("c:{:032x}", i) }))
            .collect();
        assert_eq!(
            normalize_custom(Some(&Value::Array(many))).len(),
            MAX_CUSTOM
        );
    }

    #[test]
    fn wheel_keeps_known_refs_and_replaces_the_rest() {
        let mine = [custom(H)];
        let mut w: Vec<String> = DEFAULT_WHEEL.map(String::from).to_vec();
        w[2] = format!("c:{H}");
        assert_eq!(normalize_wheel(Some(&json!(w)), &mine), w);
        // 导入的表情删掉了：那一格换回默认值
        assert_eq!(normalize_wheel(Some(&json!(w)), &[])[2], DEFAULT_WHEEL[2]);
        // 默认值已经在别的格子里：换成清单里第一个没用到的
        let mut dup: Vec<String> = DEFAULT_WHEEL.map(String::from).to_vec();
        dup[1] = "unworthy_cn".into(); // 第 0 格的默认值放到了第 1 格
        dup[0] = format!("c:{H}"); // 第 0 格是删掉了的导入表情
        let fixed = normalize_wheel(Some(&json!(dup)), &[]);
        assert_eq!(fixed[0], "facepalm", "清单里第一个没用到的");
        assert!(
            fixed
                .iter()
                .enumerate()
                .all(|(i, r)| !fixed[..i].contains(r))
        );
        // 形状不对、重复、数目不对：整个换成默认
        for bad in [
            json!(vec!["facepalm"; 8]),
            json!(["facepalm"]),
            json!(["Bad!", "a", "b", "c", "d", "e", "f", "g"]),
        ] {
            assert_eq!(
                normalize_wheel(Some(&bad), &[]),
                DEFAULT_WHEEL.map(String::from).to_vec()
            );
        }
    }

    #[test]
    fn click_emote_must_exist() {
        assert_eq!(normalize_click(Some(&json!("wahaha")), &[]), "wahaha");
        assert_eq!(
            normalize_click(Some(&json!(format!("c:{H}"))), &[]),
            DEFAULT_CLICK
        );
        assert_eq!(
            normalize_click(Some(&json!(format!("c:{H}"))), &[custom(H)]),
            format!("c:{H}")
        );
    }

    #[test]
    fn announced_hashes_are_distinct_and_capped() {
        let wheel: Vec<String> = (0..8).map(|i| format!("c:{:032x}", i)).collect();
        let list = announced(&wheel, &format!("c:{:032x}", 3));
        assert_eq!(list.len(), 8, "单击的和轮盘上的重复");
        assert_eq!(list[0], format!("{:032x}", 3));
        assert!(announced(&DEFAULT_WHEEL.map(String::from), DEFAULT_CLICK).is_empty());
        let wire = join_announced(&list);
        assert_eq!(wire.len(), 16 * 8);
        assert_eq!(split_announced(&wire), list);
        assert_eq!(
            hash_bytes(H).map(|b| crate::crypto::hex(&b)),
            Some(H.to_string())
        );
        assert_eq!(hash_bytes("xyz"), None);
    }
}
