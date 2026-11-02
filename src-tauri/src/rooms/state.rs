//! 由状态事件推导房间状态（架构 8.1）。
//!
//! 实现流程：
//! 1. 只看 state stream 的事件；同一作者同一 seq 有多条（分叉）时，只取 event_id 最小的一条。
//! 2. 房主：公钥摘要等于 owner_tag 的作者，且有能解码的 RoomCreate；取其 seq 最小的 RoomCreate。
//!    其他人发的 RoomCreate、Rename、ConfigNotice、Dissolve 一律不生效（房主信任根）。
//! 3. 成员：每位作者 seq 最大的 Join / Leave 决定是否在房；昵称取 seq 最大的 Profile。
//! 4. 群名：房主 seq 最大的 RoomCreate / Rename。
//! 5. 规则：房主第一条 RoomCreate 的规则（epoch 必须为 0），之后 epoch 严格递增的 ConfigNotice；
//!    超出范围的规则无效。当前规则取最大 epoch。
//! 6. 解散：房主 seq 最小的 Dissolve，其 ts_ms 就是 dissolve_ts。
//!
//! 整个推导是事件集合的纯函数，到达顺序不影响结果。

use std::collections::{BTreeMap, BTreeSet};

use crate::crypto::MemberId;
use crate::heartsteel::Rules;
use crate::protocol::body::{
    self, ConfigNoticeBody, ProfileBody, RenameBody, RoomCreateBody, Rule,
};
use crate::protocol::event::{SignedEvent, Stream, kind};
use crate::protocol::room_code::owner_tag_of;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoomState {
    pub owner: Option<MemberId>,
    pub title: Option<String>,
    pub members: BTreeSet<MemberId>,
    pub nicknames: BTreeMap<MemberId, String>,
    pub rules: Rules,
    pub current_rule: Option<Rule>,
    pub dissolve_ts: Option<u64>,
}

pub fn derive_room_state(owner_tag: &[u8; 10], events: &[SignedEvent]) -> RoomState {
    // (作者, seq) → 该序号下 event_id 最小的事件
    let mut by_slot: BTreeMap<(MemberId, u64), &SignedEvent> = BTreeMap::new();
    for e in events.iter().filter(|e| e.event.stream == Stream::State) {
        let slot = (e.event.author_id, e.event.seq);
        match by_slot.get(&slot) {
            Some(cur) if cur.event_id <= e.event_id => {}
            _ => {
                by_slot.insert(slot, e);
            }
        }
    }

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
                }
            }
            kind::ROOM_CREATE if is_owner_key => {
                if let Ok(c) = body::decode::<RoomCreateBody>(&ev.body) {
                    if st.owner.is_none() {
                        st.owner = Some(*author);
                        if c.rule.rule_epoch == 0 && c.rule.is_valid() {
                            st.rules.insert(0, c.rule);
                        }
                    }
                    st.title = Some(c.title);
                }
            }
            kind::RENAME if st.owner == Some(*author) => {
                if let Ok(r) = body::decode::<RenameBody>(&ev.body) {
                    st.title = Some(r.title);
                }
            }
            kind::CONFIG_NOTICE if st.owner == Some(*author) => {
                if let Ok(c) = body::decode::<ConfigNoticeBody>(&ev.body) {
                    let max_epoch = st.rules.keys().max().copied();
                    if c.rule.is_valid() && max_epoch.is_some_and(|m| c.rule.rule_epoch > m) {
                        st.rules.insert(c.rule.rule_epoch, c.rule);
                    }
                }
            }
            kind::DISSOLVE if st.owner == Some(*author) && st.dissolve_ts.is_none() => {
                st.dissolve_ts = Some(ev.ts_ms);
            }
            _ => {} // 非房主的管理事件、心之钢、未知 kind：不影响房间状态
        }
    }

    st.members = last_membership
        .into_iter()
        .filter(|(_, inside)| *inside)
        .map(|(m, _)| m)
        .collect();
    st.current_rule = st.rules.keys().max().and_then(|e| st.rules.get(e)).copied();
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
        build_event(me, ROOM, Stream::State, seq, ts, k, body)
    }

    fn create(me: &Identity, seq: u64, title: &str) -> SignedEvent {
        state(
            me,
            seq,
            seq * 1000,
            kind::ROOM_CREATE,
            encode(&RoomCreateBody {
                title: title.into(),
                rule: Rule::DEFAULT,
            }),
        )
    }

    fn config(me: &Identity, seq: u64, rule: Rule) -> SignedEvent {
        state(
            me,
            seq,
            seq * 1000,
            kind::CONFIG_NOTICE,
            encode(&ConfigNoticeBody { rule }),
        )
    }

    #[test]
    fn owner_members_title_rules() {
        let owner = person(1);
        let bob = person(2);
        let tag = owner_tag_of(&owner.member_id());
        let r1 = Rule {
            rule_epoch: 1,
            threshold: 4,
            ..Rule::DEFAULT
        };
        let events = vec![
            create(&owner, 1, "开黑"),
            state(&owner, 2, 2000, kind::JOIN, vec![]),
            state(
                &owner,
                3,
                3000,
                kind::PROFILE,
                encode(&ProfileBody {
                    nickname: "房主".into(),
                }),
            ),
            state(&bob, 1, 1500, kind::JOIN, vec![]),
            state(
                &owner,
                4,
                4000,
                kind::RENAME,
                encode(&RenameBody {
                    title: "开黑 2".into(),
                }),
            ),
            config(&owner, 5, r1),
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
            Some("房主")
        );
        assert_eq!(st.rules.len(), 2);
        assert_eq!(st.current_rule, Some(r1));
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
            state(
                &mallory,
                2,
                2000,
                kind::RENAME,
                encode(&RenameBody {
                    title: "被改名".into(),
                }),
            ),
            config(
                &mallory,
                3,
                Rule {
                    rule_epoch: 9,
                    threshold: 2,
                    ..Rule::DEFAULT
                },
            ),
            state(&mallory, 4, 4000, kind::DISSOLVE, vec![]),
        ];
        let st = derive_room_state(&tag, &events);
        assert_eq!(st.owner, Some(owner.member_id()));
        assert_eq!(st.title.as_deref(), Some("开黑"));
        assert_eq!(st.current_rule, Some(Rule::DEFAULT));
        assert_eq!(st.dissolve_ts, None);
    }

    #[test]
    fn rule_epochs_must_increase_and_be_in_range() {
        let owner = person(1);
        let tag = owner_tag_of(&owner.member_id());
        let r2 = Rule {
            rule_epoch: 2,
            ..Rule::DEFAULT
        };
        let events = vec![
            create(&owner, 1, "开黑"),
            config(&owner, 2, r2),
            config(
                &owner,
                3,
                Rule {
                    rule_epoch: 1,
                    ..Rule::DEFAULT
                },
            ), // epoch 回退，无效
            config(
                &owner,
                4,
                Rule {
                    rule_epoch: 5,
                    threshold: 11,
                    ..Rule::DEFAULT
                },
            ), // 超范围，无效
        ];
        let st = derive_room_state(&tag, &events);
        assert_eq!(
            st.rules.keys().copied().collect::<BTreeSet<u32>>(),
            [0, 2].into_iter().collect()
        );
        assert_eq!(st.current_rule, Some(r2));
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
        let a = state(
            &owner,
            2,
            2000,
            kind::RENAME,
            encode(&RenameBody {
                title: "甲".into()
            }),
        );
        let b = state(
            &owner,
            2,
            2001,
            kind::RENAME,
            encode(&RenameBody {
                title: "乙".into()
            }),
        );
        let expected = if a.event_id < b.event_id {
            "甲"
        } else {
            "乙"
        };
        let st = derive_room_state(&tag, &[create(&owner, 1, "开黑"), a, b]);
        assert_eq!(st.title.as_deref(), Some(expected));
    }

    #[test]
    fn chat_events_are_ignored() {
        let owner = person(1);
        let tag = owner_tag_of(&owner.member_id());
        let chat = build_event(&owner, ROOM, Stream::Chat, 1, 1000, kind::JOIN, vec![]);
        assert!(derive_room_state(&tag, &[chat]).members.is_empty());
    }
}
