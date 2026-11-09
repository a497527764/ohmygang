//! 采样率转换：麦克风常见是 48 kHz（也有 44.1 kHz），语音编码用 16 kHz；放的时候再转回扬声器的采样率。
//!
//! 实现流程：
//! 1. 线性插值，按块处理，块与块之间接得上（记住上一块的最后一个采样和小数位置）。
//! 2. 降采样前先过一个一阶低通（约 7 kHz），减少高频折叠成的杂音；16 kHz 的语音本来只到 8 kHz。
//!    对说话够用，不追求音乐的音质，也不引入新依赖。

use std::f64::consts::PI;

pub struct Resampler {
    /// 每个输出采样前进多少个输入采样。
    step: f64,
    /// 下一个输出采样在「[上一块最后一个采样, 这一块…]」里的位置。
    pos: f64,
    prev: f32,
    /// 低通的系数（0 表示不过滤）和状态。
    alpha: f32,
    lp: f32,
}

impl Resampler {
    pub fn new(from_hz: u32, to_hz: u32) -> Self {
        let alpha = if from_hz > to_hz {
            (1.0 - (-2.0 * PI * 7000.0 / f64::from(from_hz)).exp()) as f32
        } else {
            0.0
        };
        Self {
            step: f64::from(from_hz) / f64::from(to_hz),
            pos: 1.0,
            prev: 0.0,
            alpha,
            lp: 0.0,
        }
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if input.is_empty() {
            return;
        }
        let mut buf = Vec::with_capacity(input.len() + 1);
        buf.push(self.prev);
        if self.alpha > 0.0 {
            for &x in input {
                self.lp += self.alpha * (x - self.lp);
                buf.push(self.lp);
            }
        } else {
            buf.extend_from_slice(input);
        }
        let last = (buf.len() - 1) as f64;
        while self.pos <= last {
            let i = self.pos.floor() as usize;
            let frac = (self.pos - i as f64) as f32;
            let next = buf.get(i + 1).copied().unwrap_or(buf[i]);
            out.push(buf[i] + (next - buf[i]) * frac);
            self.pos += self.step;
        }
        self.pos -= last;
        self.prev = buf[buf.len() - 1];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, rate: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (i as f32 * hz * 2.0 * std::f32::consts::PI / rate as f32).sin() * 0.5)
            .collect()
    }

    /// 过零点个数（估频率用）。
    fn crossings(s: &[f32]) -> usize {
        s.windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count()
    }

    #[test]
    fn down_from_48k_keeps_length_and_pitch() {
        let input = sine(440.0, 48_000, 48_000);
        let mut out = Vec::new();
        Resampler::new(48_000, 16_000).process(&input, &mut out);
        assert!((out.len() as i64 - 16_000).abs() <= 1, "长度 {}", out.len());
        // 1 秒 440 Hz 约 880 个过零点
        assert!((crossings(&out) as i64 - crossings(&input) as i64).abs() <= 2);
        let peak = out[1000..].iter().fold(0f32, |m, s| m.max(s.abs()));
        assert!((0.4..=0.55).contains(&peak), "幅度 {peak}");
    }

    #[test]
    fn up_and_odd_rates() {
        let mut out = Vec::new();
        Resampler::new(16_000, 48_000).process(&sine(300.0, 16_000, 16_000), &mut out);
        assert!((out.len() as i64 - 48_000).abs() <= 3);
        let mut out = Vec::new();
        Resampler::new(44_100, 16_000).process(&sine(300.0, 44_100, 44_100), &mut out);
        assert!((out.len() as i64 - 16_000).abs() <= 1);
    }

    #[test]
    fn chunks_join_up_like_one_block() {
        let input = sine(500.0, 48_000, 4800);
        let mut whole = Vec::new();
        Resampler::new(48_000, 16_000).process(&input, &mut whole);
        let mut pieces = Vec::new();
        let mut r = Resampler::new(48_000, 16_000);
        for chunk in input.chunks(441) {
            r.process(chunk, &mut pieces);
        }
        assert_eq!(whole.len(), pieces.len());
        assert!(whole.iter().zip(&pieces).all(|(a, b)| (a - b).abs() < 1e-6));
    }
}
