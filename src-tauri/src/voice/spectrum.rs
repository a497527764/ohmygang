//! 麦克风频谱：把最近一窗采样做 512 点 FFT，按对数频率分成 BANDS 段，每段给出 0..1 的幅度。
//! 主页的电平表因此是「不同频率各自的波幅」，而不是整排柱子共用同一个音量。
//! 纯 Rust 实现（不引第三方 DSP crate）：基 2 Cooley-Tukey + Hann 窗 + 频段能量平均。

/// 发给前端的频段数。窗口 500px 时主页画 17 根、设置页画 56 根，前端会插值到实际柱数。
pub const BANDS: usize = 24;
/// FFT 点数：16 kHz 采样下约 32 ms 一窗，频率分辨率 31.25 Hz。
const N: usize = 512;
/// 关注的人声/环境声频率范围。
const F_MIN: f32 = 70.0;
const F_MAX: f32 = 7600.0;
const RATE: f32 = 16_000.0;

/// 原地基 2 FFT（实部、虚部分开存）。调用方保证长度是 2 的幂。
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2usize;
    while len <= n {
        let angle = -2.0 * std::f32::consts::PI / len as f32;
        let (step_sin, step_cos) = angle.sin_cos();
        let half = len / 2;
        for start in (0..n).step_by(len) {
            let (mut wr, mut wi) = (1.0f32, 0.0f32);
            for k in 0..half {
                let a = start + k;
                let b = a + half;
                let ur = re[a];
                let ui = im[a];
                let vr = re[b] * wr - im[b] * wi;
                let vi = re[b] * wi + im[b] * wr;
                re[a] = ur + vr;
                im[a] = ui + vi;
                re[b] = ur - vr;
                im[b] = ui - vi;
                let next_wr = wr * step_cos - wi * step_sin;
                wi = wr * step_sin + wi * step_cos;
                wr = next_wr;
            }
        }
        len <<= 1;
    }
}

/// 对数分段的边界（FFT bin 下标），相邻段至少跨一个 bin，避免空段。
fn band_edges() -> [usize; BANDS + 1] {
    let mut edges = [0usize; BANDS + 1];
    let bins = N / 2;
    let hz_per_bin = RATE / N as f32;
    let ratio = F_MAX / F_MIN;
    for (i, e) in edges.iter_mut().enumerate() {
        let hz = F_MIN * ratio.powf(i as f32 / BANDS as f32);
        *e = ((hz / hz_per_bin).round() as usize).clamp(1, bins);
    }
    for i in 1..=BANDS {
        if edges[i] <= edges[i - 1] {
            edges[i] = (edges[i - 1] + 1).min(bins);
        }
    }
    edges
}

/// 采样 → 各频段幅度。samples 里只取最后 N 个，不足时前面补零。
pub fn analyze(samples: &[f32]) -> [f32; BANDS] {
    let mut re = [0f32; N];
    let mut im = [0f32; N];
    let take = samples.len().min(N);
    let src = &samples[samples.len() - take..];
    let at = N - take;
    for (i, &s) in src.iter().enumerate() {
        // Hann 窗：抑制帧边界的频谱泄漏
        let w = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / N as f32).cos();
        re[at + i] = s * w;
    }
    fft(&mut re, &mut im);

    let edges = band_edges();
    let scale = 2.0 / N as f32;
    let mut out = [0f32; BANDS];
    for b in 0..BANDS {
        let (lo, hi) = (edges[b], edges[b + 1].max(edges[b] + 1));
        let mut power = 0f64;
        let mut count = 0usize;
        for k in lo..hi {
            let (r, i) = (f64::from(re[k]) * f64::from(scale), f64::from(im[k]) * f64::from(scale));
            power += r * r + i * i;
            count += 1;
        }
        let amp = (power / count.max(1) as f64).sqrt() as f32;
        // 高频能量天然更弱，稍微抬一点让整排柱子都有信息量；再按经验增益顶到 0..1
        let tilt = 1.0 + 1.1 * (b as f32 / BANDS as f32);
        out[b] = (amp * tilt * 34.0).min(1.0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pure_tone_lights_only_its_band() {
        let hz = 1000.0;
        let samples: Vec<f32> = (0..N)
            .map(|i| (2.0 * std::f32::consts::PI * hz * i as f32 / RATE).sin() * 0.2)
            .collect();
        let bands = analyze(&samples);
        let peak = bands.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        let edges = band_edges();
        let hz_per_bin = RATE / N as f32;
        assert!(hz >= edges[peak] as f32 * hz_per_bin - 40.0);
        assert!(hz <= edges[peak + 1] as f32 * hz_per_bin + 40.0);
        // 不是「所有柱子一样高」：峰值明显大于最低段
        assert!(bands[peak] > *bands.iter().min_by(|a, b| a.total_cmp(b)).unwrap() + 0.2);
    }

    #[test]
    fn silence_is_silent() {
        assert!(analyze(&vec![0f32; N]).iter().all(|&v| v == 0.0));
    }
}
