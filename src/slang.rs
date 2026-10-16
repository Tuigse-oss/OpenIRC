//! The script language for the commands and events people write.
//!
//! OpenIRC's own interpreter: `on`
//! events with user levels, `alias` blocks, `if` / `elseif` / `else` and
//! `while` with `&&`, `||`, and `!`, `%variables`, `/return` and `/halt`,
//! and identifiers such as `$nick`, `$1-`, `$iif`, `$calc`, `$left`,
//! `$replace`, `$gettok`, `$read`, and `$hget`. An alias that returns a
//! value can be used as an identifier: `$name(args)`. `hadd`, `hdel`, and
//! `hfree` keep a hash table.

/// Loops stop after this many rounds, so a script cannot hang the window.
const LOOP_LIMIT: usize = 1_000;
/// Work one event or alias may do in all, counting statements, loop rounds
/// and every 1 KiB of text built, across nested loops and aliases. Past it
/// the script stops, so nesting loops cannot multiply the work.
const STEP_LIMIT: u32 = 20_000;
/// The longest value a variable or identifier holds, in bytes.
const VALUE_LIMIT: usize = 16 * 1024;
/// Text being built may run past `VALUE_LIMIT` by this much before it is
/// stopped, so `$+` marks removed afterwards still leave a full value.
const BUILD_LIMIT: usize = VALUE_LIMIT + 1024;
/// All variables together hold at most this many bytes.
const VARS_LIMIT: usize = 4 * 1024 * 1024;
/// Commands one event or alias may send.
const OUTPUT_LIMIT: usize = 200;
/// `$read` reads files up to this size.
const READ_LIMIT: u64 = 1024 * 1024;
/// What the window shows when a script was stopped for doing too much.
const STOPPED: &str = "/echo Script stopped: it did too much work at once.";
/// Aliases used as identifiers may call each other this deep.
const CALL_DEPTH: u8 = 8;
/// Marks where `$+` joined two words, so the spaces around it go.
const JOIN: char = '\u{0}';

#[derive(Clone, Debug, PartialEq, Eq)]
struct Handler {
    /// `*` for everyone, a number for that level or higher, or a name.
    level: String,
    event: String,
    text_mask: String,
    target_mask: String,
    body: Vec<Stmt>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Alias {
    name: String,
    body: Vec<Stmt>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Stmt {
    Line(String),
    Assign {
        name: String,
        value: String,
    },
    If {
        cond: String,
        then_body: Vec<Stmt>,
        else_body: Vec<Stmt>,
    },
    While {
        cond: String,
        body: Vec<Stmt>,
    },
    Return(String),
    Halt,
}

/// How a block ended.
enum Flow {
    Next,
    Return(String),
    Halt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    handlers: Vec<Handler>,
    aliases: Vec<Alias>,
}

#[derive(Clone, Debug)]
pub struct Fields<'a> {
    pub nick: &'a str,
    pub chan: &'a str,
    pub me: &'a str,
    pub text: &'a str,
    pub time: &'a str,
    pub args: &'a [&'a str],
    pub nicks: &'a [&'a str],
    /// The user levels of the person who caused the event.
    pub levels: &'a [&'a str],
}

/// What a running script can see: the event fields and the whole program,
/// for aliases called as identifiers.
struct Scope<'a> {
    fields: &'a Fields<'a>,
    program: &'a Program,
    depth: u8,
    /// Steps left for this whole run; see `STEP_LIMIT`.
    steps: &'a std::cell::Cell<u32>,
}

impl Scope<'_> {
    /// Use `count` steps; false once the run has none left.
    fn spend(&self, count: u32) -> bool {
        let left = self.steps.get();
        self.steps.set(left.saturating_sub(count));
        left > count
    }

    fn exhausted(&self) -> bool {
        self.steps.get() == 0
    }
}

/// `text` cut to at most `limit` bytes, on a character boundary. The memory
/// past the cut is given back, so what is kept is what is counted.
fn cut(mut text: String, limit: usize) -> String {
    if text.len() > limit {
        text.truncate(boundary(&text, limit));
        text.shrink_to_fit();
    }
    text
}

/// The largest character boundary in `text` at or before `limit`.
fn boundary(text: &str, limit: usize) -> usize {
    let mut end = limit.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

/// Append `value` to `out` without letting `out` grow past `limit` bytes.
/// False once `out` is full.
fn push_within(out: &mut String, value: &str, limit: usize) -> bool {
    let room = limit.saturating_sub(out.len());
    if value.len() <= room {
        out.push_str(value);
        true
    } else {
        out.push_str(&value[..boundary(value, room)]);
        false
    }
}

type Vars = Vec<(String, String)>;

pub fn check(source: &str) -> Result<(), String> {
    parse(source).map(|_| ())
}

/// The parsed form of the current script, kept until the text changes, so
/// events and aliases do not parse the whole script every time.
#[derive(Default)]
pub struct Cache {
    source: String,
    parsed: Option<Result<std::rc::Rc<Program>, String>>,
}

impl Cache {
    pub fn program(&mut self, source: &str) -> Result<std::rc::Rc<Program>, String> {
        if self.parsed.is_none() || self.source != source {
            self.source = source.to_string();
            self.parsed = Some(parse(source).map(std::rc::Rc::new));
        }
        self.parsed
            .clone()
            .unwrap_or_else(|| Err("script not parsed".into()))
    }
}

#[cfg(test)]
fn run_event(
    source: &str,
    event: &str,
    fields: &Fields,
    vars: &mut Vars,
) -> Result<Vec<String>, String> {
    Ok(run_event_in(&parse(source)?, event, fields, vars))
}

/// Run every handler for `event` that matches, in order. `/halt` stops
/// the rest.
pub fn run_event_in(
    program: &Program,
    event: &str,
    fields: &Fields,
    vars: &mut Vars,
) -> Vec<String> {
    let steps = std::cell::Cell::new(STEP_LIMIT);
    let scope = Scope {
        fields,
        program,
        depth: 0,
        steps: &steps,
    };
    let mut out = Vec::new();
    for handler in &program.handlers {
        if !handler.event.eq_ignore_ascii_case(event)
            || !level_matches(&handler.level, fields.levels)
            || !event_matches(handler, event, fields)
        {
            continue;
        }
        if let Flow::Halt = exec(&handler.body, &scope, vars, &mut out) {
            break;
        }
    }
    if scope.exhausted() {
        out.push(STOPPED.to_string());
    }
    out
}

#[cfg(test)]
fn run_alias(
    source: &str,
    name: &str,
    fields: &Fields,
    vars: &mut Vars,
) -> Result<Option<Vec<String>>, String> {
    Ok(run_alias_in(&parse(source)?, name, fields, vars))
}

pub fn run_alias_in(
    program: &Program,
    name: &str,
    fields: &Fields,
    vars: &mut Vars,
) -> Option<Vec<String>> {
    let alias = program
        .aliases
        .iter()
        .find(|alias| alias.name.eq_ignore_ascii_case(name))?;
    let steps = std::cell::Cell::new(STEP_LIMIT);
    let scope = Scope {
        fields,
        program,
        depth: 0,
        steps: &steps,
    };
    let mut out = Vec::new();
    exec(&alias.body, &scope, vars, &mut out);
    if scope.exhausted() {
        out.push(STOPPED.to_string());
    }
    Some(out)
}

/// Whether a handler's level lets a person with `levels` through. A
/// number lets equal or higher numbers through; a name must be held.
fn level_matches(level: &str, levels: &[&str]) -> bool {
    let level = level.trim().trim_start_matches(['+', '!', '@']);
    if level.is_empty() || level == "*" {
        return true;
    }
    match level.parse::<u32>() {
        Ok(needed) => levels
            .iter()
            .filter_map(|held| held.parse::<u32>().ok())
            .any(|held| held >= needed),
        Err(_) => levels.iter().any(|held| held.eq_ignore_ascii_case(level)),
    }
}

fn event_matches(handler: &Handler, event: &str, fields: &Fields) -> bool {
    let target_ok = match handler.target_mask.as_str() {
        "" | "*" => true,
        "#" => fields.chan.starts_with('#'),
        "?" => !fields.chan.starts_with('#'),
        mask => crate::script::mask_match(mask, fields.chan),
    };
    if !target_ok {
        return false;
    }
    if matches!(event, "text" | "action" | "notice" | "ctcp") {
        crate::script::mask_match(&handler.text_mask, fields.text)
    } else {
        true
    }
}

fn exec(body: &[Stmt], scope: &Scope, vars: &mut Vars, out: &mut Vec<String>) -> Flow {
    for stmt in body {
        if !scope.spend(1) {
            return Flow::Halt;
        }
        match stmt {
            Stmt::Line(line) => {
                if builtin(line, scope, vars) {
                    continue;
                }
                let line = eval(line, scope, vars);
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if out.len() >= OUTPUT_LIMIT {
                    scope.steps.set(0);
                    return Flow::Halt;
                }
                if line.starts_with('/') {
                    out.push(line.to_string());
                } else {
                    out.push(format!("/{line}"));
                }
            }
            Stmt::Assign { name, value } => {
                let value = eval(value, scope, vars);
                set_var(vars, name, value);
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
            } => {
                let chosen = if cond_true(cond, scope, vars) {
                    then_body
                } else {
                    else_body
                };
                match exec(chosen, scope, vars, out) {
                    Flow::Next => {}
                    flow => return flow,
                }
            }
            Stmt::While { cond, body } => {
                for _ in 0..LOOP_LIMIT {
                    if !scope.spend(1) {
                        return Flow::Halt;
                    }
                    if !cond_true(cond, scope, vars) {
                        break;
                    }
                    match exec(body, scope, vars, out) {
                        Flow::Next => {}
                        flow => return flow,
                    }
                }
            }
            Stmt::Return(value) => return Flow::Return(eval(value, scope, vars)),
            Stmt::Halt => return Flow::Halt,
        }
    }
    Flow::Next
}

/// Evaluate a condition: `||` and `&&` (with `&&` binding tighter), `!`
/// in front of a test, parentheses, the comparisons `==`, `!=`, `>`,
/// `<`, `>=`, `<=`, `isin` and `iswm`, or a bare value that is true when
/// it is not empty, `0`, or `$false`.
fn cond_true(cond: &str, scope: &Scope, vars: &mut Vars) -> bool {
    let cond = cond.trim();
    if let Some(inner) = wrapped(cond) {
        return cond_true(inner, scope, vars);
    }
    let any = split_top(cond, "||");
    if any.len() > 1 {
        return any.iter().any(|part| cond_true(part, scope, vars));
    }
    let all = split_top(cond, "&&");
    if all.len() > 1 {
        return all.iter().all(|part| cond_true(part, scope, vars));
    }
    if let Some(rest) = cond.strip_prefix('!')
        && !rest.starts_with('=')
    {
        return !cond_true(rest, scope, vars);
    }
    for op in [" isin ", " iswm ", ">=", "<=", "==", "!=", ">", "<"] {
        let parts = split_top(cond, op);
        if parts.len() == 2 {
            let left = eval(&parts[0], scope, vars);
            let right = eval(&parts[1], scope, vars);
            return compare(&left, op.trim(), &right);
        }
    }
    let value = eval(cond, scope, vars);
    let value = value.trim();
    !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("$false")
}

/// The inside of `(…)` when the parentheses wrap the whole text.
fn wrapped(text: &str) -> Option<&str> {
    let inner = text.strip_prefix('(')?.strip_suffix(')')?;
    let mut depth = 0i32;
    for ch in inner.chars() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
            _ => {}
        }
    }
    (depth == 0).then_some(inner)
}

/// Split at `separator` where it is outside every parenthesis.
fn split_top(text: &str, separator: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        // Compare bytes: a separator is ASCII, so a match always starts on a
        // character boundary, and slicing the text there could not.
        if depth == 0 && bytes[index..].starts_with(separator.as_bytes()) {
            parts.push(text[start..index].to_string());
            index += separator.len();
            start = index;
            continue;
        }
        index += 1;
    }
    parts.push(text[start..].to_string());
    parts
}

fn compare(left: &str, op: &str, right: &str) -> bool {
    let left = left.trim();
    let right = right.trim();
    match op {
        "isin" => return right.to_lowercase().contains(&left.to_lowercase()),
        "iswm" => return crate::script::mask_match(left, right),
        _ => {}
    }
    if let (Ok(left_n), Ok(right_n)) = (left.parse::<f64>(), right.parse::<f64>()) {
        return match op {
            "==" => left_n == right_n,
            "!=" => left_n != right_n,
            ">" => left_n > right_n,
            "<" => left_n < right_n,
            ">=" => left_n >= right_n,
            "<=" => left_n <= right_n,
            _ => false,
        };
    }
    let ordering = left.to_lowercase().cmp(&right.to_lowercase());
    match op {
        "==" => ordering.is_eq(),
        "!=" => ordering.is_ne(),
        ">" => ordering.is_gt(),
        "<" => ordering.is_lt(),
        ">=" => ordering.is_ge(),
        "<=" => ordering.is_le(),
        _ => false,
    }
}

fn set_var(vars: &mut Vars, name: &str, value: String) {
    let name = name.trim_start_matches('%');
    let value = cut(value, VALUE_LIMIT);
    let held: usize = vars
        .iter()
        .filter(|(item, _)| item != name)
        .map(|(item, value)| item.len() + value.len())
        .sum();
    // Past the total limit a new value is refused; the old one stays.
    if held + name.len() + value.len() > VARS_LIMIT {
        return;
    }
    if let Some(slot) = vars.iter_mut().find(|(item, _)| item == name) {
        slot.1 = value;
    } else {
        vars.push((name.to_string(), value));
    }
}

fn var_value<'a>(vars: &'a [(String, String)], name: &str) -> &'a str {
    vars.iter()
        .find(|(item, _)| item == name)
        .map(|(_, value)| value.as_str())
        .unwrap_or("")
}

fn eval(input: &str, scope: &Scope, vars: &mut Vars) -> String {
    let mut out = String::new();
    let chars: Vec<char> = input.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '$' {
            if chars.get(index + 1) == Some(&'+')
                && chars
                    .get(index + 2)
                    .is_none_or(|next| !next.is_ascii_alphanumeric())
            {
                out.push(JOIN);
                index += 2;
                continue;
            }
            let (value, next) = eval_dollar(&chars, index, scope, vars);
            // Past the limit the rest would only be cut off again, so it is
            // not worked out at all.
            if !push_within(&mut out, &value, BUILD_LIMIT) {
                break;
            }
            index = next;
            continue;
        }
        if chars[index] == '%' {
            let start = index + 1;
            let mut end = start;
            while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') {
                end += 1;
            }
            let name: String = chars[start..end].iter().collect();
            let value = if name.is_empty() {
                "%"
            } else {
                var_value(vars, &name)
            };
            if !push_within(&mut out, value, BUILD_LIMIT) {
                break;
            }
            index = end;
            continue;
        }
        if out.len() >= BUILD_LIMIT {
            break;
        }
        out.push(chars[index]);
        index += 1;
    }
    if out.contains(JOIN) {
        out = join_marks(&out);
    }
    // Building long text is work too.
    scope.spend((out.len() / 1024) as u32);
    cut(out, VALUE_LIMIT)
}

/// Remove `$+` marks and the spaces on both sides of each.
fn join_marks(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == JOIN {
            while out.ends_with(' ') {
                out.pop();
            }
            while chars.peek() == Some(&' ') {
                chars.next();
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn eval_dollar(chars: &[char], index: usize, scope: &Scope, vars: &mut Vars) -> (String, usize) {
    let start = index + 1;
    let mut end = start;
    while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') {
        end += 1;
    }
    let name: String = chars[start..end].iter().collect();
    if end < chars.len() && chars[end] == '(' {
        let (inner, next) = take_paren(chars, end);
        let args = split_args(&inner);
        let value = cut(identifier(&name, &inner, &args, scope, vars), VALUE_LIMIT);
        return (value, next);
    }
    if !name.is_empty() && name.chars().all(|ch| ch.is_ascii_digit()) {
        let number: usize = name.parse().unwrap_or(0);
        let ranged = end < chars.len() && chars[end] == '-';
        let next = if ranged { end + 1 } else { end };
        let args = scope.fields.args;
        let value = if number == 0 {
            String::new()
        } else if ranged {
            args.iter()
                .skip(number - 1)
                .copied()
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            args.get(number - 1).copied().unwrap_or("").to_string()
        };
        return (value, next);
    }
    let fields = scope.fields;
    let value = match name.to_ascii_lowercase().as_str() {
        "nick" => fields.nick.to_string(),
        "chan" => fields.chan.to_string(),
        "me" => fields.me.to_string(),
        "text" => fields.text.to_string(),
        "time" => fields.time.to_string(),
        "true" => "$true".to_string(),
        "false" => "$false".to_string(),
        "null" => String::new(),
        "ctime" => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs().to_string())
            .unwrap_or_default(),
        "date" => {
            let now = crate::clock::now();
            format!("{:04}-{:02}-{:02}", now.year, now.month, now.day)
        }
        _ => match call_alias(&name, Vec::new(), scope, vars) {
            Some(value) => value,
            // Unknown identifiers stay as written, so a stray `$` in text
            // such as a price is not eaten.
            None if name.is_empty() => "$".to_string(),
            None => String::new(),
        },
    };
    (value, end)
}

/// A `$name(…)` identifier. `inner` is the raw text between the
/// parentheses and `args` the same text split at top-level commas.
fn identifier(name: &str, inner: &str, args: &[String], scope: &Scope, vars: &mut Vars) -> String {
    let arg = |index: usize, vars: &mut Vars| -> String {
        args.get(index)
            .map(|text| eval(text, scope, vars))
            .unwrap_or_default()
    };
    match name.to_ascii_lowercase().as_str() {
        "iif" => {
            let yes = cond_true(args.first().map(String::as_str).unwrap_or(""), scope, vars);
            arg(if yes { 1 } else { 2 }, vars)
        }
        "calc" => calc(&eval(inner, scope, vars)),
        "len" => eval(inner, scope, vars).chars().count().to_string(),
        "lower" => eval(inner, scope, vars).to_lowercase(),
        "upper" => eval(inner, scope, vars).to_uppercase(),
        "strip" => crate::colors::strip(&eval(inner, scope, vars)),
        "mid" => {
            let text = arg(0, vars);
            let start = arg(1, vars).trim().parse::<i64>().unwrap_or(1);
            let count = args
                .get(2)
                .map(|_| arg(2, vars).trim().parse::<i64>().unwrap_or(0));
            mid(&text, start, count)
        }
        "left" => {
            let text = arg(0, vars);
            let count = arg(1, vars).trim().parse::<i64>().unwrap_or(0);
            left(&text, count)
        }
        "right" => {
            let text = arg(0, vars);
            let count = arg(1, vars).trim().parse::<i64>().unwrap_or(0);
            right(&text, count)
        }
        "replace" => {
            let mut text = arg(0, vars);
            let mut index = 1;
            while index + 1 < args.len() {
                let from = arg(index, vars);
                let to = arg(index + 1, vars);
                text = replace_nocase(&text, &from, &to, scope);
                index += 2;
            }
            text
        }
        "remove" => {
            let mut text = arg(0, vars);
            for index in 1..args.len() {
                let gone = arg(index, vars);
                text = replace_nocase(&text, &gone, "", scope);
            }
            text
        }
        "pos" => {
            let text = arg(0, vars).to_lowercase();
            let needle = arg(1, vars).to_lowercase();
            let nth = args
                .get(2)
                .map(|_| arg(2, vars).trim().parse::<usize>().unwrap_or(1))
                .unwrap_or(1);
            position(&text, &needle, nth).to_string()
        }
        "str" => {
            let text = arg(0, vars);
            let times = arg(1, vars)
                .trim()
                .parse::<usize>()
                .unwrap_or(0)
                .min(10_000)
                .min(VALUE_LIMIT / text.len().max(1) + 1);
            cut(text.repeat(times), VALUE_LIMIT)
        }
        "gettok" => {
            let text = arg(0, vars);
            let which = arg(1, vars);
            let sep = token_char(&arg(2, vars));
            gettok(&text, which.trim(), sep)
        }
        "numtok" => {
            let text = arg(0, vars);
            let sep = token_char(&arg(1, vars));
            tokens(&text, sep).len().to_string()
        }
        "istok" => {
            let text = arg(0, vars);
            let token = arg(1, vars);
            let sep = token_char(&arg(2, vars));
            let found = tokens(&text, sep)
                .iter()
                .any(|item| item.eq_ignore_ascii_case(&token));
            if found { "$true" } else { "$false" }.to_string()
        }
        "chr" => arg(0, vars)
            .trim()
            .parse::<u32>()
            .ok()
            .and_then(char::from_u32)
            .map(String::from)
            .unwrap_or_default(),
        "asc" => arg(0, vars)
            .chars()
            .next()
            .map(|ch| (ch as u32).to_string())
            .unwrap_or_default(),
        "abs" => number(calc(&arg(0, vars)).parse::<f64>().unwrap_or(0.0).abs()),
        "int" => number(calc(&arg(0, vars)).parse::<f64>().unwrap_or(0.0).trunc()),
        "round" => {
            let value = calc(&arg(0, vars)).parse::<f64>().unwrap_or(0.0);
            let places = arg(1, vars).trim().parse::<i32>().unwrap_or(0).clamp(0, 10);
            let scale = 10f64.powi(places);
            number((value * scale).round() / scale)
        }
        "rand" => {
            let low = arg(0, vars).trim().parse::<i64>().unwrap_or(0);
            let high = arg(1, vars).trim().parse::<i64>().unwrap_or(low);
            rand_between(low, high, random()).to_string()
        }
        "read" => {
            let path = arg(0, vars);
            let line = args
                .get(1)
                .map(|_| arg(1, vars).trim().parse::<usize>().unwrap_or(1))
                .unwrap_or(1);
            read_line(&path, line)
        }
        "hget" => {
            let table = arg(0, vars);
            let item = arg(1, vars);
            var_value(vars, &hash_key(&table, &item)).to_string()
        }
        "nick" => {
            let channel = arg(0, vars);
            let index = arg(1, vars).trim().parse::<usize>().unwrap_or(1);
            if !channel.is_empty() && !channel.eq_ignore_ascii_case(scope.fields.chan) {
                String::new()
            } else {
                scope
                    .fields
                    .nicks
                    .get(index.saturating_sub(1))
                    .copied()
                    .unwrap_or("")
                    .to_string()
            }
        }
        _ => {
            let values: Vec<String> = (0..args.len()).map(|index| arg(index, vars)).collect();
            call_alias(name, values, scope, vars).unwrap_or_default()
        }
    }
}

/// Run an alias as an identifier with `args` as `$1`, `$2`, …, and give
/// back what it returns. Commands inside it are not sent.
fn call_alias(name: &str, args: Vec<String>, scope: &Scope, vars: &mut Vars) -> Option<String> {
    if name.is_empty() || scope.depth >= CALL_DEPTH {
        return None;
    }
    let alias = scope
        .program
        .aliases
        .iter()
        .find(|alias| alias.name.eq_ignore_ascii_case(name))?;
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let fields = Fields {
        args: &words,
        ..scope.fields.clone()
    };
    let inner = Scope {
        fields: &fields,
        program: scope.program,
        depth: scope.depth + 1,
        steps: scope.steps,
    };
    let mut ignored = Vec::new();
    match exec(&alias.body, &inner, vars, &mut ignored) {
        Flow::Return(value) => Some(value),
        Flow::Next | Flow::Halt => Some(String::new()),
    }
}

fn mid(text: &str, start: i64, count: Option<i64>) -> String {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len() as i64;
    let first = if start < 0 { len + start } else { start - 1 }.clamp(0, len);
    let take = match count {
        Some(count) if count < 0 => (len - first + count).max(0),
        Some(count) => count,
        None => len - first,
    };
    chars
        .iter()
        .skip(first as usize)
        .take(take.max(0) as usize)
        .collect()
}

/// The first `count` characters; a negative count drops that many from the end.
fn left(text: &str, count: i64) -> String {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len() as i64;
    let keep = if count < 0 { len + count } else { count }.clamp(0, len);
    chars[..keep as usize].iter().collect()
}

/// The last `count` characters; a negative count drops that many from the start.
fn right(text: &str, count: i64) -> String {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len() as i64;
    let keep = if count < 0 { len + count } else { count }.clamp(0, len);
    chars[(len - keep) as usize..].iter().collect()
}

/// `text` with every match of `from`, in any case, replaced by `to`. The
/// result never grows past `VALUE_LIMIT`, and the comparing counts against
/// the run's steps, so a long text and a long pattern cannot stall a run.
fn replace_nocase(text: &str, from: &str, to: &str, scope: &Scope) -> String {
    if from.is_empty() {
        return text.to_string();
    }
    let needle: Vec<char> = from.chars().flat_map(char::to_lowercase).collect();
    let compared = std::cell::Cell::new(0usize);
    // The byte length of the match starting at `at`, comparing lowercase
    // letters but always ending on a character of the original text.
    let match_at = |at: usize| -> Option<usize> {
        let mut wanted = needle.iter();
        let mut want = wanted.next();
        for (offset, ch) in text[at..].char_indices() {
            compared.set(compared.get() + 1);
            for lower in ch.to_lowercase() {
                if want != Some(&lower) {
                    return None;
                }
                want = wanted.next();
            }
            if want.is_none() {
                return Some(offset + ch.len_utf8());
            }
        }
        None
    };
    let mut out = String::new();
    let mut at = 0;
    while at < text.len() {
        if let Some(len) = match_at(at) {
            if !push_within(&mut out, to, VALUE_LIMIT) {
                break;
            }
            at += len;
        } else {
            let ch = text[at..].chars().next().unwrap_or_default();
            if !push_within(&mut out, ch.encode_utf8(&mut [0; 4]), VALUE_LIMIT) {
                break;
            }
            at += ch.len_utf8().max(1);
        }
        // One step for every 1024 characters compared, as for text built.
        if compared.get() >= 1024 {
            let spent = compared.replace(0) / 1024;
            if !scope.spend(spent as u32) {
                break;
            }
        }
    }
    out
}

/// The 1-based character position of the `nth` match, or 0.
fn position(text: &str, needle: &str, nth: usize) -> usize {
    if needle.is_empty() || nth == 0 {
        return 0;
    }
    text.match_indices(needle)
        .nth(nth - 1)
        .map(|(at, _)| text[..at].chars().count() + 1)
        .unwrap_or(0)
}

/// A token separator given as a character code, such as `32` for a space.
fn token_char(code: &str) -> char {
    code.trim()
        .parse::<u32>()
        .ok()
        .and_then(char::from_u32)
        .unwrap_or(' ')
}

fn tokens(text: &str, sep: char) -> Vec<&str> {
    text.split(sep).filter(|item| !item.is_empty()).collect()
}

/// Token `which` of `text`: `2`, `-1` (counting from the end), `2-`, or `2-3`.
fn gettok(text: &str, which: &str, sep: char) -> String {
    let items = tokens(text, sep);
    let count = items.len() as i64;
    let place = |number: i64| {
        if number < 0 {
            count + number + 1
        } else {
            number
        }
    };
    let (first, last) = match which.split_once('-').filter(|(start, _)| !start.is_empty()) {
        Some((start, "")) => (start.parse().unwrap_or(1), count),
        Some((start, end)) => (
            start.parse().unwrap_or(1),
            place(end.parse().unwrap_or(count)),
        ),
        None => {
            let number = place(which.parse().unwrap_or(0));
            (number, number)
        }
    };
    let first = place(first);
    if first < 1 || last < first {
        return if which == "0" {
            count.to_string()
        } else {
            String::new()
        };
    }
    items
        .iter()
        .skip((first - 1) as usize)
        .take((last - first + 1) as usize)
        .copied()
        .collect::<Vec<_>>()
        .join(&sep.to_string())
}

fn read_line(path: &str, line: usize) -> String {
    if path.trim().is_empty() {
        return String::new();
    }
    let full = if std::path::Path::new(path).is_absolute() {
        std::path::PathBuf::from(path)
    } else {
        crate::settings::config_dir().join(path)
    };
    // Only a plain file of a sensible size: never a pipe or device that
    // could keep the window waiting.
    let Ok(meta) = std::fs::metadata(&full) else {
        return String::new();
    };
    if !meta.is_file() || meta.len() > READ_LIMIT {
        return String::new();
    }
    let Ok(file) = std::fs::File::open(full) else {
        return String::new();
    };
    use std::io::{BufRead, Read};
    std::io::BufReader::new(file.take(READ_LIMIT))
        .lines()
        .nth(line.saturating_sub(1))
        .and_then(Result::ok)
        .map(|text| cut(text, VALUE_LIMIT))
        .unwrap_or_default()
}

/// A number from `low` to `high`, both included and in either order, made
/// from the random `seed`. The arithmetic is wide enough for the whole `i64`
/// range, whose 2^64 values do not fit a `u64` count.
fn rand_between(low: i64, high: i64, seed: u64) -> i64 {
    let (low, high) = (low.min(high), low.max(high));
    let span = i128::from(high) - i128::from(low) + 1;
    let offset = i128::from(seed) % span;
    // Between low and high, so it fits an i64.
    (i128::from(low) + offset) as i64
}

/// A small random number from the clock and a counter; good enough for
/// scripts, never for anything secret.
fn random() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static STATE: AtomicU64 = AtomicU64::new(0);
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(1);
    let mut x = STATE.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed) ^ seed;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

/// A number without a trailing `.0`, and at most six decimals.
fn number(value: f64) -> String {
    if !value.is_finite() {
        return "0".into();
    }
    if value.fract() == 0.0 && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    let text = format!("{value:.6}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Arithmetic with `+ - * / % ^` and parentheses. Text that is not a sum
/// comes back as 0.
fn calc(expr: &str) -> String {
    let tokens: Vec<char> = expr.chars().filter(|ch| !ch.is_whitespace()).collect();
    let mut at = 0;
    let value = calc_sum(&tokens, &mut at);
    if at == tokens.len() {
        number(value)
    } else {
        "0".into()
    }
}

fn calc_sum(tokens: &[char], at: &mut usize) -> f64 {
    let mut value = calc_product(tokens, at);
    while let Some(&op) = tokens.get(*at) {
        if op != '+' && op != '-' {
            break;
        }
        *at += 1;
        let next = calc_product(tokens, at);
        value = if op == '+' {
            value + next
        } else {
            value - next
        };
    }
    value
}

fn calc_product(tokens: &[char], at: &mut usize) -> f64 {
    let mut value = calc_power(tokens, at);
    while let Some(&op) = tokens.get(*at) {
        if !matches!(op, '*' | '/' | '%') {
            break;
        }
        *at += 1;
        let next = calc_power(tokens, at);
        value = match op {
            '*' => value * next,
            '/' if next != 0.0 => value / next,
            '%' if next != 0.0 => value % next,
            _ => 0.0,
        };
    }
    value
}

fn calc_power(tokens: &[char], at: &mut usize) -> f64 {
    let base = calc_unary(tokens, at);
    if tokens.get(*at) == Some(&'^') {
        *at += 1;
        return base.powf(calc_power(tokens, at));
    }
    base
}

fn calc_unary(tokens: &[char], at: &mut usize) -> f64 {
    match tokens.get(*at) {
        Some('-') => {
            *at += 1;
            -calc_unary(tokens, at)
        }
        Some('+') => {
            *at += 1;
            calc_unary(tokens, at)
        }
        Some('(') => {
            *at += 1;
            let value = calc_sum(tokens, at);
            if tokens.get(*at) == Some(&')') {
                *at += 1;
            }
            value
        }
        _ => {
            let start = *at;
            while tokens
                .get(*at)
                .is_some_and(|ch| ch.is_ascii_digit() || *ch == '.')
            {
                *at += 1;
            }
            tokens[start..*at]
                .iter()
                .collect::<String>()
                .parse()
                .unwrap_or(0.0)
        }
    }
}

fn take_paren(chars: &[char], open: usize) -> (String, usize) {
    let mut depth = 0;
    let mut index = open;
    let start = open + 1;
    while index < chars.len() {
        if chars[index] == '(' {
            depth += 1;
        } else if chars[index] == ')' {
            depth -= 1;
            if depth == 0 {
                return (chars[start..index].iter().collect(), index + 1);
            }
        }
        index += 1;
    }
    (chars[start..].iter().collect(), chars.len())
}

fn split_args(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut depth = 0;
    for ch in input.chars() {
        if ch == '(' {
            depth += 1;
        } else if ch == ')' && depth > 0 {
            depth -= 1;
        }
        if ch == ',' && depth == 0 {
            args.push(current.trim().to_string());
            current.clear();
        } else {
            current.push(ch);
        }
    }
    if !current.trim().is_empty() || !args.is_empty() {
        args.push(current.trim().to_string());
    }
    args
}

fn builtin(line: &str, scope: &Scope, vars: &mut Vars) -> bool {
    let raw = line.trim().trim_start_matches('/');
    let mut parts = raw.split_whitespace();
    let Some(word) = parts.next() else {
        return false;
    };
    match word.to_ascii_lowercase().as_str() {
        "inc" | "dec" => {
            let Some(name) = parts.next() else {
                return true;
            };
            let amount = parts
                .next()
                .map(|text| calc(&eval(text, scope, vars)).parse::<f64>().unwrap_or(1.0))
                .unwrap_or(1.0);
            let current = var_value(vars, name.trim_start_matches('%'))
                .parse::<f64>()
                .unwrap_or(0.0);
            let next = if word.eq_ignore_ascii_case("dec") {
                current - amount
            } else {
                current + amount
            };
            set_var(vars, name, number(next));
            true
        }
        "set" => {
            // `set %name value` inside a script, like `%name = value`.
            let Some(name) = parts.next().filter(|name| name.starts_with('%')) else {
                return false;
            };
            let value = eval(&parts.collect::<Vec<_>>().join(" "), scope, vars);
            set_var(vars, name, value);
            true
        }
        "unset" => {
            for name in parts {
                let name = name.trim_start_matches('%').to_string();
                vars.retain(|(item, _)| item != &name);
            }
            true
        }
        "hadd" => {
            let Some(table) = parts.next() else {
                return true;
            };
            let Some(item) = parts.next() else {
                return true;
            };
            let rest = parts.collect::<Vec<_>>().join(" ");
            let value = eval(&rest, scope, vars);
            let key = hash_key(&eval(table, scope, vars), &eval(item, scope, vars));
            set_var(vars, &key, value);
            true
        }
        "hdel" => {
            let Some(table) = parts.next() else {
                return true;
            };
            let Some(item) = parts.next() else {
                return true;
            };
            let key = hash_key(&eval(table, scope, vars), &eval(item, scope, vars));
            vars.retain(|(name, _)| name != &key);
            true
        }
        "hfree" => {
            let Some(table) = parts.next() else {
                return true;
            };
            let prefix = format!("h/{}/", eval(table, scope, vars));
            vars.retain(|(name, _)| !name.starts_with(&prefix));
            true
        }
        _ => false,
    }
}

fn hash_key(table: &str, item: &str) -> String {
    format!("h/{}/{}", table.trim(), item.trim())
}

fn parse(source: &str) -> Result<Program, String> {
    let mut handlers = Vec::new();
    let mut aliases = Vec::new();
    let mut rest = source;
    loop {
        rest = skip_gap(rest);
        if rest.is_empty() {
            break;
        }
        let lower = rest.to_ascii_lowercase();
        if lower.starts_with("on ") || lower.starts_with("on\t") {
            let (handler, next) = parse_on(rest)?;
            handlers.push(handler);
            rest = next;
            continue;
        }
        if lower.starts_with("alias ") || lower.starts_with("alias\t") {
            let (alias, next) = parse_alias(rest)?;
            aliases.push(alias);
            rest = next;
            continue;
        }
        return Err(format!(
            "line {}: a script line must start with on or alias",
            line_of(source, rest)
        ));
    }
    Ok(Program { handlers, aliases })
}

/// The 1-based line number where `rest` starts inside `source`.
fn line_of(source: &str, rest: &str) -> usize {
    let at = source.len() - rest.len();
    source[..at].matches('\n').count() + 1
}

fn skip_gap(source: &str) -> &str {
    let mut rest = source.trim_start();
    while rest.starts_with(';') {
        rest = rest
            .split_once('\n')
            .map(|(_, next)| next)
            .unwrap_or("")
            .trim_start();
    }
    rest
}

fn parse_on(source: &str) -> Result<(Handler, &str), String> {
    let (header, after) = source.split_once('{').ok_or("an on block needs {")?;
    let header = header.trim()[2..].trim().trim_end_matches(':');
    let parts: Vec<&str> = header.split(':').collect();
    let level = parts.first().copied().unwrap_or("*").trim();
    let level = if level.is_empty() { "*" } else { level }.to_string();
    let part = |index: usize| -> String {
        parts
            .get(index)
            .map(|part| part.trim())
            .filter(|part| !part.is_empty())
            .unwrap_or("*")
            .to_string()
    };
    let event = parts
        .get(1)
        .copied()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let (text_mask, target_mask) = match event.as_str() {
        "text" | "action" | "notice" | "ctcp" => (part(2), part(3)),
        "join" | "part" | "kick" | "mode" | "dialog" => ("*".into(), part(2)),
        _ => ("*".into(), "*".into()),
    };
    let (body, rest) = parse_block(after)?;
    Ok((
        Handler {
            level,
            event,
            text_mask,
            target_mask,
            body,
        },
        rest,
    ))
}

fn parse_alias(source: &str) -> Result<(Alias, &str), String> {
    let (header, after) = source.split_once('{').ok_or("an alias needs {")?;
    let name = header.trim()[5..].trim().to_string();
    if name.is_empty() {
        return Err("alias needs a name".into());
    }
    let (body, rest) = parse_block(after)?;
    Ok((Alias { name, body }, rest))
}

fn starts_word(text: &str, word: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.starts_with(word)
        && text[word.len()..]
            .chars()
            .next()
            .is_none_or(|next| next.is_whitespace() || next == '(' || next == '|' || next == '}')
}

fn parse_block(source: &str) -> Result<(Vec<Stmt>, &str), String> {
    let mut stmts = Vec::new();
    let mut rest = source;
    loop {
        rest = skip_gap(rest);
        if rest.is_empty() {
            return Err("a block ran off the end (a } is missing)".into());
        }
        if let Some(after) = rest.strip_prefix('}') {
            return Ok((stmts, after.trim_start()));
        }
        if starts_word(rest, "while") {
            let (cond, body, next) = parse_head_and_block(rest, "while")?;
            stmts.push(Stmt::While { cond, body });
            rest = next;
            continue;
        }
        if starts_word(rest, "if") {
            let (stmt, next) = parse_if(rest)?;
            stmts.push(stmt);
            rest = next;
            continue;
        }
        if let Some(stripped) = rest.strip_prefix('%')
            && let Some((name, after_name)) = stripped.split_once('=')
            && !name.trim().contains(char::is_whitespace)
            && !after_name.starts_with('=')
        {
            let (value, next) = take_command(after_name.trim_start());
            stmts.push(Stmt::Assign {
                name: name.trim().to_string(),
                value,
            });
            rest = next;
            continue;
        }
        let (line, next) = take_command(rest);
        let bare = line.trim_start_matches('/');
        if starts_word(bare, "return") {
            stmts.push(Stmt::Return(bare[6..].trim().to_string()));
        } else if starts_word(bare, "halt") {
            stmts.push(Stmt::Halt);
        } else if !line.is_empty() {
            stmts.push(Stmt::Line(line));
        }
        rest = next;
    }
}

/// `word (condition) { block }`: the condition and the block.
fn parse_head_and_block<'a>(
    source: &'a str,
    word: &str,
) -> Result<(String, Vec<Stmt>, &'a str), String> {
    let after_word = source[word.len()..].trim_start();
    if !after_word.starts_with('(') {
        return Err(format!("{word} needs a condition in ( )"));
    }
    let chars: Vec<char> = after_word.chars().collect();
    let (cond, after_cond) = take_paren(&chars, 0);
    let bytes_after = after_word
        .char_indices()
        .nth(after_cond)
        .map(|(index, _)| index)
        .unwrap_or(after_word.len());
    let rest = after_word[bytes_after..].trim_start();
    let rest = rest
        .strip_prefix('{')
        .ok_or_else(|| format!("{word} needs {{"))?
        .trim_start();
    let (body, rest) = parse_block(rest)?;
    Ok((cond, body, rest))
}

fn parse_if(source: &str) -> Result<(Stmt, &str), String> {
    let (cond, then_body, after_then) = parse_head_and_block(source, "if")?;
    let mut else_body = Vec::new();
    let mut rest = skip_gap(after_then);
    if starts_word(rest, "elseif") {
        let (stmt, next) = parse_if(&rest[4..])?;
        else_body.push(stmt);
        rest = next;
    } else if starts_word(rest, "else") {
        let after_else = rest[4..].trim_start();
        if starts_word(after_else, "if") {
            let (stmt, next) = parse_if(after_else)?;
            else_body.push(stmt);
            rest = next;
        } else {
            let block = after_else
                .strip_prefix('{')
                .ok_or("else needs {")?
                .trim_start();
            let (body, next) = parse_block(block)?;
            else_body = body;
            rest = next;
        }
    }
    Ok((
        Stmt::If {
            cond,
            then_body,
            else_body,
        },
        rest,
    ))
}

fn take_command(source: &str) -> (String, &str) {
    let mut depth = 0;
    let mut end = 0;
    for (index, ch) in source.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' if depth == 0 => return (source[..index].trim().to_string(), &source[index..]),
            '}' => depth -= 1,
            '|' | '\n' if depth == 0 => {
                return (
                    source[..index].trim().to_string(),
                    source[index + 1..].trim_start(),
                );
            }
            _ => {}
        }
        end = index + ch.len_utf8();
    }
    (source[..end].trim().to_string(), "")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields<'a>(nick: &'a str, chan: &'a str, text: &'a str, args: &'a [&'a str]) -> Fields<'a> {
        Fields {
            nick,
            chan,
            me: "bob",
            text,
            time: "12:00",
            args,
            nicks: &[],
            levels: &[],
        }
    }

    #[test]
    fn text_event_and_if_send_commands() {
        let source = r#"
on *:TEXT:*hi*:#:{
  if ($nick == Ann) {
    notice $nick seen $1-
  } else {
    msg $chan hello
  }
}
"#;
        let mut vars = Vec::new();
        let words = ["oh", "hi"];
        let sent = run_event(
            source,
            "text",
            &fields("Ann", "#openirc", "oh hi", &words),
            &mut vars,
        )
        .unwrap();
        assert_eq!(sent, vec!["/notice Ann seen oh hi"]);
        let sent = run_event(
            source,
            "text",
            &fields("Bea", "#openirc", "oh hi", &words),
            &mut vars,
        )
        .unwrap();
        assert_eq!(sent, vec!["/msg #openirc hello"]);
    }

    #[test]
    fn alias_uses_arguments_calc_and_iif() {
        let source = r#"
alias greet {
  %last = $1
  msg $chan $iif($calc(1+2) == 3,hello,no) $1-
}
"#;
        let mut vars = Vec::new();
        let args = ["there", "friend"];
        let sent = run_alias(source, "greet", &fields("Ann", "#c", "", &args), &mut vars)
            .unwrap()
            .unwrap();
        assert_eq!(sent, vec!["/msg #c hello there friend"]);
        assert_eq!(vars, vec![("last".into(), "there".into())]);
    }

    #[test]
    fn while_hash_mid_read_and_nick() {
        let source = r#"
alias count {
  %n = 0
  while (%n < 3) {
    inc %n
    echo %n
  }
  hadd seen Ann yes
  echo $hget(seen,Ann)
  echo $mid(hello,2,3)
  echo $nick(#c,2)
}
on *:MODE:#:{
  echo mode $chan
}
on *:START:{
  echo started
}
"#;
        let mut vars = Vec::new();
        let nicks = ["Ann", "Bea"];
        let mut fields = fields("Ann", "#c", "", &[]);
        fields.nicks = &nicks;
        let sent = run_alias(source, "count", &fields, &mut vars)
            .unwrap()
            .unwrap();
        assert_eq!(
            sent,
            vec![
                "/echo 1",
                "/echo 2",
                "/echo 3",
                "/echo yes",
                "/echo ell",
                "/echo Bea"
            ]
        );
        let mode = run_event(source, "mode", &fields, &mut vars).unwrap();
        assert_eq!(mode, vec!["/echo mode #c"]);
        let start = run_event(source, "start", &fields, &mut vars).unwrap();
        assert_eq!(start, vec!["/echo started"]);
        let dir = std::env::temp_dir().join(format!("openirc-slang-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("read.txt");
        std::fs::write(&path, "one\ntwo\n").unwrap();
        let reader = format!("alias look {{\n  echo $read({},2)\n}}\n", path.display());
        let line = run_alias(&reader, "look", &fields, &mut vars)
            .unwrap()
            .unwrap();
        assert_eq!(line, vec!["/echo two"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn conditions_join_negate_and_compare() {
        let source = r#"
alias check {
  if (($1 > 2 && $2 == yes) || *.example iswm $3) {
    echo first
  }
  elseif (!$2) {
    echo second
  }
  else {
    echo third
  }
  if (an isin Ann && $1 >= 3 && $1 <= 3) {
    echo range
  }
}
"#;
        let mut vars = Vec::new();
        let run = |args: &[&str], vars: &mut Vec<(String, String)>| {
            run_alias(source, "check", &fields("Ann", "#c", "", args), vars)
                .unwrap()
                .unwrap()
        };
        assert_eq!(
            run(&["3", "yes", "x"], &mut vars),
            vec!["/echo first", "/echo range"]
        );
        assert_eq!(
            run(&["1", "no", "irc.example"], &mut vars),
            vec!["/echo first"]
        );
        assert_eq!(run(&["1"], &mut vars), vec!["/echo second"]);
        assert_eq!(run(&["1", "no", "x"], &mut vars), vec!["/echo third"]);
    }

    #[test]
    fn return_halt_and_aliases_as_identifiers() {
        let source = r#"
alias double {
  return $calc($1 * 2)
}
alias greet {
  echo $double(21) and $upper($left(hello,2)) $+ !
  halt
  echo never
}
on 10:TEXT:*:#:{
  echo trusted
  halt
}
on *:TEXT:*:#:{
  echo everyone
}
"#;
        let mut vars = Vec::new();
        let sent = run_alias(source, "greet", &fields("Ann", "#c", "", &[]), &mut vars)
            .unwrap()
            .unwrap();
        assert_eq!(sent, vec!["/echo 42 and HE!"]);
        let mut trusted = fields("Ann", "#c", "hi", &[]);
        trusted.levels = &["20"];
        assert_eq!(
            run_event(source, "text", &trusted, &mut vars).unwrap(),
            vec!["/echo trusted"],
            "halt stops the later handler"
        );
        assert_eq!(
            run_event(source, "text", &fields("Bea", "#c", "hi", &[]), &mut vars).unwrap(),
            vec!["/echo everyone"],
            "level 10 needs a level of 10 or more"
        );
    }

    #[test]
    fn text_and_token_identifiers() {
        let source = r#"
alias t {
  echo $right(abcdef,2) $left(abcdef,-2) $mid(abcdef,-3,2)
  echo $replace(Hello World,world,there,hello,hi) $remove(a-b-c,-)
  echo $pos(banana,an,2) $gettok(a.b.c.d,2-3,46) $gettok(a b c,-1,32) $numtok(a.b..c,46)
  echo $chr(65)$asc(B) $calc((2+3)*4/8) $calc(2^10) $round(3.14159,2) $istok(a b c,B,32)
  %price = 5
  echo costs $ %price at 50%
}
"#;
        let mut vars = Vec::new();
        let sent = run_alias(source, "t", &fields("Ann", "#c", "", &[]), &mut vars)
            .unwrap()
            .unwrap();
        assert_eq!(
            sent,
            vec![
                "/echo ef abcd de",
                "/echo hi there abc",
                "/echo 4 b.c c 3",
                "/echo A66 2.5 1024 3.14 $true",
                "/echo costs $ 5 at 50%",
            ]
        );
    }

    #[test]
    fn letters_beyond_ascii_never_break_conditions_or_replace() {
        let source = r#"
alias t {
  if (café == café && é != e) { echo same }
  echo $replace($1-,i,x) $replace(Straße,SSE,-) $replace(ÀB,à,a)
}
"#;
        let mut vars = Vec::new();
        let sent = run_alias(source, "t", &fields("Ann", "#c", "", &["İİK"]), &mut vars)
            .unwrap()
            .unwrap();
        assert_eq!(sent[0], "/echo same");
        // İ lowercases to two characters, so it is not a plain i.
        assert_eq!(sent[1], "/echo İİK Straße aB");
        with_scope(|scope| {
            assert_eq!(replace_nocase("ıIi", "i", "x", scope), "ıxx");
            assert_eq!(replace_nocase("aé", "É", "e", scope), "ae");
        });
    }

    fn with_scope<R>(act: impl FnOnce(&Scope) -> R) -> R {
        let program = parse("").unwrap();
        let steps = std::cell::Cell::new(STEP_LIMIT);
        let fields = fields("Ann", "#c", "", &[]);
        act(&Scope {
            fields: &fields,
            program: &program,
            depth: 0,
            steps: &steps,
        })
    }

    #[test]
    fn replacing_never_builds_more_than_one_value() {
        // Sizes chosen so that code without the limit builds a few MB and
        // fails the checks, never the huge text a real script could ask for.
        with_scope(|scope| {
            let long = "y".repeat(2_000);
            let out = replace_nocase(&"x".repeat(2_000), "x", &long, scope);
            assert_eq!(
                out.len(),
                VALUE_LIMIT,
                "4 MB would be built without the limit"
            );
            assert!(out.capacity() <= 2 * VALUE_LIMIT);
            // A cut never splits a letter.
            let out = replace_nocase(&"x".repeat(10_000), "x", "é", scope);
            assert!(out.len() <= VALUE_LIMIT && out.chars().all(|ch| ch == 'é'));
        });
        // The same through a script, in several stages, and kept values
        // hold no hidden memory past what is counted.
        let source = r#"
alias t {
  %r = $replace($str(x,2000),x,$str(y,2000),y,$str(z,8))
  echo $len(%r)
  %s = $remove($str(ab,8000),a) $+ $str(q,30000)
  echo $len(%s)
}
"#;
        let mut vars = Vec::new();
        let sent = run_alias(source, "t", &fields("Ann", "#c", "", &[]), &mut vars)
            .unwrap()
            .unwrap();
        assert_eq!(
            sent,
            vec![
                format!("/echo {VALUE_LIMIT}"),
                format!("/echo {VALUE_LIMIT}")
            ]
        );
        assert!(
            vars.iter()
                .all(|(_, value)| value.capacity() <= VALUE_LIMIT)
        );
    }

    #[test]
    fn random_numbers_cover_any_range_without_overflow() {
        let (min, max) = (i64::MIN, i64::MAX);
        for seed in [0, 1, 7, u64::MAX / 2, u64::MAX - 1, u64::MAX] {
            for (low, high) in [
                (min, max),
                (max, min),
                (5, 5),
                (-3, 3),
                (3, -3),
                (min, min),
                (max, max),
                (-1, 0),
            ] {
                let got = rand_between(low, high, seed);
                assert!(
                    got >= low.min(high) && got <= low.max(high),
                    "{low} {high} {seed}: {got}"
                );
            }
        }
        // The full range reaches both ends.
        assert_eq!(rand_between(min, max, 0), min);
        assert_eq!(rand_between(min, max, u64::MAX), max);
        // Through a script, with the reviewer's exact call.
        let source =
            "alias dice { echo $rand(-9223372036854775808,9223372036854775807) $rand(4,4) }";
        let sent = run_alias(
            source,
            "dice",
            &fields("Ann", "#c", "", &[]),
            &mut Vec::new(),
        )
        .unwrap()
        .unwrap();
        let words: Vec<&str> = sent[0].split_whitespace().collect();
        assert!(words[1].parse::<i64>().is_ok(), "{sent:?}");
        assert_eq!(words[2], "4");
    }

    #[test]
    fn comparing_long_text_uses_up_the_run() {
        // A pattern that nearly matches everywhere is slow to compare; the
        // work counts as steps, and the run stops when they are gone.
        let steps = with_scope(|scope| {
            let text = "a".repeat(VALUE_LIMIT);
            let from = format!("{}b", "a".repeat(VALUE_LIMIT / 2));
            let _ = replace_nocase(&text, &from, "", scope);
            scope.steps.get()
        });
        assert_eq!(steps, 0, "the run has no steps left");
    }

    #[test]
    fn nested_loops_and_growing_values_share_one_budget() {
        let source = r#"
alias t {
  %a = 0
  while ($true) {
    while ($true) {
      while ($true) { %a = %a $+ x }
    }
  }
  echo never
}
alias grow {
  %s = x
  while ($true) { %s = %s %s }
  echo $len(%s)
}
alias many {
  while ($true) { echo spam }
}
"#;
        let mut vars = Vec::new();
        let sent = run_alias(source, "t", &fields("Ann", "#c", "", &[]), &mut vars)
            .unwrap()
            .unwrap();
        assert_eq!(sent, vec![STOPPED.to_string()]);
        let mut vars = Vec::new();
        let sent = run_alias(source, "grow", &fields("Ann", "#c", "", &[]), &mut vars)
            .unwrap()
            .unwrap();
        assert!(vars.iter().all(|(_, value)| value.len() <= VALUE_LIMIT));
        assert_eq!(sent.len(), 1, "{sent:?}");
        let mut vars = Vec::new();
        let sent = run_alias(source, "many", &fields("Ann", "#c", "", &[]), &mut vars)
            .unwrap()
            .unwrap();
        assert_eq!(sent.len(), OUTPUT_LIMIT + 1);
        assert_eq!(sent.last().unwrap(), STOPPED);
    }

    #[test]
    fn read_only_reads_plain_files() {
        assert_eq!(read_line("/dev/zero", 1), "");
        assert_eq!(read_line("/", 1), "");
        let path = std::env::temp_dir().join(format!("openirc-read-{}.txt", std::process::id()));
        std::fs::write(&path, "one\ntwo\n").unwrap();
        assert_eq!(read_line(path.to_str().unwrap(), 2), "two");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn errors_name_the_line() {
        let err = check(
            "alias ok {
  echo hi
}

nonsense here
",
        )
        .unwrap_err();
        assert!(err.starts_with("line 5:"), "{err}");
        assert!(
            check(
                "alias broken {
 echo"
            )
            .unwrap_err()
            .contains("missing")
        );
    }
}
