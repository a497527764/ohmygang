//! Crockford Base32 编解码（房间码、指纹用）。
//!
//! 实现流程：
//! 1. 把字节看成一串比特（高位在前），每 5 比特对应一个字符，最后不足 5 比特的用 0 补齐。
//! 2. 字母表去掉了 I、L、O、U，避免和 1、0 混淆；解码时大小写不敏感，I、L 当 1，O 当 0。
//! 3. 解码时补齐用的比特必须是 0，否则视为输入有误。

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// 取 `data` 的前 `nbits` 位编码，输出 `nbits / 5` 向上取整个字符。
pub fn encode(data: &[u8], nbits: usize) -> String {
    let chars = nbits.div_ceil(5);
    let mut out = String::with_capacity(chars);
    for c in 0..chars {
        let mut v = 0u8;
        for b in 0..5 {
            let bit = c * 5 + b;
            v <<= 1;
            if bit < nbits && data[bit / 8] & (0x80 >> (bit % 8)) != 0 {
                v |= 1;
            }
        }
        out.push(ALPHABET[v as usize] as char);
    }
    out
}

/// `encode` 的逆过程。字符数必须正好对应 `nbits`，返回 `nbits / 8` 向上取整个字节。
pub fn decode(s: &str, nbits: usize) -> Option<Vec<u8>> {
    if s.chars().count() != nbits.div_ceil(5) {
        return None;
    }
    let mut out = vec![0u8; nbits.div_ceil(8)];
    for (c, ch) in s.chars().enumerate() {
        let v = value_of(ch)?;
        for b in 0..5 {
            let bit = c * 5 + b;
            let set = v & (0x10 >> b) != 0;
            if bit >= nbits {
                if set {
                    return None; // 补齐位必须为 0
                }
            } else if set {
                out[bit / 8] |= 0x80 >> (bit % 8);
            }
        }
    }
    Some(out)
}

fn value_of(ch: char) -> Option<u8> {
    let up = match ch.to_ascii_uppercase() {
        'I' | 'L' => '1',
        'O' => '0',
        other => other,
    };
    ALPHABET
        .iter()
        .position(|&a| a as char == up)
        .map(|p| p as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_aliases() {
        let data = [0xde, 0xad, 0xbe, 0xef, 0x01];
        let s = encode(&data, 40);
        assert_eq!(s, "VTPVXVR1");
        assert_eq!(decode(&s, 40), Some(data.to_vec()));
        // 小写、I/L/O 别名都能解
        assert_eq!(decode("vtpvxvri", 40), Some(data.to_vec()));
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(decode("VTPVXVR", 40), None); // 长度不对
        assert_eq!(decode("VTPVXVRU", 40), None); // U 不在字母表
        // 12 位只用到第 3 个字符的前 2 位，后 3 位是补齐位，必须为 0
        assert_eq!(decode("001", 12), None);
        assert_eq!(decode("008", 12), Some(vec![0, 0x10]));
    }
}
