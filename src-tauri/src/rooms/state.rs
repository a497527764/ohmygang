//! 由状态事件推导队伍状态。
//!
//! 实现流程：
//! 1. 同一作者同一 seq 有多条（分叉）时，只取 event_id 最小的一条。
//! 2. 队长：公钥摘要等于 owner_tag 的作者，且有能解码的 RoomCreate；取其 seq 最小的 RoomCreate。
//!    其他人发的 RoomCreate、Rename、Dissolve 一律不生效（队长信任根）。
//! 3. 成员：每位作者 seq 最大的 Join / Leave 决定是否在队；昵称和头像取 seq 最大的 Profile
//!    （这条 Profile 没带头像或头像不合格，就用自动生成的头像）。
//! 4. 队名：队长 seq 最大的 RoomCreate / Rename。
//! 5. 解散：队长 seq 最小的 Dissolve，其 ts_ms 就是 dissolve_ts。
//!
//! 整个推导是事件集合的纯函数，到达顺序不影响结果。

use std::collections::{BTreeMap, BTreeSet};

use crate::crypto::MemberId;
use crate::protocol::body::{self, Avatar, ProfileBody, RenameBody, RoomCreateBody};
use crate::protocol::event::{SignedEvent, kind};
use crate::protocol::room_code::owner_tag_of;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoomState {
    pub owner: Option<MemberId>,
    pub title: Option<String>,
    pub members: BTreeSet<MemberId>,
    pub nicknames: BTreeMap<MemberId, String>,
    /// 只含最新 Profile 带了合格头像的人。
    pub avatars: BTreeMap<MemberId, Avatar>,
    /// 最新 Profile 报了大头像哈希（形状合法）的人；图片走分块传输，这里只存哈希。
    pub avatar_hashes: BTreeMap<MemberId, String>,
    pub dissolve_ts: Option<u64>,
}

/// 同一作者同一 seq 只留 event_id 最小的一条，按 (作者, seq) 升序。
pub fn dedup_forks(events: &[SignedEvent]) -> BTreeMap<(MemberId, u64), &SignedEvent> {
    let mut by_slot: BTreeMap<(MemberId, u64), &SignedEvent> = BTreeMap::new();
    for e in events {
        let slot = (e.event.author_id, e.event.seq);
        match by_slot.get(&slot) {
            Some(cur) if cur.event_id <= e.event_id => {}
            _ => {
                by_slot.insert(slot, e);
            }
        }
    }
    by_slot
}

pub fn derive_room_state(owner_tag: &[u8; 10], events: &[SignedEvent]) -> RoomState {
    let by_slot = dedup_forks(events);
    let mut st = RoomState::default();
    let mut last_membership: BTreeMap<MemberId, bool> = BTreeMap::new();

    // BTreeMap 按 (作者, seq) 升序遍历，同一作者后面的事件覆盖前面的
    for ((author, _seq), e) in &by_slot {
        let ev = &e.event;
        let is_owner_key = owner_tag_of(author) == *owner_tag;
        match ev.kind {
            kind::JOIN => {
                last_membership.insert(*author, true);
            }
            kind::LEAVE => {
                last_membership.insert(*author, false);
            }
            kind::PROFILE => {
                if let Ok(p) = body::decode::<ProfileBody>(&ev.body) {
                    st.nicknames.insert(*author, p.nickname);
                    match p.avatar.filter(Avatar::is_valid) {
                        Some(a) => {
                            st.avatars.insert(*author, a);
                        }
                        None => {
                            st.avatars.remove(author);
                        }
                    };
                    match p.avatar_hash.filter(|h| crate::emotes::is_hash(h)) {
                        Some(h) => {
                            st.avatar_hashes.insert(*author, h);
                        }
                        None => {
                            st.avatar_hashes.remove(author);
                        }
                    };
                }
            }
            kind::ROOM_CREATE if is_owner_key => {
                if let Ok(c) = body::decode::<RoomCreateBody>(&ev.body) {
                    if st.owner.is_none() {
                        st.owner = Some(*author);
                    }
                    st.title = Some(c.title);
                }
            }
            kind::RENAME if st.owner == Some(*author) => {
                if let Ok(r) = body::decode::<RenameBody>(&ev.body) {
                    st.title = Some(r.title);
                }
            }
            kind::DISSOLVE if st.owner == Some(*author) && st.dissolve_ts.is_none() => {
                st.dissolve_ts = Some(ev.ts_ms);
            }
            _ => {} // 非队长的管理事件、命中、未知 kind：不影响队伍状态
        }
    }

    st.members = last_membership
        .into_iter()
        .filter(|(_, inside)| *inside)
        .map(|(m, _)| m)
        .collect();
    st
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Identity;
    use crate::protocol::body::encode;
    use crate::protocol::event::build_event;

    const ROOM: [u8; 16] = [1u8; 16];

    fn person(n: u8) -> Identity {
        Identity::from_secret(&[n; 32])
    }

    fn state(me: &Identity, seq: u64, ts: u64, k: u16, body: Vec<u8>) -> SignedEvent {
        build_event(me, ROOM, seq, ts, k, body)
    }

    fn create(me: &Identity, seq: u64, title: &str) -> SignedEvent {
        state(
            me,
            seq,
            seq * 1000,
            kind::ROOM_CREATE,
            encode(&RoomCreateBody {
                title: title.into(),
            }),
        )
    }

    fn rename(me: &Identity, seq: u64, ts: u64, title: &str) -> SignedEvent {
        state(
            me,
            seq,
            ts,
            kind::RENAME,
            encode(&RenameBody {
                title: title.into(),
            }),
        )
    }

    #[test]
    fn owner_members_and_title() {
        let owner = person(1);
        let bob = person(2);
        let tag = owner_tag_of(&owner.member_id());
        let events = vec![
            create(&owner, 1, "开黑"),
            state(&owner, 2, 2000, kind::JOIN, vec![]),
            state(
                &owner,
                3,
                3000,
                kind::PROFILE,
                encode(&ProfileBody {
                    nickname: "队长".into(),
                    avatar: None,
                    avatar_hash: None,
                }),
            ),
            state(&bob, 1, 1500, kind::JOIN, vec![]),
            rename(&owner, 4, 4000, "开黑 2"),
        ];
        let st = derive_room_state(&tag, &events);
        assert_eq!(st.owner, Some(owner.member_id()));
        assert_eq!(st.title.as_deref(), Some("开黑 2"));
        assert_eq!(
            st.members,
            [owner.member_id(), bob.member_id()].into_iter().collect()
        );
        assert_eq!(
            st.nicknames.get(&owner.member_id()).map(String::as_str),
            Some("队长")
        );
        assert_eq!(st.dissolve_ts, None);
    }

    #[test]
    fn non_owner_cannot_act_as_owner() {
        let owner = person(1);
        let mallory = person(3);
        let tag = owner_tag_of(&owner.member_id());
        let events = vec![
            create(&owner, 1, "开黑"),
            create(&mallory, 1, "被劫持"),
            rename(&mallory, 2, 2000, "被改名"),
            state(&mallory, 4, 4000, kind::DISSOLVE, vec![]),
        ];
        let st = derive_room_state(&tag, &events);
        assert_eq!(st.owner, Some(owner.member_id()));
        assert_eq!(st.title.as_deref(), Some("开黑"));
        assert_eq!(st.dissolve_ts, None);
    }

    #[test]
    fn leave_dissolve_and_order_independence() {
        let owner = person(1);
        let bob = person(2);
        let tag = owner_tag_of(&owner.member_id());
        let mut events = vec![
            create(&owner, 1, "开黑"),
            state(&owner, 2, 2000, kind::JOIN, vec![]),
            state(&bob, 1, 1500, kind::JOIN, vec![]),
            state(&bob, 2, 2500, kind::LEAVE, vec![]),
            state(&owner, 3, 50_000, kind::DISSOLVE, vec![]),
            state(&owner, 4, 60_000, kind::DISSOLVE, vec![]),
        ];
        let st = derive_room_state(&tag, &events);
        assert_eq!(st.members, [owner.member_id()].into_iter().collect());
        assert_eq!(st.dissolve_ts, Some(50_000));
        events.reverse();
        assert_eq!(derive_room_state(&tag, &events), st);
    }

    #[test]
    fn fork_takes_smallest_event_id() {
        let owner = person(1);
        let tag = owner_tag_of(&owner.member_id());
        let a = rename(&owner, 2, 2000, "甲");
        let b = rename(&owner, 2, 2001, "乙");
        let expected = if a.event_id < b.event_id {
            "甲"
        } else {
            "乙"
        };
        let st = derive_room_state(&tag, &[create(&owner, 1, "开黑"), a, b]);
        assert_eq!(st.title.as_deref(), Some(expected));
    }

    #[test]
    fn latest_profile_decides_avatar() {
        let owner = person(1);
        let tag = owner_tag_of(&owner.member_id());
        let profile = |seq: u64, avatar: Option<Avatar>| {
            state(
                &owner,
                seq,
                seq * 1000,
                kind::PROFILE,
                encode(&ProfileBody {
                    nickname: format!("第{seq}版"),
                    avatar,
                    avatar_hash: None,
                }),
            )
        };
        let preset = Avatar::Preset { icon: 1, color: 2 };
        let me = owner.member_id();
        let st = derive_room_state(&tag, &[profile(1, Some(preset.clone()))]);
        assert_eq!(st.avatars.get(&me), Some(&preset));
        // 后一条没带头像：恢复自动生成的头像，昵称照常更新
        let st = derive_room_state(&tag, &[profile(1, Some(preset.clone())), profile(2, None)]);
        assert_eq!(st.avatars.get(&me), None);
        assert_eq!(st.nicknames.get(&me).map(String::as_str), Some("第2版"));
        // 不合格的头像当作没有
        let bad = Avatar::Image(b"<svg/>".to_vec());
        let st = derive_room_state(&tag, &[profile(1, Some(preset)), profile(2, Some(bad))]);
        assert_eq!(st.avatars.get(&me), None);
    }
}
