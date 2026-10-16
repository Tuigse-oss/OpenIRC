//! Dialog layouts in a plain text form.
//!
//! A dialog is a title, a size, and controls placed in pixels:
//! text, edit, check, and button. The editor draws that layout and can
//! turn it back into the same text.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Control {
    pub kind: String,
    pub id: u16,
    pub label: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub command: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dialog {
    pub name: String,
    pub title: String,
    pub width: i32,
    pub height: i32,
    pub controls: Vec<Control>,
}

impl Dialog {
    pub fn empty(name: &str) -> Self {
        Self {
            name: name.to_string(),
            title: name.to_string(),
            width: 280,
            height: 160,
            controls: Vec::new(),
        }
    }
}

pub fn parse(text: &str) -> Option<Dialog> {
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let header = lines.next()?;
    let mut header = header.strip_prefix("dialog")?.trim();
    header = header.strip_suffix('{').unwrap_or(header).trim();
    if header.is_empty() {
        return None;
    }
    let mut dialog = Dialog::empty(header);
    for line in lines {
        if line == "}" {
            break;
        }
        if let Some(title) = line.strip_prefix("title") {
            let title = title.trim();
            dialog.title = match title.strip_prefix('"') {
                Some(quoted) => take_quoted(quoted).map(|(text, _)| text)?,
                None => title.to_string(),
            };
        } else if let Some(rest) = line.strip_prefix("size") {
            let mut parts = rest.split_whitespace();
            dialog.width = parts.next()?.parse().ok()?;
            dialog.height = parts.next()?.parse().ok()?;
        } else if let Some(control) = parse_control(line) {
            dialog.controls.push(control);
        }
    }
    Some(dialog)
}

pub fn emit(dialog: &Dialog) -> String {
    let mut out = format!(
        "dialog {} {{\n  title {}\n  size {} {}\n",
        dialog.name,
        quote(&dialog.title),
        dialog.width,
        dialog.height
    );
    for control in &dialog.controls {
        out.push_str(&format!(
            "  {} {}, {}, {} {} {} {}",
            control.kind,
            quote(&control.label),
            control.id,
            control.x,
            control.y,
            control.w,
            control.h
        ));
        if !control.command.is_empty() {
            out.push_str(", ");
            out.push_str(&control.command);
        }
        out.push('\n');
    }
    out.push_str("}\n");
    out
}

/// `"text"` with `\` and `"` escaped by a backslash.
fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Read a quoted string whose opening quote was already removed. Returns the
/// text and what follows the closing quote.
fn take_quoted(source: &str) -> Option<(String, &str)> {
    let mut out = String::new();
    let mut chars = source.char_indices();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '\\' => {
                if let Some((_, escaped)) = chars.next() {
                    out.push(escaped);
                }
            }
            '"' => return Some((out, &source[index + 1..])),
            _ => out.push(ch),
        }
    }
    None
}

/// A control line: `kind "label", id, x y w h[, command]`. Everything after
/// the comma that follows the position is the command, commas included.
fn parse_control(line: &str) -> Option<Control> {
    let (kind, rest) = line.split_once(char::is_whitespace)?;
    if !matches!(kind, "text" | "edit" | "button" | "check") {
        return None;
    }
    let rest = rest.trim();
    let (label, rest) = if let Some(quoted) = rest.strip_prefix('"') {
        let (label, rest) = take_quoted(quoted)?;
        (label, rest.trim().strip_prefix(',')?.trim())
    } else {
        let (label, rest) = rest.split_once(',')?;
        (label.trim().to_string(), rest.trim())
    };
    let (id, rest) = rest.split_once(',')?;
    let id: u16 = id.trim().parse().ok()?;
    let (place, command) = match rest.split_once(',') {
        Some((place, command)) => (place, command.trim()),
        None => (rest, ""),
    };
    let mut place = place.split_whitespace();
    let x = place.next()?.parse().ok()?;
    let y = place.next()?.parse().ok()?;
    let w = place.next()?.parse().ok()?;
    let h = place.next()?.parse().ok()?;
    Some(Control {
        kind: kind.to_string(),
        id,
        label,
        x,
        y,
        w,
        h,
        command: command.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialog_text_round_trips() {
        let source = "\
dialog greet {
  title \"Greet\"
  size 220 120
  text \"Name\", 1, 8 8 40 16
  edit \"\", 2, 52 8 150 16
  button \"OK\", 3, 160 80 50 20, /echo hello
}
";
        let dialog = parse(source).unwrap();
        assert_eq!(dialog.name, "greet");
        assert_eq!(dialog.title, "Greet");
        assert_eq!(dialog.controls.len(), 3);
        assert_eq!(dialog.controls[2].command, "/echo hello");
        let again = parse(&emit(&dialog)).unwrap();
        assert_eq!(again.controls[0].label, "Name");
        assert_eq!(again.controls[1].kind, "edit");
        assert_eq!(again.controls[2].command, "/echo hello");
    }

    #[test]
    fn commas_in_commands_and_quotes_in_labels_round_trip() {
        let mut dialog = Dialog::empty("demo");
        dialog.title = "Say \"hi\"".into();
        dialog.controls.push(Control {
            kind: "button".into(),
            id: 4,
            label: "A \"quoted\" \\ label".into(),
            x: 160,
            y: 100,
            w: 80,
            h: 20,
            command: "/msg #openirc hi, there, friend".into(),
        });
        let again = parse(&emit(&dialog)).unwrap();
        assert_eq!(again, dialog);
        let typed = parse("dialog d {\n  button \"Go\", 1, 1 2 3 4, /echo a, b\n}").unwrap();
        assert_eq!(typed.controls[0].command, "/echo a, b");
    }
}
