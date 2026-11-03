//! 队伍文字（Alt+T）：清理输入、认出投票指令。移植自 lolPing `src/shared/textCommands.ts` 和
//! `roomProtocol.ts` 的 sanitizeText（MIT，© 2026 Danrui Meng）。
//!
//! 实现流程：
//! 1. 去掉控制字符、零宽字符、双向控制符这类看不见的字（能让一句话看起来是空的或者倒过来），去掉首尾空白，最多 120 个字。
//! 2. 以 / 开头的是指令：/ff（/surrender）发起投降，/noff 对投降投反对，/ready 发起准备确认；别的斜杠开头的是不认识的指令。
//! 3. 收到队友的文字也过第 1 条，空了就不显示。

pub const MAX_CHARS: usize = 120;

fn unsafe_char(c: char) -> bool {
    matches!(c,
        '\u{0000}'..='\u{001f}' | '\u{007f}'..='\u{009f}' | '\u{00ad}' | '\u{034f}' | '\u{061c}' | '\u{115f}' | '\u{1160}'
        | '\u{180e}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{3164}' | '\u{feff}'
        | '\u{ffa0}')
}

pub fn sanitize(raw: &str) -> String {
    let kept: String = raw.chars().filter(|c| !unsafe_char(*c)).collect();
    kept.trim()
        .chars()
        .take(MAX_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    Text(String),
    Ff,
    NoFf,
    Ready,
    Unknown,
    Empty,
}

pub fn parse(raw: &str) -> Input {
    let text = sanitize(raw);
    if text.is_empty() {
        return Input::Empty;
    }
    if !text.starts_with('/') {
        return Input::Text(text);
    }
    match text.to_lowercase().as_str() {
        "/ff" | "/surrender" => Input::Ff,
        "/noff" => Input::NoFf,
        "/ready" => Input::Ready,
        _ => Input::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invisible_characters_and_length_are_cleaned() {
        assert_eq!(sanitize("  上\u{200b}路\u{202e}来人\n "), "上路来人");
        assert_eq!(sanitize("\u{3164}\u{200b} "), "");
        let long = "钢".repeat(130);
        assert_eq!(sanitize(&long).chars().count(), MAX_CHARS);
        assert_eq!(sanitize("😀 好"), "😀 好");
    }

    #[test]
    fn slash_words_are_vote_commands() {
        assert_eq!(parse(" /FF "), Input::Ff);
        assert_eq!(parse("/surrender"), Input::Ff);
        assert_eq!(parse("/noff"), Input::NoFf);
        assert_eq!(parse("/ready"), Input::Ready);
        assert_eq!(parse("/ff now"), Input::Unknown);
        assert_eq!(parse("/kick"), Input::Unknown);
        assert_eq!(parse("打龙 /ff"), Input::Text("打龙 /ff".into()));
        assert_eq!(parse(" \u{200b} "), Input::Empty);
    }
}
