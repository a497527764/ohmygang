//! 把一段话的 Opus 小帧打成语音实时帧（`LiveBody::Voice`）。
//!
//! 实现流程：
//! 1. 每个小帧 20 毫秒。攒够 20 个（400 毫秒）就打成一帧发出去：16 kbps 恒定码率时每个小帧 40 字节，
//!    一帧约 1 KB，装得进一个 1200 字节的包；中继每秒约 2.5 包，发送预算是每秒 5 包，给信号留出余量。
//! 2. 万一小帧变大（比如以后换了码率），攒的字节数超过上限就提前发，绝不超过包的大小。
//! 3. 每一帧带上「这段话的号」和「第一个小帧的序号」，收的人据此排好顺序、发现丢了哪些。
//!    说完时把剩下的小帧打成最后一帧（标上「结束」），一个小帧都没有也发一个空的结束帧。

use serde_bytes::ByteBuf;

use crate::protocol::live::LiveBody;

/// 一帧最多装多少个 20 毫秒的小帧（400 毫秒）。
pub const FRAMES_PER_PACKET: usize = 20;
/// 一帧里小帧加起来最多这么多字节；加上签名、公钥这些仍在包的上限（1141 字节）以内。
const MAX_AUDIO_BYTES: usize = 900;

pub struct Packer {
    stream: u32,
    /// 下一个要打包的小帧在这段话里的序号。
    next: u32,
    pending: Vec<ByteBuf>,
    bytes: usize,
}

impl Packer {
    pub fn new(stream: u32) -> Self {
        Self {
            stream,
            next: 0,
            pending: Vec::new(),
            bytes: 0,
        }
    }

    /// 已经发出去（交给 `push` 的返回值）的小帧个数。
    pub fn sent(&self) -> u32 {
        self.next
    }

    /// 加一个小帧；攒够了就返回要发的一帧。
    pub fn push(&mut self, frame: Vec<u8>) -> Option<LiveBody> {
        let mut out = None;
        if !self.pending.is_empty() && self.bytes + frame.len() > MAX_AUDIO_BYTES {
            out = Some(self.take(false));
        }
        self.bytes += frame.len();
        self.pending.push(ByteBuf::from(frame));
        if self.pending.len() >= FRAMES_PER_PACKET {
            debug_assert!(out.is_none(), "提前发过的话，新攒的只有一个小帧");
            out = Some(self.take(false));
        }
        out
    }

    /// 说完了：剩下的小帧打成最后一帧。
    pub fn finish(&mut self) -> LiveBody {
        self.take(true)
    }

    fn take(&mut self, end: bool) -> LiveBody {
        let frames = std::mem::take(&mut self.pending);
        let first = self.next;
        self.next += frames.len() as u32;
        self.bytes = 0;
        LiveBody::Voice {
            stream: self.stream,
            first,
            frames,
            end,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(b: &LiveBody) -> (u32, u32, usize, bool) {
        match b {
            LiveBody::Voice {
                stream,
                first,
                frames,
                end,
            } => (*stream, *first, frames.len(), *end),
            _ => panic!("不是语音帧"),
        }
    }

    #[test]
    fn twenty_small_frames_make_one_packet_and_the_rest_goes_with_the_end() {
        let mut p = Packer::new(7);
        let mut sent = Vec::new();
        for _ in 0..45 {
            sent.extend(p.push(vec![1; 40]));
        }
        assert_eq!(
            sent.iter().map(parts).collect::<Vec<_>>(),
            vec![(7, 0, 20, false), (7, 20, 20, false)]
        );
        assert_eq!(p.sent(), 40);
        assert_eq!(parts(&p.finish()), (7, 40, 5, true));
        assert_eq!(
            parts(&p.finish()),
            (7, 45, 0, true),
            "什么都没剩也能发结束帧"
        );
    }

    #[test]
    fn big_frames_are_sent_early_so_a_packet_never_overflows() {
        let mut p = Packer::new(1);
        let mut sent = Vec::new();
        for _ in 0..10 {
            sent.extend(p.push(vec![2; 200]));
        }
        sent.push(p.finish());
        let got: Vec<_> = sent.iter().map(parts).collect();
        // 每帧最多 4 个 200 字节的小帧（900 字节以内），序号接得上，一个不丢
        assert_eq!(
            got,
            vec![(1, 0, 4, false), (1, 4, 4, false), (1, 8, 2, true)]
        );
    }
}
