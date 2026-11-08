//! 实时帧（2.0 架构第 5 节）：在线状态、信号、表情和导入表情的传输、文字、投票。
//! 不入库、不补齐，丢了就丢了。做法参考 lolPing 设计文档 12 节（MIT，© 2026 Danrui Meng）。
//!
//! 实现流程：
//! 1. 线上是 `WireLive`：`[发送者公钥, seq, ts_ms, body, sig]`，和事件一样装在队伍密钥加密的包里。
//!    body 是 CBOR 编码的 `LiveBody`；sig = Ed25519(发送者私钥, "hs/sig/v1" ‖ SHA-256("gang/live/v2" ‖ 队伍 ‖ 公钥 ‖ seq ‖ ts ‖ body))。
//!    包已经加密了，签名防的是队伍里的人冒充别人发信号。
//! 2. 收端：验签 → 发送者必须是队员（由调用方查）→ `ReplayGuard` 挡重放：时间差超过 ±10 分钟的不要；
//!    每个发送者记住最近 64 个序号，见过的不要、比这些都旧的不要。同一帧从局域网和中继各来一份时，第二份就被挡掉。
//! 3. 发送方的 seq 用「微秒时间戳，且比上一个大」：重启后也比以前的大，不用存。
//! 4. 位置用「显示器号 + 0–65535 的比例」，落到收的人同号显示器的同一比例处（2.0 架构 3.1）。
//! 5. 和 2.0 互通（3.0 去掉了对线模式，协议版本不变）：2.0 的鼠标帧（C）、充能帧（K）在这里是不认识的帧，丢掉；
//!    在线状态里 2.0 必须有的「对线」一项照发，总是 false。

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::crypto::{self, Identity, MemberId, RoomId};
use crate::protocol::body;

/// 收发两端的时钟差超过这么多的帧不要。
pub const MAX_SKEW_MS: u64 = 10 * 60 * 1000;
/// 每个发送者记住的序号个数。
const WINDOW: usize = 64;

/// 线上的实时帧：[发送者公钥, seq, ts_ms, body, sig]。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireLive(
    #[serde(with = "serde_bytes")] pub MemberId,
    pub u64,
    pub u64,
    #[serde(with = "serde_bytes")] pub Vec<u8>,
    #[serde(with = "serde_bytes")] pub [u8; 64],
);

/// 在线状态里的状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    #[serde(rename = "o")]
    Online,
    /// 信号暂停了。
    #[serde(rename = "p")]
    Paused,
    /// 5 分钟没碰键鼠。
    #[serde(rename = "a")]
    Away,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LiveBody {
    /// 在线状态：局域网 2 秒、中继 5 秒一次。
    #[serde(rename = "P")]
    Presence {
        #[serde(rename = "s")]
        status: Status,
        /// 2.0 的「开着对线模式」。3.0 没有对线，总是 false；2.0 解码在线状态时要有这一项，所以留着。
        #[serde(rename = "l")]
        laning: bool,
        #[serde(rename = "v")]
        version: String,
        /// win、mac、linux。
        #[serde(rename = "o")]
        platform: String,
        /// 轮盘上导入表情的哈希，每个 16 字节连在一起，最多 9 个（没有这张图的队友来要）。
        #[serde(rename = "e", default, with = "serde_bytes")]
        emotes: Vec<u8>,
        /// 关掉了的功能：`OFF_TEXT`、`OFF_VOTES` 按位或。关了投票的人不算投票人。
        #[serde(rename = "f", default)]
        off: u8,
    },
    /// 信号：第几号显示器、0–65535 的比例位置、信号 id。
    #[serde(rename = "G")]
    Ping {
        #[serde(rename = "d")]
        display: u8,
        #[serde(rename = "x")]
        x: u16,
        #[serde(rename = "y")]
        y: u16,
        #[serde(rename = "i")]
        id: String,
    },
    /// 表情：第几号显示器、比例位置、表情引用（内置的 slug 或 `c:` 加哈希）、导入表情的出场效果。
    #[serde(rename = "E")]
    Emote {
        #[serde(rename = "d")]
        display: u8,
        #[serde(rename = "x")]
        x: u16,
        #[serde(rename = "y")]
        y: u16,
        #[serde(rename = "r")]
        emote: String,
        #[serde(rename = "f", default)]
        fx: u8,
    },
    /// 向 `to` 要一张导入表情的静态图；`need` 是缺的块号，空表示全要。
    #[serde(rename = "W")]
    Want {
        #[serde(rename = "t", with = "serde_bytes")]
        to: MemberId,
        #[serde(rename = "h", with = "serde_bytes")]
        hash: [u8; 16],
        #[serde(rename = "n", with = "serde_bytes")]
        need: Vec<u8>,
    },
    /// 静态图的一块：第 i 块、共 n 块。
    #[serde(rename = "H")]
    Chunk {
        #[serde(rename = "h", with = "serde_bytes")]
        hash: [u8; 16],
        #[serde(rename = "i")]
        i: u8,
        #[serde(rename = "n")]
        n: u8,
        #[serde(rename = "b", with = "serde_bytes")]
        bytes: Vec<u8>,
    },
    /// 一句话（Alt+T）：第几号显示器、比例位置、最多 120 个字。
    #[serde(rename = "T")]
    Text {
        #[serde(rename = "d")]
        display: u8,
        #[serde(rename = "x")]
        x: u16,
        #[serde(rename = "y")]
        y: u16,
        #[serde(rename = "t")]
        text: String,
    },
    /// 发起投票：编号、种类（`VoteKind::code`）、踢人时的目标。
    #[serde(rename = "V")]
    Vote {
        #[serde(rename = "i")]
        id: u32,
        #[serde(rename = "k")]
        kind: u8,
        #[serde(rename = "t", default, with = "serde_bytes")]
        target: Option<MemberId>,
    },
    /// 对投票的回答；投票结束前每 3 秒重发一次（实时帧可能丢）。
    #[serde(rename = "B")]
    Ballot {
        #[serde(rename = "i")]
        id: u32,
        #[serde(rename = "y")]
        yes: bool,
    },
    /// 离开队伍或退出程序。
    #[serde(rename = "Y")]
    Bye,
}

/// 在线状态里「关掉了的功能」的位。
pub const OFF_TEXT: u8 = 1;
pub const OFF_VOTES: u8 = 2;

fn digest(room: &RoomId, from: &MemberId, seq: u64, ts_ms: u64, body: &[u8]) -> [u8; 32] {
    crypto::sha256(&[
        b"gang/live/v2",
        room,
        from,
        &seq.to_be_bytes(),
        &ts_ms.to_be_bytes(),
        &(body.len() as u32).to_be_bytes(),
        body,
    ])
}

pub fn sign_live(me: &Identity, room: &RoomId, seq: u64, ts_ms: u64, live: &LiveBody) -> WireLive {
    let from = me.member_id();
    let b = body::encode(live);
    let sig = me.sign_event_id(&digest(room, &from, seq, ts_ms, &b));
    WireLive(from, seq, ts_ms, b, sig)
}

/// 一帧验过签、解出来的内容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenedLive {
    pub from: MemberId,
    pub seq: u64,
    pub ts_ms: u64,
    pub body: LiveBody,
}

/// 验签并解码。签名不对、或内容是不认识的帧类型（将来的版本加的）就返回 None。
pub fn open_live(w: &WireLive, room: &RoomId) -> Option<OpenedLive> {
    let WireLive(from, seq, ts_ms, b, sig) = w;
    if !crypto::verify_event_sig(from, &digest(room, from, *seq, *ts_ms, b), sig) {
        return None;
    }
    let body = body::decode(b).ok()?;
    Some(OpenedLive {
        from: *from,
        seq: *seq,
        ts_ms: *ts_ms,
        body,
    })
}

/// 挡重放：见 1、2 条。
#[derive(Default)]
pub struct ReplayGuard {
    seen: HashMap<MemberId, BTreeSet<u64>>,
}

impl ReplayGuard {
    /// 这一帧要不要。要的话记下它的序号。
    pub fn accept(&mut self, from: &MemberId, seq: u64, ts_ms: u64, now_ms: u64) -> bool {
        if ts_ms.abs_diff(now_ms) > MAX_SKEW_MS {
            return false;
        }
        let seen = self.seen.entry(*from).or_default();
        if seen.contains(&seq)
            || (seen.len() >= WINDOW && seen.first().is_some_and(|&oldest| seq < oldest))
        {
            return false;
        }
        seen.insert(seq);
        while seen.len() > WINDOW {
            seen.pop_first();
        }
        true
    }

    /// 队员离开后忘掉他。
    pub fn forget(&mut self, from: &MemberId) {
        self.seen.remove(from);
    }
}

/// 发送方的序号：微秒时间戳，且严格递增。
#[derive(Default)]
pub struct LiveSeq {
    last: u64,
}

impl LiveSeq {
    pub fn next(&mut self, now_us: u64) -> u64 {
        self.last = now_us.max(self.last + 1);
        self.last
    }
}

/// 窗口里的 CSS 像素位置 ↔ 0–65535 的比例。
pub fn to_ratio(v: f64, size: f64) -> u16 {
    if size <= 0.0 || !v.is_finite() {
        return 0;
    }
    ((v / size).clamp(0.0, 1.0) * 65535.0).round() as u16
}

pub fn from_ratio(r: u16, size: f64) -> f64 {
    f64::from(r) / 65535.0 * size
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOM: RoomId = [3u8; 16];
    const NOW: u64 = 1_700_000_000_000;

    fn me() -> Identity {
        Identity::from_secret(&[7u8; 32])
    }

    fn ping() -> LiveBody {
        LiveBody::Ping {
            display: 2,
            x: 10,
            y: 65535,
            id: "omw".into(),
        }
    }

    #[test]
    fn signed_frames_open_with_the_same_content() {
        let w = sign_live(&me(), &ROOM, 5, NOW, &ping());
        let opened = open_live(&w, &ROOM).unwrap();
        assert_eq!(
            opened,
            OpenedLive {
                from: me().member_id(),
                seq: 5,
                ts_ms: NOW,
                body: ping()
            }
        );
        let presence = LiveBody::Presence {
            status: Status::Away,
            laning: false,
            version: "2.0.0".into(),
            platform: "win".into(),
            emotes: vec![9; 32],
            off: OFF_VOTES,
        };
        let w = sign_live(&me(), &ROOM, 6, NOW, &presence);
        assert_eq!(open_live(&w, &ROOM).unwrap().body, presence);
        assert_eq!(
            open_live(&sign_live(&me(), &ROOM, 7, NOW, &LiveBody::Bye), &ROOM)
                .unwrap()
                .body,
            LiveBody::Bye
        );
    }

    #[test]
    fn tampered_or_moved_frames_do_not_open() {
        let w = sign_live(&me(), &ROOM, 5, NOW, &ping());
        assert!(open_live(&w, &[4u8; 16]).is_none()); // 别的队伍
        let mut bad = w.clone();
        bad.1 += 1; // 改序号
        assert!(open_live(&bad, &ROOM).is_none());
        let mut bad = w.clone();
        bad.2 += 1; // 改时间
        assert!(open_live(&bad, &ROOM).is_none());
        let mut bad = w.clone();
        bad.0 = Identity::from_secret(&[8u8; 32]).member_id(); // 冒充别人
        assert!(open_live(&bad, &ROOM).is_none());
        let mut bad = w;
        bad.3 = body::encode(&LiveBody::Bye); // 换内容
        assert!(open_live(&bad, &ROOM).is_none());
    }

    #[test]
    fn unknown_frame_types_are_ignored() {
        let me = me();
        let from = me.member_id();
        let b = body::encode(&std::collections::BTreeMap::from([("Z", 1u8)]));
        let sig = me.sign_event_id(&digest(&ROOM, &from, 1, NOW, &b));
        assert!(open_live(&WireLive(from, 1, NOW, b, sig), &ROOM).is_none());
    }

    #[test]
    fn guard_drops_duplicates_old_frames_and_clock_skew() {
        let a = me().member_id();
        let b = Identity::from_secret(&[8u8; 32]).member_id();
        let mut g = ReplayGuard::default();
        assert!(g.accept(&a, 100, NOW, NOW));
        assert!(!g.accept(&a, 100, NOW, NOW)); // 同一帧从中继又来一份
        assert!(g.accept(&a, 99, NOW, NOW)); // 晚到的旧帧，窗口内没见过
        assert!(g.accept(&b, 100, NOW, NOW)); // 各算各的
        assert!(!g.accept(&a, 101, NOW + MAX_SKEW_MS + 1, NOW));
        assert!(!g.accept(&a, 102, NOW - MAX_SKEW_MS - 1, NOW));
        assert!(g.accept(&a, 103, NOW + MAX_SKEW_MS, NOW));
        // 记满 64 个以后，比窗口里最旧的还旧的不要
        let mut g = ReplayGuard::default();
        for s in 1000..1064 {
            assert!(g.accept(&a, s, NOW, NOW));
        }
        assert!(!g.accept(&a, 999, NOW, NOW));
        assert!(g.accept(&a, 1064, NOW, NOW));
        assert!(!g.accept(&a, 1000, NOW, NOW)); // 1000 已经被挤出窗口，也比最旧的旧
        g.forget(&a);
        assert!(g.accept(&a, 5, NOW, NOW));
    }

    #[test]
    fn sequence_grows_even_when_the_clock_does_not() {
        let mut s = LiveSeq::default();
        assert_eq!(s.next(1000), 1000);
        assert_eq!(s.next(1000), 1001);
        assert_eq!(s.next(500), 1002);
        assert_eq!(s.next(5000), 5000);
    }

    #[test]
    fn ratios_cover_the_whole_display() {
        assert_eq!(to_ratio(0.0, 1920.0), 0);
        assert_eq!(to_ratio(1920.0, 1920.0), 65535);
        assert_eq!(to_ratio(-5.0, 1920.0), 0);
        assert_eq!(to_ratio(960.0, 1920.0), 32768);
        assert_eq!(to_ratio(1.0, 0.0), 0);
        assert!((from_ratio(32768, 1080.0) - 540.0).abs() < 0.01);
        assert_eq!(from_ratio(65535, 1080.0), 1080.0);
    }

    fn wire_len(body: &LiveBody) -> usize {
        let w = sign_live(&me(), &ROOM, u64::MAX, u64::MAX, body);
        crate::protocol::frame::encode_frame(&crate::protocol::frame::Frame::Live(w)).len()
    }

    #[test]
    fn frames_are_small() {
        assert!(wire_len(&ping()) < 160, "信号帧 {} 字节", wire_len(&ping()));
        let emote = LiveBody::Emote {
            display: 1,
            x: 1,
            y: 2,
            emote: format!("c:{}", "a".repeat(32)),
            fx: 5,
        };
        assert!(wire_len(&emote) < 200);
        let text = LiveBody::Text {
            display: 1,
            x: 1,
            y: 2,
            text: "钢".repeat(crate::team_text::MAX_CHARS),
        };
        assert!(
            wire_len(&text) <= crate::protocol::frame::MAX_FRAME_LEN,
            "最长的文字帧 {} 字节",
            wire_len(&text)
        );
    }

    /// 最大的分块、报满 9 个哈希的在线状态都要装得进一个包。
    #[test]
    fn biggest_frames_fit_one_packet() {
        use crate::emotes::transfer::{CHUNK_BYTES, CHUNKS_MAX};
        use crate::protocol::frame::MAX_FRAME_LEN;
        let chunk = LiveBody::Chunk {
            hash: [0xff; 16],
            i: 255,
            n: 255,
            bytes: vec![0xff; CHUNK_BYTES],
        };
        assert!(
            wire_len(&chunk) <= MAX_FRAME_LEN,
            "分块帧 {} 字节",
            wire_len(&chunk)
        );
        let want = LiveBody::Want {
            to: [0xff; 32],
            hash: [0xff; 16],
            need: (0..CHUNKS_MAX).collect(),
        };
        assert!(wire_len(&want) <= MAX_FRAME_LEN);
        let presence = LiveBody::Presence {
            status: Status::Online,
            laning: false,
            version: "20.10.100-beta.1".into(),
            platform: "linux".into(),
            emotes: vec![0xff; 16 * crate::emotes::ANNOUNCED_MAX],
            off: 0xff,
        };
        assert!(wire_len(&presence) <= MAX_FRAME_LEN);
    }

    /// 2.0 的对线帧（C、K）验签后当作不认识的帧丢掉；3.0 的在线状态仍带「对线」一项，2.0 解得开。
    #[test]
    fn laning_frames_from_2_0_are_dropped_and_presence_stays_readable() {
        #[derive(Serialize)]
        enum Old {
            #[serde(rename = "C")]
            Cursor {
                #[serde(rename = "d")]
                display: u8,
                #[serde(rename = "x")]
                x: u16,
                #[serde(rename = "y")]
                y: u16,
            },
            #[serde(rename = "K")]
            Charge {
                #[serde(rename = "t", with = "serde_bytes")]
                target: MemberId,
                #[serde(rename = "o")]
                on: bool,
            },
        }
        let me = me();
        let from = me.member_id();
        let old = [
            Old::Cursor {
                display: 1,
                x: 2,
                y: 3,
            },
            Old::Charge {
                target: [5; 32],
                on: true,
            },
        ];
        for (seq, frame) in (1..).zip(old) {
            let b = body::encode(&frame);
            let sig = me.sign_event_id(&digest(&ROOM, &from, seq, NOW, &b));
            assert_eq!(open_live(&WireLive(from, seq, NOW, b, sig), &ROOM), None);
        }

        // 2.0 的在线状态：「对线」那一项没有默认值，缺了整帧解不开
        #[derive(Deserialize)]
        enum OldPresence {
            #[serde(rename = "P")]
            Presence {
                #[serde(rename = "l")]
                laning: bool,
            },
        }
        let mine = LiveBody::Presence {
            status: Status::Online,
            laning: false,
            version: "3.0.0".into(),
            platform: "win".into(),
            emotes: Vec::new(),
            off: 0,
        };
        let OldPresence::Presence { laning } = body::decode(&body::encode(&mine)).unwrap();
        assert!(!laning);
    }

    #[test]
    fn votes_and_ballots_round_trip() {
        for body in [
            LiveBody::Vote {
                id: 0xdead_beef,
                kind: 2,
                target: Some([5u8; 32]),
            },
            LiveBody::Vote {
                id: 1,
                kind: 0,
                target: None,
            },
            LiveBody::Ballot { id: 7, yes: false },
            LiveBody::Text {
                display: 2,
                x: 3,
                y: 4,
                text: "上路来人".into(),
            },
        ] {
            let w = sign_live(&me(), &ROOM, 9, NOW, &body);
            assert_eq!(open_live(&w, &ROOM).unwrap().body, body);
        }
    }

    /// 旧的在线状态（没有 e）照样能读，导入表情当作没有。
    #[test]
    fn presence_without_emotes_still_opens() {
        #[derive(Serialize)]
        enum Old {
            #[serde(rename = "P")]
            Presence {
                s: Status,
                l: bool,
                v: String,
                o: String,
            },
        }
        let old = Old::Presence {
            s: Status::Online,
            l: false,
            v: "2.0.0".into(),
            o: "win".into(),
        };
        let LiveBody::Presence { emotes, .. } = body::decode(&body::encode(&old)).unwrap() else {
            panic!("不是在线状态");
        };
        assert!(emotes.is_empty());
        let LiveBody::Presence { off, .. } = body::decode(&body::encode(&old)).unwrap() else {
            panic!("不是在线状态");
        };
        assert_eq!(off, 0);
    }
}
