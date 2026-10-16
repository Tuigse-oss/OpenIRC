//! IRC color and attribute codes inside a message.
//!
//! `\x03` selects foreground and background colors from the 16-color
//! palette; bold, italic, underline, strikethrough, and reverse toggle; `\x0f`
//! resets everything.

pub const PALETTE: [[u8; 3]; 16] = [
    [255, 255, 255],
    [0, 0, 0],
    [0, 0, 127],
    [0, 147, 0],
    [255, 0, 0],
    [127, 0, 0],
    [156, 0, 156],
    [252, 127, 0],
    [255, 255, 0],
    [0, 252, 0],
    [0, 147, 147],
    [0, 255, 255],
    [0, 0, 252],
    [255, 0, 255],
    [127, 127, 127],
    [210, 210, 210],
];

/// Text attributes that apply to a run of text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub reverse: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub color: Option<u8>,
    pub background: Option<u8>,
    pub style: Style,
    pub text: String,
}

/// Split a message into runs of text with their color and attributes.
/// Colors 0-15 use `PALETTE`; 16-98 (the extended range) and 99 fall back to
/// the default color.
pub fn parse(input: &str) -> Vec<Span> {
    let chars: Vec<char> = input.chars().collect();
    let mut spans = Vec::new();
    let mut buf = String::new();
    let mut color = None;
    let mut background = None;
    let mut style = Style::default();
    let mut index = 0;
    while index < chars.len() {
        let before = (color, background, style);
        match chars[index] {
            '\u{0003}' => {
                index += 1;
                let (foreground, used) = read_index(&chars[index..]);
                if used == 0 {
                    color = None;
                    background = None;
                } else {
                    index += used;
                    color = foreground;
                    if index + 1 < chars.len()
                        && chars[index] == ','
                        && chars[index + 1].is_ascii_digit()
                    {
                        index += 1;
                        let (back, used) = read_index(&chars[index..]);
                        index += used;
                        background = back;
                    }
                }
            }
            '\u{000f}' => {
                color = None;
                background = None;
                style = Style::default();
                index += 1;
            }
            '\u{0002}' => {
                style.bold = !style.bold;
                index += 1;
            }
            '\u{001d}' => {
                style.italic = !style.italic;
                index += 1;
            }
            '\u{001f}' => {
                style.underline = !style.underline;
                index += 1;
            }
            '\u{001e}' => {
                style.strike = !style.strike;
                index += 1;
            }
            '\u{0016}' => {
                style.reverse = !style.reverse;
                index += 1;
            }
            '\u{0011}' => index += 1,
            ch => {
                buf.push(ch);
                index += 1;
                continue;
            }
        }
        if (color, background, style) != before {
            push(&mut spans, before, &mut buf);
        }
    }
    push(&mut spans, (color, background, style), &mut buf);
    spans
}

/// The text without any color or attribute codes.
pub fn strip(input: &str) -> String {
    parse(input).into_iter().map(|span| span.text).collect()
}

fn push(spans: &mut Vec<Span>, state: (Option<u8>, Option<u8>, Style), buf: &mut String) {
    if buf.is_empty() {
        return;
    }
    spans.push(Span {
        color: state.0,
        background: state.1,
        style: state.2,
        text: std::mem::take(buf),
    });
}

fn read_index(chars: &[char]) -> (Option<u8>, usize) {
    if chars.first().is_none_or(|ch| !ch.is_ascii_digit()) {
        return (None, 0);
    }
    let width = if chars.get(1).is_some_and(|ch| ch.is_ascii_digit()) {
        2
    } else {
        1
    };
    let text: String = chars[..width].iter().collect();
    let value: u8 = text.parse().unwrap_or(0);
    if value < 16 {
        (Some(value), width)
    } else {
        (None, width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_colored_word_and_resets() {
        let spans = parse("hello \u{0003}04,02red\u{000f} plain");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].text, "hello ");
        assert_eq!(spans[0].color, None);
        assert_eq!(spans[1].text, "red");
        assert_eq!(spans[1].color, Some(4));
        assert_eq!(spans[1].background, Some(2));
        assert_eq!(spans[2].text, " plain");
        assert_eq!(spans[2].color, None);
        assert_eq!(spans[2].background, None);
    }

    #[test]
    fn keeps_attributes_as_style() {
        let spans = parse("a\u{0002}bold\u{001d}both\u{0002}italic\u{000f}plain");
        let styles: Vec<(String, bool, bool)> = spans
            .iter()
            .map(|span| (span.text.clone(), span.style.bold, span.style.italic))
            .collect();
        assert_eq!(
            styles,
            vec![
                ("a".to_string(), false, false),
                ("bold".to_string(), true, false),
                ("both".to_string(), true, true),
                ("italic".to_string(), false, true),
                ("plain".to_string(), false, false),
            ]
        );
    }

    #[test]
    fn extended_colors_fall_back_and_comma_text_is_kept() {
        let spans = parse("\u{0003}52x\u{0003}4,text");
        assert_eq!(spans[0].color, None);
        assert_eq!(spans[1].color, Some(4));
        assert_eq!(spans[1].text, ",text");
        assert_eq!(strip("\u{0002}a\u{0003}4b"), "ab");
    }
}
