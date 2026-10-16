//! The OpenIRC window, in the classic IRC client layout: menu, toolbar, switchbar,
//! tree of windows, topic, transcript, nick list, and an input line.
//!
//! The same eframe window runs on Windows, Linux, and macOS.

use crate::i18n::{tr, trf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use eframe::egui;

mod channel;
mod chrome;
mod commands;
mod icons;
pub(crate) mod input;
mod logs;
mod monitor;
mod prompts;
mod rules;
mod scripts;
mod search;
mod servers;
mod toast;
mod transfers;
mod view;

use crate::Args;
use crate::connection::{self, Connection};
use crate::dcc::{self, DccChat, DccOffer, PendingListen};
use crate::dcc_send::DccSendOffer;
use crate::ident::IdentServer;
use crate::pump::{Config, Effect, Event};
use crate::settings::{self, Alias, Favorite, Perform, SOUND_EVENTS, Settings};
use crate::theme::{self, Rgb, Theme};

const NICK_COLORS: [egui::Color32; 8] = [
    egui::Color32::from_rgb(0, 0, 128),
    egui::Color32::from_rgb(0, 128, 0),
    egui::Color32::from_rgb(128, 0, 0),
    egui::Color32::from_rgb(128, 0, 128),
    egui::Color32::from_rgb(128, 128, 0),
    egui::Color32::from_rgb(0, 128, 128),
    egui::Color32::from_rgb(0, 0, 180),
    egui::Color32::from_rgb(139, 69, 19),
];

pub fn run(prefill: Option<Args>) -> eframe::Result {
    let (settings, warnings) = Settings::load();
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([980.0, 640.0])
        .with_min_inner_size([720.0, 480.0])
        .with_title("OpenIRC")
        .with_app_id("openirc")
        .with_icon(app_icon());
    // Open where the window was last closed.
    if let Some([_, _, width, height]) = settings.window_place {
        viewport = viewport.with_inner_size([width.max(720) as f32, height.max(480) as f32]);
    }
    let start = start_position(&settings);
    if let Some(at) = start {
        viewport = viewport.with_position(at);
    }
    let centered = start.is_none();
    if settings.window_maximized {
        viewport = viewport.with_maximized(true);
    }
    let options = eframe::NativeOptions {
        viewport,
        centered,
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        "OpenIRC",
        options,
        Box::new(move |cc| {
            install_fonts(&cc.egui_ctx);
            Ok(Box::new(OpenIrcApp::with_settings(
                settings, &warnings, prefill,
            )))
        }),
    )
}

/// Where to ask the window manager to put the window at start: the saved
/// place, unless it is far off any screen (a monitor that is gone).
fn start_position(settings: &Settings) -> Option<egui::Pos2> {
    let [x, y, _, _] = settings.window_place?;
    let sane = -200..=16_000;
    (sane.contains(&x) && sane.contains(&y)).then(|| egui::pos2(x as f32, y as f32))
}

/// The window and taskbar icon. Window managers scale it as needed.
fn app_icon() -> egui::IconData {
    const SIZE: u32 = 128;
    egui::IconData {
        rgba: crate::icon_art::render(SIZE),
        width: SIZE,
        height: SIZE,
    }
}

/// A system font read in the background for a script, or `None` if there
/// is none on this computer.
struct FontLoad {
    script: crate::fonts::Script,
    loaded: Option<(std::path::PathBuf, u32, Vec<u8>)>,
}

/// Bumped whenever fonts change, so lines measured with the old ones are
/// measured again.
static FONT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn install_fonts(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions(&[]));
}

/// Bundled DejaVu faces (see assets/DejaVu-LICENSE.txt), then `extra`
/// system fonts for scripts DejaVu lacks. egui's own fonts stay as the last
/// fallbacks, for symbols and emoji.
fn font_definitions(extra: &[(String, Arc<egui::FontData>)]) -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    for (name, data) in extra {
        fonts.font_data.insert(name.clone(), data.clone());
    }
    let extra_names: Vec<String> = extra.iter().map(|(name, _)| name.clone()).collect();
    let faces: [(&str, &'static [u8]); 5] = [
        ("DejaVu", include_bytes!("../assets/DejaVuSans.ttf")),
        (
            "DejaVu Bold",
            include_bytes!("../assets/DejaVuSans-Bold.ttf"),
        ),
        (
            "DejaVu Oblique",
            include_bytes!("../assets/DejaVuSans-Oblique.ttf"),
        ),
        (
            "DejaVu Mono",
            include_bytes!("../assets/DejaVuSansMono.ttf"),
        ),
        (
            "DejaVu Mono Bold",
            include_bytes!("../assets/DejaVuSansMono-Bold.ttf"),
        ),
    ];
    for (name, bytes) in faces {
        fonts.font_data.insert(
            name.to_owned(),
            Arc::new(egui::FontData::from_static(bytes)),
        );
    }
    let fallbacks = fonts
        .families
        .get(&egui::FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let family = |first: &str| {
        let mut list = vec![first.to_owned()];
        list.extend(extra_names.iter().cloned());
        list.extend(fallbacks.iter().cloned());
        list
    };
    fonts
        .families
        .insert(egui::FontFamily::Proportional, family("DejaVu"));
    fonts
        .families
        .insert(egui::FontFamily::Name("bold".into()), family("DejaVu Bold"));
    fonts.families.insert(
        egui::FontFamily::Name("italic".into()),
        family("DejaVu Oblique"),
    );
    fonts.families.insert(
        egui::FontFamily::Name("mono-bold".into()),
        family("DejaVu Mono Bold"),
    );
    let mut mono = vec!["DejaVu Mono".to_owned()];
    mono.extend(extra_names.iter().cloned());
    mono.extend(
        fonts
            .families
            .get(&egui::FontFamily::Monospace)
            .cloned()
            .unwrap_or_default(),
    );
    fonts.families.insert(egui::FontFamily::Monospace, mono);
    fonts
}

fn paint(color: Rgb) -> egui::Color32 {
    egui::Color32::from_rgb(color.0, color.1, color.2)
}

fn color_face() -> egui::Color32 {
    paint(theme::current().face)
}

fn color_paper() -> egui::Color32 {
    paint(theme::current().paper)
}

fn color_ink() -> egui::Color32 {
    paint(theme::current().ink)
}

fn color_shadow() -> egui::Color32 {
    paint(theme::current().shadow)
}

fn color_hilite() -> egui::Color32 {
    paint(theme::current().hilite)
}

fn color_attention() -> egui::Color32 {
    paint(theme::current().attention)
}

fn mix(left: egui::Color32, right: egui::Color32, amount: f32) -> egui::Color32 {
    let blend = |a: u8, b: u8| (a as f32 * (1.0 - amount) + b as f32 * amount).round() as u8;
    egui::Color32::from_rgb(
        blend(left.r(), right.r()),
        blend(left.g(), right.g()),
        blend(left.b(), right.b()),
    )
}

/// Install the theme. Chrome (menus, buttons, labels) is drawn on Face, so
/// its text uses Ink only when Ink reads well there; otherwise black or
/// white. Text fields and the transcript are drawn in Ink on Paper.
/// Selected items use a text color picked for the Highlight color.
fn theme(ctx: &egui::Context, font_size: f32) {
    let face = color_face();
    let paper = color_paper();
    let chrome = text_on(face, color_ink());
    let mut visuals = if luminance(face) < 0.18 {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.panel_fill = face;
    visuals.window_fill = face;
    visuals.extreme_bg_color = paper;
    visuals.text_edit_bg_color = Some(paper);
    visuals.faint_bg_color = face;
    visuals.override_text_color = None;
    visuals.hyperlink_color = readable(egui::Color32::from_rgb(0, 0, 200), paper, color_ink());
    // Hint text in fields sits on Paper.
    visuals.weak_text_color = Some(muted_color());
    visuals.window_corner_radius = egui::CornerRadius::ZERO;
    visuals.menu_corner_radius = egui::CornerRadius::ZERO;
    visuals.window_shadow = egui::Shadow::NONE;
    visuals.popup_shadow = egui::Shadow::NONE;
    visuals.window_stroke = egui::Stroke::new(1.0, color_shadow());
    visuals.selection.bg_fill = color_hilite();
    visuals.selection.stroke =
        egui::Stroke::new(1.0, text_on(color_hilite(), egui::Color32::WHITE));
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::ZERO;
        widget.bg_stroke = egui::Stroke::new(1.0, color_shadow());
        widget.fg_stroke = egui::Stroke::new(1.0, chrome);
    }
    visuals.widgets.noninteractive.bg_fill = face;
    visuals.widgets.noninteractive.weak_bg_fill = face;
    visuals.widgets.inactive.bg_fill = face;
    visuals.widgets.inactive.weak_bg_fill = face;
    visuals.widgets.hovered.bg_fill = mix(face, chrome, 0.10);
    visuals.widgets.hovered.weak_bg_fill = mix(face, chrome, 0.10);
    visuals.widgets.active.bg_fill = mix(face, chrome, 0.20);
    visuals.widgets.active.weak_bg_fill = mix(face, chrome, 0.20);
    visuals.widgets.open.bg_fill = mix(face, chrome, 0.10);
    visuals.widgets.open.weak_bg_fill = mix(face, chrome, 0.10);
    ctx.set_visuals(visuals);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(4.0, 2.0);
        style.spacing.button_padding = egui::vec2(8.0, 2.0);
        style.spacing.window_margin = egui::Margin::same(4);
        style.spacing.menu_margin = egui::Margin::same(2);
        style.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::new(font_size, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::new((font_size - 1.0).max(9.0), egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::new(font_size + 4.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::new((font_size - 2.0).max(9.0), egui::FontFamily::Proportional),
        );
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Status,
    Channel,
    Query,
    List,
    Dcc,
}

/// Title of the window that collects private messages.
const MESSAGES: &str = "Messages";
/// Title of the window that shows raw lines and script events.
const DEBUG: &str = "Debug";

/// Lines kept per window. Older lines are dropped from the top.
const MAX_SCROLLBACK: usize = 5_000;

/// What a piece of transcript text is, not its color. Colors are resolved
/// while painting, so a theme change recolors every line already shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tone {
    Ink,
    Nick(u8),
    Palette(u8),
    Attention,
    Event(Ev),
}

/// Kinds of event line, each drawn in its own color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ev {
    Join,
    Part,
    Kick,
    Mode,
    Notice,
    Info,
}

/// A run of transcript text: what it is, how it is styled, and the text.
#[derive(Clone, Debug)]
struct Piece {
    tone: Tone,
    style: crate::colors::Style,
    background: Option<u8>,
    text: String,
}

impl Piece {
    fn new(tone: Tone, text: impl Into<String>) -> Self {
        Self {
            tone,
            style: crate::colors::Style::default(),
            background: None,
            text: text.into(),
        }
    }

    fn bold(mut self) -> Self {
        self.style.bold = true;
        self
    }
}

/// How much happened in a window since it was last looked at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
enum Activity {
    #[default]
    None,
    /// Joins, parts, modes, and other events.
    Event,
    /// Someone said something.
    Message,
    /// A highlight word or a private message.
    Highlight,
}

#[derive(Clone)]
struct RichLine {
    /// When the line arrived. Shown when timestamps are on, in the format
    /// chosen in Options.
    time: Option<crate::clock::LocalTime>,
    spans: Vec<Piece>,
    nick: Option<String>,
    /// Laid-out height and the layout key it was measured with.
    measured: std::cell::Cell<(u64, f32)>,
}

/// Something the user picked in the transcript, topic, or nick list. It is
/// collected while painting and run afterwards.
enum LineAction {
    Nick(&'static str, String),
    /// A double-click on a nick in the nick list.
    NickDouble(String),
    /// A double-click in the transcript of the active window.
    WindowDouble,
    /// Switch the active window between the fixed-width and normal font.
    ToggleMono,
    Select(String),
    Copy(String),
    Open(String),
    /// The word under the pointer when a line was right-clicked.
    MenuWord(Option<String>),
    /// Search the web for these words.
    Search(String),
    Popup {
        body: String,
        nick: String,
        text: String,
    },
}

/// Messages taken per frame from each server, and from all DCC chats
/// together (shared out among them, a different one first each frame).
const FRAME_MESSAGES: usize = 500;

/// Messages a server's thread may have waiting for the window. Past this
/// the thread waits for the window to catch up and stops reading from the
/// server meanwhile, so a flood is held back by the connection itself (the
/// server ends a client that falls too far behind) instead of growing in
/// memory. The window takes messages even while hidden, so this is reached
/// only when lines come faster than it can show them.
const INGRESS_LIMIT: usize = 5_000;

/// A DCC chat being connected to: the finished chat, or why it failed.
type ChatConnecting = Receiver<Result<DccChat, String>>;

/// One DCC chat connection.
struct DccLink {
    /// The window it belongs to (`ChatWindow::uid`).
    window: u64,
    /// This connection's own number: a timer set during one chat never
    /// speaks into a later one in the same window.
    id: u64,
    /// Lines to the peer; `None` once sending failed, while what the peer
    /// sent before it went is still read.
    outgoing: Option<mpsc::SyncSender<String>>,
    incoming: Receiver<String>,
    /// Hangs the chat up when dropped; dropped early when a new chat takes
    /// the window, so the old peer can send nothing more while what it sent
    /// is still shown.
    hangup: Option<dcc::Hangup>,
    /// A new chat waiting for the old one in its window to finish showing
    /// its lines (`DccLink::id` of the old one), and the line that opens it
    /// once it has.
    after: Option<(u64, String)>,
}

impl DccLink {
    /// Whether lines may go to this chat: it can send, and it has begun
    /// (its opening line is shown). Until then the window still shows the
    /// old chat, and a reply meant for that one must not reach this peer.
    fn usable(&self) -> bool {
        self.outgoing.is_some() && self.after.is_none()
    }
}

/// Who words typed in the window in front go to, where that can change
/// under the user: the DCC chat in a chat window, the person the Messages
/// window answers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Recipient {
    chat: Option<u64>,
    reply: Option<Reply>,
    /// The server `reply` is a nick on.
    server: Option<u64>,
}

/// The person a line in the Messages window was written to answer.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Reply {
    nick: String,
    /// They have quit since. Whoever takes the nick later is someone else,
    /// so nothing written for them goes there.
    left: bool,
}

impl Reply {
    fn to(nick: String) -> Self {
        Self { nick, left: false }
    }
}

/// What kind of words `speak` sends.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Speech {
    Plain,
    Say,
    Action,
}

/// Where words without a target of their own go (`destination`).
enum Destination {
    /// A DCC chat window, its connection's id and its way out.
    Dcc(usize, u64, mpsc::SyncSender<String>),
    /// The server's own target, which follows the window typed in.
    Server,
    /// The Messages window, with nobody to answer yet.
    NoReply(usize),
    /// The Messages window, and the person a paste or timer there was made
    /// for, who has quit since.
    Left(usize, String),
    /// The window it was meant for is closed.
    Gone,
    /// The window is open, but the DCC chat it was meant for has ended or
    /// been replaced.
    ChatOver,
    /// The window still shows an old chat's last lines before a new chat
    /// with the same name begins.
    ChatStarting(usize),
}

/// The latest a timer may wait for: a year. Later is refused rather than
/// computed, since a time past what the clock holds cannot be made.
const TIMER_LIMIT: Duration = Duration::from_secs(366 * 24 * 60 * 60);

/// `now` plus `wait`, if `wait` is within `TIMER_LIMIT`.
fn later(wait: Duration) -> Option<std::time::Instant> {
    (wait <= TIMER_LIMIT)
        .then(|| std::time::Instant::now().checked_add(wait))
        .flatten()
}

/// How deep aliases, scripts and the commands they run may call each other.
const MAX_DEPTH: u8 = 8;

/// Commands one typed line, event or timer may lead to in all, so aliases
/// that each run many commands cannot multiply without end.
const DISPATCH_BUDGET: u32 = 2_000;

/// Where a command line came from. Only typed lines open dialogs for bare
/// commands such as `/join`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Origin {
    Input,
    Script,
}

use crate::i18n::tr_key;

const NICK_ACTIONS: [(&str, &str); 12] = [
    (tr_key("Query"), "query"),
    (tr_key("Whois"), "whois"),
    (tr_key("Op"), "op"),
    (tr_key("Deop"), "deop"),
    (tr_key("Voice"), "voice"),
    (tr_key("Devoice"), "devoice"),
    (tr_key("Halfop"), "halfop"),
    (tr_key("Owner"), "owner"),
    (tr_key("Kick"), "kick"),
    (tr_key("Ban"), "ban"),
    (tr_key("Except"), "except"),
    (tr_key("Ignore"), "ignore"),
];

impl RichLine {
    fn plain(&self) -> String {
        self.spans.iter().map(|piece| piece.text.as_str()).collect()
    }

    /// The text with color and style codes for the pieces that have them,
    /// for logs that keep codes.
    fn coded(&self) -> String {
        let mut out = String::new();
        for piece in &self.spans {
            let mut open = String::new();
            for (on, code) in [
                (piece.style.bold, '\u{2}'),
                (piece.style.italic, '\u{1d}'),
                (piece.style.underline, '\u{1f}'),
                (piece.style.strike, '\u{1e}'),
                (piece.style.reverse, '\u{16}'),
            ] {
                if on {
                    open.push(code);
                }
            }
            let color = match piece.tone {
                Tone::Palette(color) => Some(color),
                _ => None,
            };
            if color.is_some() || piece.background.is_some() {
                open.push_str(&format!("\u{3}{:02}", color.unwrap_or(1)));
                if let Some(background) = piece.background {
                    open.push_str(&format!(",{background:02}"));
                }
            }
            out.push_str(&open);
            out.push_str(&piece.text);
            if !open.is_empty() {
                out.push('\u{f}');
            }
        }
        out
    }
}

/// Split `[YYYY-MM-DD HH:MM:SS] text` into the time and the text.
fn split_log_stamp(raw: &str) -> (Option<crate::clock::LocalTime>, &str) {
    let parsed = raw.strip_prefix('[').and_then(|rest| {
        let (stamp, text) = rest.split_once("] ")?;
        let (date, time) = stamp.split_once(' ')?;
        let mut date = date.splitn(3, '-').map(|part| part.parse::<i32>().ok());
        let mut time = time.splitn(3, ':').map(|part| part.parse::<i32>().ok());
        let local = crate::clock::LocalTime {
            year: date.next()??,
            month: date.next()??,
            day: date.next()??,
            hour: time.next()??,
            minute: time.next()??,
            second: time.next()??,
        };
        Some((local, text))
    });
    match parsed {
        Some((time, text)) => (Some(time), text),
        None => (None, raw),
    }
}

struct PassiveOut {
    token: String,
    path: std::path::PathBuf,
    filename: String,
    nick: String,
    /// The server the offer went out on; only a reply there counts.
    server: Option<u64>,
    made: std::time::Instant,
}

/// How long a file offered without listening waits for its answer.
const PASSIVE_WAIT: Duration = Duration::from_secs(30 * 60);

struct UiTimer {
    next: std::time::Instant,
    every: Duration,
    left: u32,
    command: String,
    /// The server the timer was set on. Its command runs there, whatever
    /// window is in front, and the timer ends when that server is closed.
    server: Option<u64>,
    /// The window the timer was set from (`ChatWindow::uid`): what it says
    /// goes there, never to whatever window is in front when it fires, and
    /// nowhere once that window is closed.
    window: Option<u64>,
    /// The history entry whose typed line set the timer.
    origin: Option<u64>,
    /// Set in a DCC chat: the connection then (`DccLink::id`).
    chat: Option<u64>,
    /// Set in the Messages window: the person it answered then.
    reply: Option<Reply>,
}

struct ChatWindow {
    /// This window's own number, never reused: a timer set here finds this
    /// window again, and not a later one that happens to share its name.
    uid: u64,
    kind: Kind,
    server: Option<u64>,
    title: String,
    topic: Option<String>,
    lines: Vec<RichLine>,
    nicks: Vec<String>,
    activity: Activity,
    /// Bumped whenever `lines` changes, so cached Find results know when
    /// they are stale.
    generation: u64,
    /// Index of the first line that arrived after the window was last left.
    marker: Option<usize>,
    /// Channel modes as last seen, such as `+nt` or `+ntk key`.
    modes: String,
    /// Ban, exception, and invite lists from the server, keyed by mode letter.
    mode_lists: std::collections::BTreeMap<char, Vec<ListEntry>>,
    /// Who is typing here, whether they paused, and when that was sent.
    typing: Vec<(String, bool, std::time::Instant)>,
}

/// One entry of a channel ban, exception, or invite list.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ListEntry {
    mask: String,
    by: String,
}

fn next_window_uid() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl ChatWindow {
    fn status() -> Self {
        Self {
            uid: next_window_uid(),
            kind: Kind::Status,
            server: None,
            title: "Status".into(),
            topic: None,
            lines: vec![
                note("OpenIRC"),
                note(tr("Not connected.")),
                note("Choose File, then Connect."),
            ],
            nicks: Vec::new(),
            activity: Activity::None,
            generation: 0,
            marker: None,
            modes: String::new(),
            mode_lists: std::collections::BTreeMap::new(),
            typing: Vec::new(),
        }
    }

    fn channel(title: &str) -> Self {
        Self {
            uid: next_window_uid(),
            kind: Kind::Channel,
            server: None,
            title: title.into(),
            topic: None,
            lines: Vec::new(),
            nicks: Vec::new(),
            activity: Activity::None,
            generation: 0,
            marker: None,
            modes: String::new(),
            mode_lists: std::collections::BTreeMap::new(),
            typing: Vec::new(),
        }
    }

    fn query(title: &str) -> Self {
        Self::plain(Kind::Query, title)
    }

    fn list(title: &str) -> Self {
        Self::plain(Kind::List, title)
    }

    fn dcc(title: &str) -> Self {
        Self::plain(Kind::Dcc, title)
    }

    fn plain(kind: Kind, title: &str) -> Self {
        Self {
            uid: next_window_uid(),
            kind,
            server: None,
            title: title.into(),
            topic: None,
            lines: Vec::new(),
            nicks: Vec::new(),
            activity: Activity::None,
            generation: 0,
            marker: None,
            modes: String::new(),
            mode_lists: std::collections::BTreeMap::new(),
            typing: Vec::new(),
        }
    }
}

struct Link {
    stop: Arc<AtomicBool>,
    /// Raw lines go to the Debug window while this is set.
    debug: Arc<AtomicBool>,
    outgoing: Sender<String>,
    incoming: Receiver<NetMsg>,
}

struct Session {
    id: u64,
    host: String,
    port: u16,
    tls: bool,
    nick: String,
    hangup: bool,
    reconnect_at: Option<std::time::Instant>,
    away: Option<String>,
    fingerprint: Option<String>,
    link: Option<Link>,
    /// Network name from the server, such as `Libera.Chat`.
    network: Option<String>,
    /// When registration finished, for the online timer.
    connected_at: Option<std::time::Instant>,
    /// Own lines waiting in the flood queue.
    queued: usize,
    /// Channels to join on connect, kept so a reconnect after a failed
    /// first attempt still joins them.
    autojoin: Vec<String>,
    /// The server settings this session uses (login, encoding, group).
    entry: crate::servers::ServerEntry,
    /// Failed connection attempts in a row.
    attempts: u32,
    /// IRCv3 capabilities in use.
    caps: Vec<String>,
    /// How this server compares nicks and channel names.
    casemap: crate::state::CaseMap,
    /// People known to be away, by lowercase nick, with their reason.
    away_nicks: std::collections::HashMap<String, String>,
    /// How many nick changes and quits this connection's server side had
    /// seen in the last of its news shown here (`Effect::renames`); lines
    /// sent to it carry this count.
    renames: u64,
    /// Services accounts, by lowercase nick.
    accounts: std::collections::HashMap<String, String>,
    /// Our own user modes on this server, such as `+iSx`.
    umodes: String,
    /// Notify nicks online on this server, as the server spells them.
    notify_online: Vec<String>,
}

enum NetMsg {
    Effect(Effect),
    /// How many own lines are waiting in the flood queue.
    Queued(usize),
    /// The server's certificate did not verify and is not trusted.
    Untrusted(crate::tls::UntrustedCert),
    Failed(String),
    Cert(String),
    /// Raw lines for the Debug window: true for lines from the server.
    Wire(Vec<(bool, String)>),
}

#[derive(Clone)]
struct ConnectForm {
    server: String,
    port: String,
    tls: bool,
    nick: String,
    channel: String,
}

/// The Options window edits this copy. OK and Apply copy it back; Cancel
/// and the close button drop it.
struct OptionsDraft {
    settings: Settings,
    picks: rules::RuleSelection,
    form: ConnectForm,
    sounds: Vec<String>,
    sound_note: String,
}

impl OptionsDraft {
    fn from(settings: &Settings, form: &ConnectForm) -> Self {
        Self {
            settings: settings.clone(),
            picks: rules::RuleSelection::default(),
            form: form.clone(),
            sounds: SOUND_EVENTS
                .iter()
                .map(|event| settings.event_sounds.path(event).to_string())
                .collect(),
            sound_note: String::new(),
        }
    }
}

struct OpenIrcApp {
    windows: Vec<ChatWindow>,
    active: usize,
    input: String,
    show_tree: bool,
    show_switchbar: bool,
    cert_note: String,
    form: ConnectForm,
    networks: Vec<crate::networks::Network>,
    network_note: String,
    network_rx: Option<Receiver<Result<Vec<crate::networks::Network>, String>>>,
    sessions: Vec<Session>,
    next_server: u64,
    routing: Option<u64>,
    nick: String,
    server_label: String,
    preview: bool,
    autoconnect: bool,
    settings: Settings,
    options_page: usize,
    show_favorites: bool,
    script_draft: Option<scripts::ScriptDraft>,
    show_address: bool,
    show_about: bool,
    show_dcc: bool,
    dcc_nick: String,
    favorite_name: String,
    alias_name: String,
    alias_body: String,
    notify_nick: String,
    find_text: String,
    perform_server: String,
    perform_command: String,
    options_draft: Option<OptionsDraft>,
    ident_server: Option<IdentServer>,
    ident_for: Option<String>,
    ident_failed: bool,
    remote_event: String,
    remote_mask: String,
    remote_body: String,
    popup_label: String,
    popup_body: String,
    logs: logs::LogBrowser,
    /// The Search all windows window.
    search: search::Search,
    /// The menu chosen for a new popup in the script editor.
    popup_menu: String,
    /// The Commands menu dialog that is open, if any.
    prompt: Option<prompts::Prompt>,
    /// Highlights, private messages, notices, and server messages from
    /// every window, for the monitor panel.
    monitor: monitor::Monitor,
    /// The Clear history window and what it will clear: typed lines, caught
    /// URLs, find text, channel list results, and the Messages window.
    clear_history: Option<[bool; 5]>,
    show_cert: bool,
    show_dialog_editor: bool,
    editing_dialog: String,
    dialog_script: String,
    open_dialog: Option<String>,
    new_dialog_name: String,
    dialog_note: String,
    dialog_dirty: bool,
    drag_rest: egui::Vec2,
    drag_origin: egui::Pos2,
    dialog_values: std::collections::HashMap<(String, u16), String>,
    tray: Option<crate::tray::Tray>,
    tray_noted: bool,
    /// The window is hidden in the tray.
    tray_hidden: bool,
    /// When the tray icon last blinked.
    tray_blinked: std::time::Instant,
    flash: bool,
    script_depth: u8,
    /// Commands left for the command or event being handled now, counting
    /// everything its aliases and scripts run in turn.
    dispatch_budget: u32,
    /// When writing a log last failed, so the failure is told once a minute.
    log_failed: Option<std::time::Instant>,
    /// Set while an event's commands share one budget.
    budget_held: bool,
    /// When other people opened query windows, for the query limit.
    query_opens: Vec<std::time::Instant>,
    /// Who last sent a private message that went to the Messages window;
    /// text typed there answers them.
    /// Who the Messages window of each server answers.
    last_private: std::collections::HashMap<Option<u64>, String>,
    /// `/exit` was typed; the window closes on the next frame.
    exit_requested: bool,
    /// `/dns` lookups still running.
    lookups: Vec<Receiver<String>>,
    /// When you last typed, for the automatic away.
    last_input: std::time::Instant,
    /// The automatic away is on (and should end when you type).
    auto_away_on: bool,
    /// Held while transfers run, so the computer does not sleep.
    awake: Option<crate::awake::Awake>,
    /// When the notify list was last checked on servers without MONITOR.
    notify_polled: std::time::Instant,
    /// The nick a file is being dragged over in the nick list.
    drop_nick: Option<String>,
    /// Server time of the lines being applied (IRCv3 server-time).
    line_time: Option<crate::clock::LocalTime>,
    /// Where a typing notice was last sent, and when.
    typing_sent: Option<(u64, String, std::time::Instant)>,
    script_fault: String,
    script_started: bool,
    timers: Vec<UiTimer>,
    dictionary: std::collections::HashSet<String>,
    dictionary_path: String,
    /// Words spell checking leaves alone in the current window (its nicks,
    /// its name, my nick), and what they were built from.
    spell_skip: std::collections::HashSet<String>,
    spell_skip_key: (usize, u64, String),
    /// The word right-clicked in the transcript, for the web search item.
    menu_word: Option<String>,
    /// Scripts a system font was looked for, the fonts found and loaded,
    /// and where the background loads report back.
    font_scripts: Vec<crate::fonts::Script>,
    extra_fonts: Vec<(String, Arc<egui::FontData>)>,
    extra_scripts: Vec<crate::fonts::Script>,
    /// The language setting last put in use.
    applied_language: Option<String>,
    font_tx: mpsc::Sender<FontLoad>,
    font_rx: mpsc::Receiver<FontLoad>,
    /// Where the window was asked to open, and how far its content landed
    /// from there (learned once the window manager has framed it).
    place_request: Option<egui::Pos2>,
    place_offset: Option<egui::Vec2>,
    /// The input-line word right-clicked: its byte range and suggestions.
    input_word: Option<(usize, usize, Vec<String>)>,
    offers: Vec<DccOffer>,
    /// Incoming file offers with the nick that sent each one.
    send_offers: Vec<transfers::QueuedOffer>,
    send_path: String,
    transfers: Vec<transfers::Transfer>,
    show_transfers: bool,
    /// An incoming file waiting for an answer.
    get_prompt: Option<transfers::GetPrompt>,
    passive_out: Vec<PassiveOut>,
    dcc_passive: bool,
    resume_wait: Option<transfers::ResumeWait>,
    /// The DCC chat connections, each tied to its window by the window's
    /// `uid` and known by an id of its own.
    dcc_links: Vec<DccLink>,
    /// Which connection `poll_dcc` reads first, turn by turn.
    dcc_turn: usize,
    /// A chat we offered, waiting for the other side: its nick and the
    /// server it was offered on.
    outgoing_dcc: Vec<(String, Option<u64>, PendingListen)>,
    /// A chat we accepted, connecting: its nick and the server of the offer.
    dcc_connecting: Vec<(String, Option<u64>, ChatConnecting)>,
    selected_nick: Option<String>,
    copy_request: Option<String>,
    script: crate::slang::Cache,
    profile: crate::profile::Profiler,
    show_profiler: bool,
    find_cache: Option<FindKey>,
    applied_style: Option<(Theme, u8)>,
    full_screen: bool,
    on_top: bool,
    history: input::History,
    completion: Option<input::Completion>,
    paste_pending: Option<Vec<String>>,
    /// The window (`ChatWindow::uid`) a timer's command belongs to while
    /// it runs.
    context: Option<u64>,
    /// For a timer set in a DCC chat, the connection (`DccLink::id`) it was
    /// set during.
    context_chat: Option<u64>,
    /// For a paste made in the Messages window, the person it answers.
    context_reply: Option<Reply>,
    /// The history entry whose typed line is running now, directly or
    /// through a timer it set.
    provenance: Option<u64>,
    /// The run the lines being dispatched now belong to: one typed line or
    /// paste, one timer firing, one popup or button. The server side reads
    /// which target a command in it chose on purpose (`/query alice |
    /// /say …`), and the rest of that run follows it.
    run: Option<u64>,
    /// The number the next run gets.
    next_run: u64,
    /// Whether a line of the current run went to the server choosing a
    /// target on purpose (`pump::picks_target`). Words of the run meant
    /// for a DCC chat, or for nobody yet, then go where that choice was.
    run_picks: bool,
    /// The history entry `enter_line` just added, for `submit`.
    typed_entry: Option<u64>,
    /// The history entry of a paste waiting for confirmation.
    paste_entry: Option<u64>,
    /// Where a paste waiting for confirmation was made: its window
    /// (`ChatWindow::uid`), who it was for then, and its server.
    paste_target: Option<(u64, Recipient, Option<u64>)>,
    /// While something is typed in the input line: who it is being typed
    /// for (`recipient`).
    draft_for: Option<Recipient>,
    /// History entries of timers that are due and being run now.
    running: Vec<u64>,
    /// Every history snapshot written, for tests.
    #[cfg(test)]
    history_saves: Vec<Vec<String>>,
    color_picker: bool,
    toasts: Vec<toast::Toast>,
    central: Option<channel::Central>,
    channel_list: channel::ChannelList,
    /// Window title last sent to the platform, to avoid resending it.
    shown_title: String,
    started_connect: bool,
    /// Last known `user@host` per server and nick, from joins.
    addresses: std::collections::HashMap<(Option<u64>, String), String>,
    /// A server certificate waiting for the user to trust it or not.
    trust_prompt: Option<(u64, crate::tls::UntrustedCert)>,
    /// The server list's selection and edit form.
    server_list: servers::ServerList,
}

/// Window index, title, line generation, and Find text, with the matching
/// line indices.
type FindKey = ((usize, String, u64, String), std::rc::Rc<[usize]>);

impl OpenIrcApp {
    #[cfg(feature = "bench")]
    fn new(prefill: Option<Args>) -> Self {
        let (settings, warnings) = Settings::load();
        Self::with_settings(settings, &warnings, prefill)
    }

    fn with_settings(
        mut settings: Settings,
        load_warnings: &[String],
        prefill: Option<Args>,
    ) -> Self {
        let mut windows = vec![ChatWindow::status()];
        let mut channel = ChatWindow::channel("#openirc");
        channel.topic = Some(tr("This is a preview of the channel window.").into());
        channel.nicks = vec!["@alice".into(), "+carol".into(), "bob".into()];
        channel.lines = vec![
            note("bob joined #openirc"),
            chat("alice", "hello", false, false),
            chat("alice", "waves", true, false),
            chat("bob", "hi https://example.com", false, false),
        ];
        windows.push(channel);
        for warning in load_warnings {
            windows[0].lines.push(note(warning.clone()));
        }
        if let Some(args) = &prefill {
            settings.server = args.host.clone();
            settings.port = args.port;
            settings.tls = args.tls;
            settings.nick = args.nick.clone();
            settings.user = args.user.clone();
            settings.realname = args.realname.clone();
            settings.autojoin = args.channels.first().cloned().unwrap_or_default();
        }
        let form = ConnectForm {
            server: settings.server.clone(),
            port: settings.port.to_string(),
            tls: settings.tls,
            nick: settings.nick.clone(),
            channel: settings.autojoin.clone(),
        };
        let nick = form.nick.clone();
        let mut history = input::History::default();
        history.restore(&settings.history);
        let (font_tx, font_rx) = mpsc::channel();
        // A window opened maximized tells nothing about its frame.
        let place_request = (!settings.window_maximized)
            .then(|| start_position(&settings))
            .flatten();
        Self {
            windows,
            active: 1,
            input: String::new(),
            show_tree: true,
            show_switchbar: true,
            cert_note: String::new(),
            form,
            networks: crate::networks::builtin(),
            network_note: String::new(),
            network_rx: None,
            sessions: Vec::new(),
            next_server: 1,
            routing: None,
            nick,
            server_label: "Preview — not connected".into(),
            preview: true,
            autoconnect: prefill.is_some(),
            settings,
            options_page: 0,
            show_favorites: false,
            script_draft: None,
            show_address: false,
            show_about: false,
            show_dcc: false,
            dcc_nick: String::new(),
            favorite_name: String::new(),
            alias_name: String::new(),
            alias_body: String::new(),
            notify_nick: String::new(),
            find_text: String::new(),
            perform_server: String::new(),
            perform_command: String::new(),
            options_draft: None,
            ident_server: None,
            ident_for: None,
            ident_failed: false,
            remote_event: "text".into(),
            remote_mask: "*".into(),
            remote_body: String::new(),
            popup_label: String::new(),
            popup_body: String::new(),
            logs: logs::LogBrowser::default(),
            prompt: None,
            popup_menu: "line".into(),
            search: search::Search::default(),
            monitor: monitor::Monitor::default(),
            clear_history: None,
            show_cert: false,
            show_dialog_editor: false,
            editing_dialog: String::new(),
            dialog_script: String::new(),
            open_dialog: None,
            new_dialog_name: String::new(),
            dialog_note: String::new(),
            dialog_dirty: false,
            drag_rest: egui::Vec2::ZERO,
            drag_origin: egui::Pos2::ZERO,
            dialog_values: std::collections::HashMap::new(),
            tray: None,
            tray_noted: false,
            tray_hidden: false,
            tray_blinked: std::time::Instant::now(),
            flash: false,
            script_depth: 0,
            dispatch_budget: 0,
            log_failed: None,
            budget_held: false,
            query_opens: Vec::new(),
            last_private: std::collections::HashMap::new(),
            drop_nick: None,
            exit_requested: false,
            lookups: Vec::new(),
            notify_polled: std::time::Instant::now(),
            awake: None,
            last_input: std::time::Instant::now(),
            auto_away_on: false,
            line_time: None,
            typing_sent: None,
            script_fault: String::new(),
            script_started: false,
            timers: Vec::new(),
            dictionary: std::collections::HashSet::new(),
            dictionary_path: String::new(),
            spell_skip: std::collections::HashSet::new(),
            spell_skip_key: (usize::MAX, 0, String::new()),
            menu_word: None,
            input_word: None,
            place_request,
            place_offset: None,
            font_scripts: Vec::new(),
            extra_fonts: Vec::new(),
            extra_scripts: Vec::new(),
            applied_language: None,
            font_tx,
            font_rx,
            offers: Vec::new(),
            send_offers: Vec::new(),
            send_path: String::new(),
            transfers: Vec::new(),
            show_transfers: false,
            get_prompt: None,
            passive_out: Vec::new(),
            dcc_passive: false,
            resume_wait: None,
            dcc_links: Vec::new(),
            dcc_turn: 0,
            outgoing_dcc: Vec::new(),
            dcc_connecting: Vec::new(),
            selected_nick: None,
            copy_request: None,
            script: crate::slang::Cache::default(),
            profile: crate::profile::Profiler::from_env(),
            show_profiler: false,
            find_cache: None,
            applied_style: None,
            full_screen: false,
            on_top: false,
            history,
            completion: None,
            paste_pending: None,
            context: None,
            context_chat: None,
            context_reply: None,
            provenance: None,
            typed_entry: None,
            run: None,
            next_run: 0,
            run_picks: false,
            paste_entry: None,
            paste_target: None,
            draft_for: None,
            running: Vec::new(),
            #[cfg(test)]
            history_saves: Vec::new(),
            color_picker: false,
            toasts: Vec::new(),
            central: None,
            channel_list: channel::ChannelList::default(),
            shown_title: String::new(),
            started_connect: false,
            trust_prompt: None,
            addresses: std::collections::HashMap::new(),
            server_list: servers::ServerList::default(),
        }
    }

    fn poll(&mut self) {
        let pending: Vec<(u64, Vec<NetMsg>)> = self
            .sessions
            .iter()
            .filter_map(|session| {
                let link = session.link.as_ref()?;
                let mut messages = Vec::new();
                // A fixed share per frame keeps the window responsive under a
                // flood; the rest waits for the next frame.
                while messages.len() < FRAME_MESSAGES {
                    match link.incoming.try_recv() {
                        Ok(message) => messages.push(message),
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            // The network thread ended without saying why.
                            messages.push(NetMsg::Failed("the connection thread stopped".into()));
                            break;
                        }
                    }
                }
                if messages.is_empty() {
                    None
                } else {
                    Some((session.id, messages))
                }
            })
            .collect();
        for (id, messages) in pending {
            for message in messages {
                match message {
                    NetMsg::Effect(effect) => self.apply_effect(id, effect),
                    NetMsg::Untrusted(cert) => {
                        self.on_server(Some(id), |app| {
                            app.note_status(trf("Not connected: {0}", &[&cert]));
                        });
                        if let Some(session) =
                            self.sessions.iter_mut().find(|session| session.id == id)
                        {
                            session.link = None;
                            session.reconnect_at = None;
                        }
                        self.trust_prompt = Some((id, cert));
                    }
                    NetMsg::Wire(lines) => {
                        self.on_server(Some(id), |app| {
                            let index = app.ensure(Kind::List, DEBUG);
                            for (from_server, line) in lines {
                                let arrow = if from_server { "<-" } else { "->" };
                                app.push(index, note(format!("{arrow} {line}")));
                            }
                        });
                    }
                    NetMsg::Queued(count) => {
                        if let Some(session) =
                            self.sessions.iter_mut().find(|session| session.id == id)
                        {
                            session.queued = count;
                        }
                    }
                    NetMsg::Failed(err) => {
                        if !self.session_connected(id) {
                            continue;
                        }
                        self.on_server(Some(id), |app| {
                            app.note_status(trf("Disconnected: {0}", &[&err]));
                        });
                        if let Some(session) =
                            self.sessions.iter_mut().find(|session| session.id == id)
                        {
                            session.link = None;
                        }
                        self.schedule_reconnect(id);
                    }
                    NetMsg::Cert(fingerprint) => {
                        if let Some(session) =
                            self.sessions.iter_mut().find(|session| session.id == id)
                        {
                            session.fingerprint = Some(fingerprint.clone());
                        }
                        self.on_server(Some(id), |app| {
                            app.note_status(format!("TLS certificate SHA-256 {fingerprint}"));
                        });
                    }
                }
            }
        }
    }

    fn apply_effect(&mut self, id: u64, effect: Effect) {
        self.drop_preview();
        // The count only grows within a connection: a line never claims to
        // have seen fewer changes than it did.
        if let Some(session) = self.sessions.iter_mut().find(|session| session.id == id) {
            session.renames = session.renames.max(effect.renames);
        }
        let previous = self.routing;
        self.routing = Some(id);
        let outer_case = casemap();
        set_casemap(self.casemap_of(Some(id)));
        self.line_time = effect.time.map(crate::clock::from_unix);
        if let Some((nick, address)) = effect.sender {
            // Remember who is speaking now, so a nick taken over by someone
            // else never inherits the old owner's address.
            let key = (Some(id), nick_key(&nick));
            if address.is_empty() {
                self.addresses.remove(&key);
            } else {
                self.addresses.insert(key, address);
            }
        }
        if effect.events.is_empty() {
            // Shown where the user is, unless that window belongs to another
            // server: then the server's own status window takes it.
            let elsewhere = self
                .windows
                .get(self.active)
                .is_some_and(|window| window.server.is_some_and(|server| server != id));
            for line in effect.lines {
                let line = line.trim_start_matches("* ").to_string();
                if elsewhere {
                    self.note_status(line);
                } else {
                    self.note_active(line);
                }
            }
        } else {
            for event in effect.events {
                self.apply_event(event);
            }
        }
        if effect.quit {
            if let Some(session) = self.sessions.iter_mut().find(|session| session.id == id) {
                session.link = None;
                session.away = None;
            }
            self.schedule_reconnect(id);
        }
        self.line_time = None;
        self.routing = previous;
        set_casemap(outer_case);
    }

    fn apply_event(&mut self, event: Event) {
        match event {
            Event::Registered { nick, text } => {
                // Addresses seen before a reconnect may belong to someone
                // else by now.
                let scope = self.routing;
                self.addresses.retain(|(server, _), _| *server != scope);
                self.nick = nick.clone();
                if let Some(id) = self.routing
                    && let Some(session) = self.sessions.iter_mut().find(|session| session.id == id)
                {
                    session.nick = nick;
                }
                if let Some(id) = self.routing
                    && let Some(session) = self.sessions.iter_mut().find(|session| session.id == id)
                {
                    session.connected_at = Some(std::time::Instant::now());
                    session.attempts = 0;
                }
                self.note_status(text.clone());
                self.server_label = text;
                if self.settings.dcc_ip_method == "server" {
                    let me = self.me().to_string();
                    self.send_user(format!("/raw USERHOST {me}"));
                }
                self.run_perform();
                if self.settings.channels_on_connect && !self.preview {
                    let server = self.routing;
                    self.show_channel_list(server, false);
                }
                let connected = self.me().to_string();
                self.fire_scripts("connect", &connected, "", "");
            }
            Event::Chat {
                channel,
                target,
                from,
                text,
                action,
            } => {
                let own = same_nick(&from, self.me());
                let (index, new_query) = if channel {
                    (self.ensure(Kind::Channel, &target), false)
                } else {
                    self.private_window(&target, own)
                };
                let rule = (!own)
                    .then(|| {
                        self.settings
                            .highlights
                            .iter()
                            .find(|rule| rule.matches(&text, &from, &target, self.me()))
                            .cloned()
                    })
                    .flatten();
                let mentions_me = self.settings.highlight_nick
                    && crate::highlight::first_match(&text, &[self.me().to_string()]).is_some();
                let hit = !own && (!channel || rule.is_some() || mentions_me);
                let mut line = chat_line(
                    &from,
                    &text,
                    action,
                    hit && channel,
                    self.settings.strip_codes,
                );
                line.spans[1].tone = self.nick_tone(index, &from);
                self.dress_nick(&mut line, index, &from, action);
                self.windows[index]
                    .typing
                    .retain(|(who, _, _)| !same_nick(who, &from));
                if self.settings.nick_color_line {
                    // Text without its own color takes the nick's color.
                    let tone = line.spans[1].tone;
                    for piece in line.spans.iter_mut().skip(3) {
                        if piece.tone == Tone::Ink {
                            piece.tone = tone;
                        }
                    }
                }
                if hit && let Some(color) = rule.as_ref().and_then(|rule| rule.color) {
                    for piece in line.spans.iter_mut().skip(3) {
                        piece.tone = Tone::Palette(color);
                    }
                }
                if !own && (hit || !channel) {
                    let tab = if channel {
                        monitor::MonitorTab::Highlights
                    } else {
                        monitor::MonitorTab::Private
                    };
                    let kind = if channel { Kind::Channel } else { Kind::Query };
                    let title = self.windows[index].title.clone();
                    let kind = if self.windows[index].kind == Kind::List {
                        Kind::List
                    } else {
                        kind
                    };
                    self.monitor_push(tab, line.clone(), Some((kind, title)));
                }
                self.push(index, line);
                if !own {
                    if hit {
                        match rule.as_ref() {
                            Some(rule) if !rule.sound.trim().is_empty() => {
                                if self.settings.beep || self.settings.sounds {
                                    crate::beep::play_file(rule.sound.trim());
                                }
                                self.flash |= rule.flash;
                            }
                            Some(rule) => {
                                let flash = self.flash;
                                self.cue("highlight");
                                self.flash = flash || rule.flash;
                            }
                            None => self.cue("highlight"),
                        }
                    } else if self.settings.sounds {
                        self.cue(if action { "action" } else { "message" });
                    }
                    self.mark(
                        index,
                        if hit {
                            Activity::Highlight
                        } else {
                            Activity::Message
                        },
                    );
                    if hit && index != self.active && rule.as_ref().is_none_or(|rule| rule.notice) {
                        let title = if channel {
                            trf("{0} in {1}", &[&from, &target])
                        } else {
                            trf("Message from {0}", &[&from])
                        };
                        self.tip(title, crate::colors::strip(&text), Some(index));
                    }
                    if new_query && self.settings.whois_on_query {
                        self.send_user(format!("/whois {from}"));
                    }
                    if new_query {
                        self.remember_windows();
                    }
                    let urls = crate::urls::find_urls(&text);
                    for url in urls {
                        let list = self.ensure(Kind::List, "URLs");
                        self.push(list, note(format!("{from}: {url}")));
                        self.mark(list, Activity::Event);
                    }
                    self.fire_scripts(
                        if action { "action" } else { "text" },
                        &from,
                        &target,
                        &text,
                    );
                }
            }
            Event::Sent {
                target,
                from,
                text,
                action,
            } => {
                let index = if target.starts_with(['#', '&', '+', '!']) {
                    self.ensure(Kind::Channel, &target)
                } else if self.settings.message_window && self.find(Kind::Query, &target).is_none()
                {
                    let index = self.ensure(Kind::List, MESSAGES);
                    let shown = if action {
                        format!("-> *{target}* * {from} {text}")
                    } else {
                        format!("-> *{target}* {text}")
                    };
                    self.push(index, note(shown));
                    return;
                } else {
                    self.ensure(Kind::Query, &target)
                };
                let mut line = chat(&from, &text, action, false);
                line.spans[1].tone = self.nick_tone(index, &from);
                self.dress_nick(&mut line, index, &from, action);
                self.push(index, line);
            }
            Event::Invite { from, channel } => {
                let status = self.event_window(self.settings.active_invites);
                if let Some(index) = status {
                    self.push(
                        index,
                        event_line(
                            Ev::Notice,
                            trf("{0} invites you to {1}", &[&from, &channel]),
                        ),
                    );
                    self.mark(index, Activity::Highlight);
                }
                if self.settings.join_on_invite {
                    self.send_user(format!("/join {channel}"));
                } else {
                    self.tip(
                        trf("Invite to {0}", &[&channel]),
                        trf("from {0} — type /join {1}", &[&from, &channel]),
                        status,
                    );
                }
            }
            Event::CaseMapping { map } => {
                if let Some(id) = self.routing
                    && let Some(session) = self.sessions.iter_mut().find(|session| session.id == id)
                {
                    session.casemap = map;
                }
                set_casemap(map);
            }
            Event::Network { name } => {
                if let Some(id) = self.routing
                    && let Some(session) = self.sessions.iter_mut().find(|session| session.id == id)
                {
                    session.network = Some(name);
                }
            }
            Event::Members { channel, names } => {
                if let Some(index) = self.find(Kind::Channel, &channel) {
                    let mut names = names;
                    sort_nicks(&mut names);
                    for name in &names {
                        self.want_fonts(name);
                    }
                    self.windows[index].nicks = names;
                }
            }
            Event::ModeChange {
                by,
                channel,
                changes,
            } => self.protect_on_mode(&by, &channel, &changes),
            Event::ChannelModes { channel, modes } => {
                if let Some(index) = self.find(Kind::Channel, &channel) {
                    self.windows[index].modes = modes;
                }
            }
            Event::Member { who, channel } => {
                let index = self.ensure(Kind::Channel, &channel);
                if !self.windows[index]
                    .nicks
                    .iter()
                    .any(|nick| same_nick(nick, &who))
                {
                    self.windows[index].nicks.push(who);
                    sort_nicks(&mut self.windows[index].nicks);
                }
            }
            Event::Join {
                who,
                channel,
                address,
            } => {
                if !address.is_empty() {
                    let key = (self.scope(), nick_key(&who));
                    self.addresses.insert(key, address.clone());
                }
                let index = self.ensure(Kind::Channel, &channel);
                if !self.windows[index]
                    .nicks
                    .iter()
                    .any(|nick| same_nick(nick, &who))
                {
                    self.windows[index].nicks.push(who.clone());
                    sort_nicks(&mut self.windows[index].nicks);
                }
                let mine = same_nick(&who, self.me());
                if self.settings.show_joins || mine {
                    let text = if mine {
                        trf("Now talking in {0}", &[&bold(&channel)])
                    } else if self.settings.short_joins {
                        format!("→ {who}")
                    } else if address.is_empty() {
                        trf("{0} joins {1}", &[&bold(&who), &channel])
                    } else {
                        trf("{0} ({1}) joins {2}", &[&bold(&who), &address, &channel])
                    };
                    self.push(index, event_line(Ev::Join, text));
                }
                if mine {
                    self.bring_forward(index);
                    self.remember_windows();
                } else {
                    self.auto_mode_on_join(index, &who, &address, &channel);
                    self.mark(index, Activity::Event);
                    self.cue("join");
                    self.fire_scripts("join", &who, &channel, "");
                }
            }
            Event::Part {
                who,
                channel,
                reason,
            } => {
                if let Some(index) = self.find(Kind::Channel, &channel) {
                    self.windows[index]
                        .nicks
                        .retain(|nick| !same_nick(nick, &who));
                    let mine = same_nick(&who, self.me());
                    let reason = reason.filter(|reason| !reason.is_empty());
                    let text = match (self.settings.short_joins && !mine, reason) {
                        (true, Some(reason)) => format!("← {who} ({reason})"),
                        (true, None) => format!("← {who}"),
                        (false, Some(reason)) => {
                            trf("{0} left {1} ({2})", &[&who, &channel, &reason])
                        }
                        (false, None) => trf("{0} left {1}", &[&who, &channel]),
                    };
                    if mine || self.settings.show_joins {
                        self.push(index, event_line(Ev::Part, text));
                        self.mark(index, Activity::Event);
                    }
                    if mine {
                        self.windows[index].nicks.clear();
                    } else {
                        self.cue("part");
                        self.fire_scripts("part", &who, &channel, "");
                    }
                }
            }
            Event::Kick {
                who,
                channel,
                by,
                reason,
            } => {
                if let Some(index) = self.find(Kind::Channel, &channel) {
                    self.windows[index]
                        .nicks
                        .retain(|nick| !same_nick(nick, &who));
                    let reason_text = reason.clone().unwrap_or_default();
                    let text = match &reason {
                        Some(reason) if !reason.is_empty() => trf(
                            "{0} kicked {1} from {2} ({3})",
                            &[&by, &who, &channel, &reason],
                        ),
                        _ => trf("{0} kicked {1} from {2}", &[&by, &who, &channel]),
                    };
                    self.push(index, event_line(Ev::Kick, text));
                    self.cue("kick");
                    self.fire_scripts("kick", &who, &channel, &reason_text);
                    if same_nick(&who, self.me()) {
                        self.windows[index].nicks.clear();
                        self.mark(index, Activity::Highlight);
                        self.tip(
                            trf("Kicked from {0}", &[&channel]),
                            trf("by {0}: {1}", &[&by, &reason_text]),
                            Some(index),
                        );
                        if self.settings.rejoin_on_kick {
                            self.send_user(format!("/join {channel}"));
                        }
                    } else {
                        self.mark(index, Activity::Event);
                    }
                }
            }
            Event::Quit { who, reason } => {
                let reason = reason.filter(|reason| !reason.is_empty());
                let text = match (self.settings.short_joins, reason) {
                    (true, Some(reason)) => trf("⇠ {0} quit ({1})", &[&who, &reason]),
                    (true, None) => trf("⇠ {0} quit", &[&who]),
                    (false, Some(reason)) => trf("{0} quit ({1})", &[&who, &reason]),
                    (false, None) => trf("{0} quit", &[&who]),
                };
                let scope = self.scope();
                self.addresses.remove(&(scope, nick_key(&who)));
                let shown: Vec<usize> = self
                    .windows
                    .iter()
                    .enumerate()
                    .filter(|(_, window)| {
                        window.server == scope
                            && ((window.kind == Kind::Channel
                                && window.nicks.iter().any(|nick| same_nick(nick, &who)))
                                || (window.kind == Kind::Query && same_nick(&window.title, &who)))
                    })
                    .map(|(index, _)| index)
                    .collect();
                for index in shown {
                    self.windows[index]
                        .nicks
                        .retain(|nick| !same_nick(nick, &who));
                    self.push(index, event_line(Ev::Part, text.clone()));
                    self.mark(index, Activity::Event);
                }
                self.note_status(text);
                if !same_nick(&who, self.me()) {
                    self.forget_nick(scope, &who);
                    self.cue("quit");
                }
            }
            Event::Nick { old, new } => {
                let mine = same_nick(&old, self.me());
                if mine {
                    self.nick = new.clone();
                    if let Some(id) = self.scope()
                        && let Some(session) =
                            self.sessions.iter_mut().find(|session| session.id == id)
                    {
                        session.nick = new.clone();
                    }
                }
                let text = trf("{0} is now {1}", &[&old, &new]);
                let scope = self.scope();
                self.follow_nick(scope, &old, &new);
                if let Some(address) = self.addresses.remove(&(scope, nick_key(&old))) {
                    self.addresses.insert((scope, nick_key(&new)), address);
                }
                let mut shown = Vec::new();
                for (index, window) in self.windows.iter_mut().enumerate() {
                    if window.server != scope {
                        continue;
                    }
                    let mut member = false;
                    for nick in &mut window.nicks {
                        if same_nick(nick, &old) {
                            let prefix: String = nick
                                .chars()
                                .take_while(|ch| matches!(ch, '@' | '+' | '%' | '&' | '~'))
                                .collect();
                            *nick = format!("{prefix}{new}");
                            member = true;
                        }
                    }
                    if member {
                        sort_nicks(&mut window.nicks);
                    }
                    let query = window.kind == Kind::Query && same_nick(&window.title, &old);
                    if query {
                        window.title = new.clone();
                    }
                    let show = match window.kind {
                        Kind::Channel => member,
                        Kind::Query => query,
                        Kind::Status => mine,
                        Kind::List | Kind::Dcc => false,
                    };
                    if show {
                        shown.push(index);
                    }
                }
                for index in shown {
                    self.push(index, note(text.clone()));
                }
            }
            Event::Topic { channel, topic } => {
                let index = self.ensure(Kind::Channel, &channel);
                let text = match &topic {
                    Some(topic) if !topic.is_empty() => trf("Topic is: {0}", &[&topic]),
                    _ => trf("{0} has no topic", &[&channel]),
                };
                self.windows[index].topic = topic;
                self.push(index, event_line(Ev::Mode, text));
                self.mark(index, Activity::Event);
            }
            Event::Names {
                channel,
                names,
                listed,
            } => {
                let index = self.ensure(Kind::Channel, &channel);
                // The list sent on join only fills the nick list; a line
                // sums it up. /names shows every name.
                let text = if listed {
                    let list = if names.is_empty() {
                        "empty".to_string()
                    } else {
                        names.join(" ")
                    };
                    format!("{channel} names: {list}")
                } else {
                    let ops = names
                        .iter()
                        .filter(|nick| nick.starts_with(['~', '&', '@']))
                        .count();
                    let voiced = names
                        .iter()
                        .filter(|nick| {
                            nick.chars()
                                .take_while(|ch| "~&@%+".contains(*ch))
                                .any(|ch| ch == '+' || ch == '%')
                        })
                        .count();
                    format!(
                        "{}: {}",
                        bold(&channel),
                        trf(
                            "{0} ({1}, {2} voiced)",
                            &[
                                &plural(names.len(), "{0} user", "{0} users"),
                                &plural(ops, "{0} op", "{0} ops"),
                                &voiced,
                            ],
                        )
                    )
                };
                let mut names = names;
                sort_nicks(&mut names);
                self.windows[index].nicks = names;
                self.push(index, event_line(Ev::Info, text));
            }
            Event::TopicChanged { by, channel, topic } => {
                let index = self.ensure(Kind::Channel, &channel);
                self.windows[index].topic = Some(topic.clone());
                self.push(
                    index,
                    event_line(
                        Ev::Mode,
                        trf("{0} changes topic to: {1}", &[&bold(&by), &topic]),
                    ),
                );
                self.mark(index, Activity::Event);
                self.fire_scripts("topic", &by, &channel, &topic);
            }
            Event::ChannelInfo { channel, text } => {
                let index = self.ensure(Kind::Channel, &channel);
                self.push(index, event_line(Ev::Mode, text));
            }
            Event::Notify {
                nick,
                address,
                online,
            } => {
                if let Some(session) = self.session_mut() {
                    session.notify_online.retain(|item| !same_nick(item, &nick));
                    if online {
                        session.notify_online.push(nick.clone());
                    }
                }
                let text = match (online, address.is_empty()) {
                    (true, true) => trf("{0} is online", &[&bold(&nick)]),
                    (true, false) => trf("{0} ({1}) is online", &[&bold(&nick), &address]),
                    (false, _) => trf("{0} has left IRC", &[&bold(&nick)]),
                };
                if let Some(index) = self.status_index() {
                    self.push(index, event_line(Ev::Notice, text));
                    self.mark(index, Activity::Event);
                }
                if online {
                    let target = self.find(Kind::Query, &nick);
                    self.tip(trf("{0} is online", &[&nick]), address, target);
                }
            }
            Event::UserModes { modes } => {
                if let Some(session) = self.session_mut() {
                    session.umodes = modes;
                }
            }
            Event::Notice { from, text } => {
                if from == "WALLOPS" || from.contains('.') {
                    let index = self.ensure(Kind::List, "Server notices");
                    let line = event_line(Ev::Notice, format!("{from}: {text}"));
                    self.monitor_push(
                        monitor::MonitorTab::Server,
                        line.clone(),
                        Some((Kind::List, "Server notices".into())),
                    );
                    self.push(index, line);
                    self.mark(index, Activity::Message);
                } else {
                    let status = self.event_window(self.settings.active_notices);
                    let line = event_line(Ev::Notice, format!("-{from}- {text}"));
                    self.monitor_push(monitor::MonitorTab::Notices, line.clone(), None);
                    match status {
                        Some(index) => {
                            self.push(index, line);
                            self.mark(index, Activity::Message);
                        }
                        None => self.note_status(format!("-{from}- {text}")),
                    }
                    if !from.eq_ignore_ascii_case("nickserv")
                        && !from.eq_ignore_ascii_case("chanserv")
                    {
                        self.tip(trf("Notice from {0}", &[&from]), text.clone(), status);
                    }
                }
                self.cue("notice");
                self.fire_scripts("notice", &from, "", &text);
            }
            Event::Ctcp {
                from,
                command,
                text,
            } => {
                let shown = if text.is_empty() || command == "PING" {
                    format!("[{from} {command}]")
                } else {
                    format!("[{from} {command}] {text}")
                };
                if let Some(index) = self.event_window(self.settings.active_ctcp) {
                    self.push(index, event_line(Ev::Notice, shown));
                    self.mark(index, Activity::Event);
                }
                let line = if text.is_empty() {
                    command
                } else {
                    format!("{command} {text}")
                };
                self.fire_scripts("ctcp", &from, "", &line);
            }
            Event::CtcpReply {
                from,
                command,
                text,
            } => {
                if let Some(index) = self.event_window(self.settings.active_ctcp) {
                    self.push(
                        index,
                        event_line(Ev::Notice, format!("[{from} {command} reply] {text}")),
                    );
                    self.mark(index, Activity::Event);
                }
            }
            Event::UserAway { who, reason } => {
                let key = nick_key(&who);
                let changed = match self.session_mut() {
                    Some(session) => match &reason {
                        Some(reason) => {
                            session.away_nicks.insert(key, reason.clone()).as_ref() != Some(reason)
                        }
                        None => session.away_nicks.remove(&key).is_some(),
                    },
                    None => false,
                };
                if changed && let Some(index) = self.find(Kind::Query, &who) {
                    let text = match &reason {
                        Some(reason) => trf("{0} is away: {1}", &[&who, &reason]),
                        None => trf("{0} is back", &[&who]),
                    };
                    self.push(index, event_line(Ev::Info, text));
                }
            }
            Event::Account { who, account } => {
                if let Some(session) = self.session_mut() {
                    let key = nick_key(&who);
                    match account {
                        Some(account) => {
                            session.accounts.insert(key, account);
                        }
                        None => {
                            session.accounts.remove(&key);
                        }
                    }
                }
            }
            Event::HostChange { who, address } => {
                let key = (self.scope(), nick_key(&who));
                self.addresses.insert(key, address);
            }
            Event::InviteNotify { by, who, channel } => {
                if let Some(index) = self.find(Kind::Channel, &channel) {
                    self.push(
                        index,
                        event_line(Ev::Notice, format!("{by} invited {who} to {channel}")),
                    );
                    self.mark(index, Activity::Event);
                }
            }
            Event::Typing { who, target, state } => {
                if !self.settings.show_typing {
                    return;
                }
                let kind = if target.starts_with(['#', '&', '+', '!']) {
                    Kind::Channel
                } else {
                    Kind::Query
                };
                if let Some(index) = self.find(kind, &target) {
                    let typing = &mut self.windows[index].typing;
                    typing.retain(|(nick, _, _)| !same_nick(nick, &who));
                    match state.as_str() {
                        "active" => typing.push((who, false, std::time::Instant::now())),
                        "paused" => typing.push((who, true, std::time::Instant::now())),
                        _ => {}
                    }
                }
            }
            Event::Sts { port, duration } => self.on_sts(port, duration),
            Event::Caps { enabled } => {
                if let Some(session) = self.session_mut() {
                    session.caps = enabled;
                }
            }
            Event::Numeric { code, text } => self.numeric(code, text),
            Event::DccChat { from, host, port } => {
                self.offers.push(DccOffer {
                    nick: from.clone(),
                    host: host.clone(),
                    port,
                    server: self.scope(),
                });
                self.note_status(trf(
                    "{0} offers DCC chat at {1}:{2}",
                    &[&from, &host, &port],
                ));
                self.tip(
                    trf("Chat request from {0}", &[&from]),
                    tr("Open Tools, DCC to accept."),
                    None,
                );
                self.show_dcc = true;
                self.cue("dcc");
            }
            Event::DccSend {
                from,
                filename,
                host,
                port,
                size,
                token,
            } => {
                // A reply to our passive offer counts only from the person we
                // offered it to, on the same server, with a port to reach.
                self.passive_out
                    .retain(|pending| pending.made.elapsed() < PASSIVE_WAIT);
                let server = self.routing;
                if let Some(token) = token.clone()
                    && port != 0
                    && let Some(index) = self.passive_out.iter().position(|pending| {
                        pending.token == token
                            && pending.server == server
                            && same_nick(&pending.nick, &from)
                    })
                {
                    let pending = self.passive_out.remove(index);
                    self.start_upload(&pending.nick, &pending.path, &host, port, &pending.filename);
                    return;
                }
                self.incoming_file(
                    from,
                    DccSendOffer {
                        filename,
                        host,
                        port,
                        size,
                        token,
                    },
                );
            }
            Event::DccAccept {
                from,
                filename,
                port,
                position,
            } => {
                self.resume_accepted(&from, &filename, port, position);
            }
            Event::DccResume {
                from,
                filename,
                port,
                position,
            } => self.resume_requested(&from, &filename, port, position),
            Event::Away { reason } => {
                if let Some(id) = self.routing
                    && let Some(session) = self.sessions.iter_mut().find(|session| session.id == id)
                {
                    session.away = reason.clone();
                }
                let text = match &reason {
                    Some(reason) => format!("away: {reason}"),
                    None => tr("You are back.").into(),
                };
                self.note_in(self.settings.active_away, text);
            }
            Event::Echo { text } => self.note_active(text),
            Event::Clear => {
                if let Some(window) = self.windows.get_mut(self.active) {
                    window.lines.clear();
                }
            }
            Event::Raw { command, text } => {
                if command.eq_ignore_ascii_case("MODE") {
                    let channel = text.split_whitespace().next().unwrap_or("");
                    let target = self.find(Kind::Channel, channel).unwrap_or(self.active);
                    if target < self.windows.len() {
                        self.push(target, event_line(Ev::Mode, format!("mode {text}")));
                        self.mark(target, Activity::Event);
                    }
                } else {
                    self.note_active(text.clone());
                }
                if command.eq_ignore_ascii_case("MODE") {
                    let channel = text.split_whitespace().next().unwrap_or("").to_string();
                    self.fire_scripts("mode", "", &channel, &text);
                }
            }
            Event::Ping => {
                if !self.settings.hide_ping {
                    self.note_status("PING? PONG!".into());
                }
            }
        }
    }

    fn connect(&mut self) {
        let port: u16 = match self.form.port.trim().parse() {
            Ok(port) if port > 0 => port,
            _ => {
                self.open_server_list();
                self.server_list.note = tr("Port must be a number from 1 to 65535.").into();
                return;
            }
        };
        let host = self.form.server.trim().to_string();
        let nick = self.form.nick.trim().to_string();
        if host.is_empty() || nick.is_empty() {
            self.open_server_list();
            self.server_list.note = tr("Server and nick are required.").into();
            return;
        }
        let channel = self.form.channel.trim().to_string();
        self.settings.server = host.clone();
        self.settings.port = port;
        self.settings.tls = self.form.tls;
        self.settings.nick = nick.clone();
        self.settings.autojoin = channel.clone();
        if self.settings.user.is_empty() {
            self.settings.user = nick.to_ascii_lowercase();
        }
        self.persist();
        // A saved server at this address brings its login, encoding, and
        // network group along.
        let mut entry = self
            .settings
            .servers
            .iter()
            .find(|entry| same_name(&entry.host, &host) && entry.port == port)
            .cloned()
            .unwrap_or_else(|| crate::servers::ServerEntry::new("", &host, port, self.form.tls));
        entry.tls = self.form.tls;
        if !channel.is_empty() {
            entry.channels = channel;
        }
        if entry.nick.trim().is_empty() {
            entry.nick = nick;
        }
        self.open_server(entry);
    }
    fn session_config(&self, entry: &crate::servers::ServerEntry, autojoin: Vec<String>) -> Config {
        let nick = if entry.nick.trim().is_empty() {
            self.settings.nick.clone()
        } else {
            entry.nick.trim().to_string()
        };
        let mut config = Config::new(
            nick,
            self.settings.user.clone(),
            self.settings.realname.clone(),
            autojoin,
            Vec::new(),
        );
        config.ignore_rules = self.settings.ignores.clone();
        config.flood_messages = self.settings.flood_messages;
        config.flood_ctcp = self.settings.flood_ctcp;
        config.flood_joins = self.settings.flood_joins;
        config.flood_window_ms = self
            .settings
            .flood_window_secs
            .saturating_mul(1_000)
            .max(1_000);
        if entry.login == crate::servers::Login::None {
            config.sasl_password = self.settings.sasl_password.clone();
            config.sasl_user = self.settings.sasl_user.clone();
        } else {
            config.login = entry.login;
            config.sasl_user = entry.login_user.clone();
            config.sasl_password = entry.login_pass.clone();
        }
        config.alt_nick = if entry.alt_nick.trim().is_empty() {
            self.settings.alt_nick.clone()
        } else {
            entry.alt_nick.clone()
        };
        config.server_password = entry.password.clone();
        config.encoding = entry.encoding.clone();
        config.quit_message = self.settings.quit_message.clone();
        config.finger_reply = self.settings.finger_reply.clone();
        config.send_queue = self.settings.send_queue;
        config.ircv3 = self.settings.ircv3;
        config.notify = self.settings.notify.clone();
        config
    }

    /// Timeout, client certificate, and trusted certificates for `host`.
    fn connect_options(&self, host: &str) -> connection::ConnectOptions {
        connection::ConnectOptions {
            tls: crate::tls::TlsOptions {
                client_cert: Some(self.settings.client_cert.clone())
                    .filter(|path| !path.trim().is_empty()),
                trusted: self
                    .settings
                    .trusted_certs
                    .iter()
                    .filter(|(trusted_host, _)| same_name(trusted_host, host))
                    .map(|(_, fingerprint)| fingerprint.clone())
                    .collect(),
            },
            timeout: Duration::from_secs(u64::from(self.settings.connect_timeout.clamp(5, 300))),
        }
    }
    fn proxy_choice(&self, host: &str) -> Option<crate::socks::Endpoint> {
        crate::net::proxy_for(host, false)
    }
    fn open_server(&mut self, entry: crate::servers::ServerEntry) {
        let id = self.next_server;
        self.next_server += 1;
        let mut autojoin = entry.channel_list();
        let (channels, queries) = self.windows_to_restore(&entry.host);
        for channel in channels {
            if !autojoin.iter().any(|item| same_name(item, &channel)) {
                autojoin.push(channel);
            }
        }
        let config = self.session_config(&entry, autojoin.clone());
        let nick = config.nick.clone();
        let proxy = self.proxy_choice(&entry.host);
        let via = proxy
            .as_ref()
            .map(|proxy| format!(" via {} {}:{}", proxy.kind, proxy.host, proxy.port))
            .unwrap_or_default();
        let (host, port, tls) = (entry.host.clone(), entry.port, entry.tls);
        let (port, tls) = self.sts_upgrade(&host, port, tls);
        let mut entry = entry;
        entry.port = port;
        entry.tls = tls;
        let options = self.connect_options(&host);
        let link = spawn_link(host.clone(), port, tls, config, proxy, options);
        self.drop_preview();
        let mut status = ChatWindow::status();
        status.server = Some(id);
        status.title = if entry.network.trim().is_empty() {
            host.clone()
        } else {
            entry.network.clone()
        };
        status.lines = vec![note(trf(
            "Connecting to {0}{1}",
            &[
                &format!("{host}:{port}{}", if tls { " (tls)" } else { "" }),
                &via,
            ],
        ))];
        self.windows.push(status);
        self.active = self.windows.len() - 1;
        self.nick = nick.clone();
        self.sessions.push(Session {
            id,
            host,
            port,
            tls,
            nick,
            hangup: false,
            reconnect_at: None,
            away: None,
            fingerprint: None,
            link: Some(link),
            network: Some(entry.network.clone()).filter(|network| !network.trim().is_empty()),
            connected_at: None,
            queued: 0,
            autojoin,
            entry,
            attempts: 0,
            caps: Vec::new(),
            casemap: crate::state::CaseMap::Rfc1459,
            away_nicks: std::collections::HashMap::new(),
            renames: 0,
            accounts: std::collections::HashMap::new(),
            umodes: String::new(),
            notify_online: Vec::new(),
        });
        // Queries open at the end of the last session come back too.
        self.on_server(Some(id), |app| {
            for nick in queries {
                app.ensure(Kind::Query, &nick);
            }
        });
    }
    /// Hang up the server being handled now: the one a timer, event or
    /// `/scon` runs on, else the one in front.
    fn disconnect(&mut self) {
        let Some(id) = self.scope() else {
            return;
        };
        if let Some(session) = self.sessions.iter_mut().find(|session| session.id == id) {
            session.hangup = true;
            session.reconnect_at = None;
            session.away = None;
            if let Some(link) = &session.link {
                // The network thread writes QUIT and then ends by itself.
                let _ = link.outgoing.send("/quit".into());
            }
        }
    }

    /// The input line was entered: keep it in the history and run it. The
    /// history entry is marked when what it ran sent a password.
    fn enter_line(&mut self) {
        // Typed for one DCC chat, or as an answer to one person, and another
        // is there now: this is not sent to someone it was not written for.
        // A second Enter sends it to the new one.
        if let Some(typed_for) = self.draft_for.take()
            && self.recipient() != typed_for
        {
            self.draft_for = Some(self.recipient());
            // The line itself is never shown: it may hold a password.
            let shown = match &typed_for.reply {
                Some(reply) if reply.left => trf(
                    "{0} left while you typed; press Enter again to send this to whoever this window answers now.",
                    &[&reply.nick],
                ),
                Some(_) => tr(
                    "The person this answers changed while you typed; press Enter again to send this to the new one.",
                )
                .into(),
                None => tr(
                    "The chat in this window changed while you typed; press Enter again to send this to the new one.",
                )
                .into(),
            };
            self.note_active(shown);
            return;
        }
        let line = self.input.clone();
        self.typed_entry = self.history.push(&line);
        self.submit();
    }

    /// The input line changed: start or end the draft, noting who it is
    /// typed for.
    fn note_draft(&mut self) {
        if self.input.trim().is_empty() {
            self.draft_for = None;
        } else if self.draft_for.is_none() {
            self.draft_for = Some(self.recipient());
        }
    }

    /// Who words typed in the window in front go to, where that can change
    /// by itself: its DCC chat, or the person the Messages window answers.
    fn recipient(&self) -> Recipient {
        let messages = self
            .windows
            .get(self.active)
            .filter(|window| window.kind == Kind::List && window.title == MESSAGES);
        Recipient {
            chat: self.active_chat(),
            reply: messages
                .and_then(|window| self.last_private.get(&window.server).cloned())
                .map(Reply::to),
            server: messages.and_then(|window| window.server),
        }
    }

    /// The target a line being run now belongs to: the channel or query of
    /// a timer's or paste's window or of the window typed in, or, in the
    /// Messages window, the person it answers. Lines sent to the server
    /// carry it with them (`/via`), so text and commands with no target of
    /// their own go there, even if the server side has moved its own target
    /// in between, on our own join, part, kick or nick change. An event's
    /// lines have none and keep the server side's.
    fn implicit_target(&self) -> Option<String> {
        let (index, deferred) = match self.context {
            Some(uid) => (self.window_by_uid(uid)?, true),
            None if self.routing.is_none() => (self.active, false),
            None => return None,
        };
        let window = self.windows.get(index)?;
        match window.kind {
            Kind::Channel | Kind::Query => Some(window.title.clone()),
            // Nobody, once the one it was made for has quit.
            Kind::List if window.title == MESSAGES && deferred => self
                .context_reply
                .as_ref()
                .filter(|reply| !reply.left)
                .map(|reply| reply.nick.clone()),
            Kind::List if window.title == MESSAGES => {
                self.last_private.get(&window.server).cloned()
            }
            _ => None,
        }
    }

    /// Run `body` as one run (see `run`), unless it is part of one already.
    fn in_run(&mut self, body: impl FnOnce(&mut Self)) {
        if self.run.is_some() {
            body(self);
            return;
        }
        self.next_run += 1;
        self.run = Some(self.next_run);
        self.run_picks = false;
        body(self);
        self.run = None;
        self.run_picks = false;
    }

    /// Run `body` as a run of its own, whatever runs now (a timer firing).
    fn new_run(&mut self, body: impl FnOnce(&mut Self)) {
        let outer = (self.run.take(), self.run_picks);
        self.in_run(body);
        (self.run, self.run_picks) = outer;
    }

    /// Someone on `server` is now `new`: whoever the Messages window, a
    /// draft, a waiting paste or a timer answers follows them, so text
    /// meant for them never goes to someone who takes the old nick. One
    /// who has left is not followed: a later change to that nick is
    /// someone else's.
    fn follow_nick(&mut self, server: Option<u64>, old: &str, new: &str) {
        let renamed = |nick: &mut String| {
            if same_nick(nick, old) {
                *nick = new.to_string();
            }
        };
        let answered = |reply: &mut Option<Reply>| {
            if let Some(reply) = reply
                && !reply.left
                && same_nick(&reply.nick, old)
            {
                reply.nick = new.to_string();
            }
        };
        if let Some(nick) = self.last_private.get_mut(&server) {
            renamed(nick);
        }
        // The typing notice in flight goes on to the same person.
        if let Some((id, nick, _)) = &mut self.typing_sent
            && Some(*id) == server
        {
            renamed(nick);
        }
        if let Some(draft) = &mut self.draft_for
            && draft.server == server
        {
            answered(&mut draft.reply);
        }
        if let Some((_, recipient, _)) = &mut self.paste_target
            && recipient.server == server
        {
            answered(&mut recipient.reply);
        }
        for timer in &mut self.timers {
            if timer.server == server {
                answered(&mut timer.reply);
            }
        }
    }

    /// Someone on `server` quit: whoever the Messages window answers is
    /// nobody now if it was them, a typing notice to them is over, and a
    /// draft, a waiting paste or a timer written to answer them is kept
    /// from whoever takes the nick next: it is refused when it would be
    /// sent, with a note.
    fn forget_nick(&mut self, server: Option<u64>, who: &str) {
        if self
            .last_private
            .get(&server)
            .is_some_and(|nick| same_nick(nick, who))
        {
            self.last_private.remove(&server);
        }
        if self
            .typing_sent
            .as_ref()
            .is_some_and(|(id, nick, _)| Some(*id) == server && same_nick(nick, who))
        {
            self.typing_sent = None;
        }
        let left = |reply: &mut Option<Reply>| {
            if let Some(reply) = reply
                && same_nick(&reply.nick, who)
            {
                reply.left = true;
            }
        };
        if let Some(draft) = &mut self.draft_for
            && draft.server == server
        {
            left(&mut draft.reply);
        }
        if let Some((_, recipient, _)) = &mut self.paste_target
            && recipient.server == server
        {
            left(&mut recipient.reply);
        }
        for timer in &mut self.timers {
            if timer.server == server {
                left(&mut timer.reply);
            }
        }
    }

    /// The usable DCC chat of the window in front, if it is one.
    fn active_chat(&self) -> Option<u64> {
        let window = self.windows.get(self.active)?;
        (window.kind == Kind::Dcc)
            .then(|| {
                self.dcc_links
                    .iter()
                    .find(|link| link.window == window.uid && link.usable())
                    .map(|link| link.id)
            })
            .flatten()
    }

    /// Bring a window forward for something that happened by itself (a
    /// chat connected, a join completed). Not while the user is typing or
    /// a paste waits: then the window is only marked, so their words stay
    /// where they were meant to go.
    fn bring_forward(&mut self, index: usize) {
        if self.input.trim().is_empty() && self.paste_pending.is_none() {
            self.activate(index);
        } else if index != self.active {
            self.mark(index, Activity::Message);
        }
    }

    /// Hold pasted `lines` for confirmation, noting where they were pasted.
    fn hold_paste(&mut self, lines: Vec<String>, entry: Option<u64>) {
        let recipient = self.recipient();
        self.paste_target = self
            .windows
            .get(self.active)
            .map(|window| (window.uid, recipient, window.server));
        self.paste_pending = Some(lines);
        self.paste_entry = entry;
    }

    /// The paste was confirmed: send it where it was pasted, whatever is in
    /// front now. A DCC chat that has ended or been replaced since gets
    /// nothing.
    fn send_paste(&mut self) {
        let lines = self.paste_pending.take().unwrap_or_default();
        let entry = self.paste_entry.take();
        match self.paste_target.take() {
            Some((window, recipient, server)) => {
                let outer = (self.context, self.context_chat, self.context_reply.take());
                self.context = Some(window);
                self.context_chat = recipient.chat;
                self.context_reply = recipient.reply;
                self.on_server(server, |app| app.run_typed(lines, entry));
                (self.context, self.context_chat, self.context_reply) = outer;
            }
            None => self.run_typed(lines, entry),
        }
    }

    /// Run typed lines on behalf of history entry `entry`: everything they
    /// start, at once or later through timers, counts for that entry
    /// (`watch_secret`).
    fn run_typed(&mut self, lines: Vec<String>, entry: Option<u64>) {
        let outer = std::mem::replace(&mut self.provenance, entry);
        self.new_run(|app| {
            for line in lines {
                app.dispatch(line, Origin::Input);
            }
        });
        self.provenance = outer;
    }

    fn submit(&mut self) {
        // The message itself ends the typing notice.
        self.typing_sent = None;
        self.user_active();
        let entry = self.typed_entry.take();
        let text = std::mem::take(&mut self.input);
        let lines: Vec<String> = text
            .lines()
            .map(str::to_string)
            .filter(|line| !line.trim().is_empty())
            .collect();
        let limit = self.settings.paste_confirm as usize;
        if lines.len() > 1 && limit > 0 && lines.len() >= limit {
            // Not run yet, so not known yet: the entry is kept out of saved
            // history until the paste is sent or dropped.
            self.hold_paste(lines, entry);
            return;
        }
        self.run_typed(lines, entry);
        // Saved only after the line ran, so it is never written out before
        // it is known whether it sent a password.
        if self.settings.keep_history && self.history.len().is_multiple_of(10) {
            self.save_history();
        }
    }

    /// Run one command line. Typed lines, aliases, script handlers, popups,
    /// timers, perform, and dialog buttons all come through here, so every
    /// one of them can use the same local commands (`/echo`, `/clear`,
    /// `/timer`, `/set`, `/dialog`, aliases) whether or not a server is
    /// connected. Anything that is not local goes to the server.
    fn dispatch(&mut self, line: String, origin: Origin) {
        if self.script_depth >= MAX_DEPTH {
            self.note_status(tr("Script: commands are nested too deeply; stopped.").into());
            return;
        }
        if self.script_depth == 0 && !self.budget_held {
            self.dispatch_budget = DISPATCH_BUDGET;
        }
        if self.dispatch_budget == 0 {
            return;
        }
        self.dispatch_budget -= 1;
        if self.dispatch_budget == 0 {
            self.note_status(
                tr("Script: too many commands at once; the rest were skipped.").into(),
            );
            return;
        }
        self.script_depth += 1;
        // A popup, button or anything else run on its own is a run of its
        // own; lines it runs belong to it.
        self.in_run(|app| app.dispatch_line(line, origin));
        self.script_depth -= 1;
    }

    fn dispatch_line(&mut self, line: String, origin: Origin) {
        let mut line = line.trim().to_string();
        if line.is_empty() {
            return;
        }
        let prefix = self.settings.command_prefix.trim();
        if origin == Origin::Input
            && !prefix.is_empty()
            && prefix != "/"
            && let Some(command) = line.strip_prefix(prefix)
            && !command.is_empty()
            && !command.starts_with(char::is_whitespace)
        {
            line = format!("/{command}");
        }
        // Every command is read before it runs, local ones such as
        // `/server -p` included, and before aliases turn it into others.
        self.watch_secret(&line);
        if let Some(parts) = self.settings.expand_alias(&line) {
            for part in parts {
                self.dispatch(part, origin);
            }
            return;
        }
        let Some(command) = line.strip_prefix('/') else {
            self.say(line);
            return;
        };
        // A typed `/query nick` opens that conversation and puts it in
        // front, so what is typed next goes there. Inside an alias, it leads
        // only the rest of the alias's run.
        if origin == Origin::Input
            && self.script_depth == 1
            && self.context.is_none()
            && let Some(nick) = crate::pump::query_nick(&line)
        {
            self.send_user(line);
            self.open_query(&nick);
            return;
        }
        let (name, rest) = command
            .split_once(char::is_whitespace)
            .map(|(name, rest)| (name, rest.trim()))
            .unwrap_or((command, ""));
        match name.to_ascii_lowercase().as_str() {
            "timer" => match crate::script::parse_timer(&line) {
                Some(spec) => {
                    // The command is read now, and again with all it leads to
                    // when it fires, for the typed line that set it.
                    self.watch_secret(&spec.command);
                    let window = match self.context {
                        Some(uid) => Some(uid),
                        None if self.routing.is_none() => {
                            self.windows.get(self.active).map(|window| window.uid)
                        }
                        None => None,
                    };
                    // In a DCC chat, the connection now: a later chat in
                    // the same window is someone else's.
                    let chat = match self.context {
                        Some(_) => self.context_chat,
                        None => window.and_then(|uid| {
                            self.dcc_links
                                .iter()
                                .find(|link| link.window == uid && link.usable())
                                .map(|link| link.id)
                        }),
                    };
                    // In the Messages window, the person it answers now.
                    let reply = match self.context {
                        Some(_) => self.context_reply.clone(),
                        None if self.routing.is_none() => self.recipient().reply,
                        None => None,
                    };
                    let Some(next) = later(Duration::from_secs(spec.seconds)) else {
                        self.note_active(tr("A timer can wait at most a year.").into());
                        return;
                    };
                    self.timers.push(UiTimer {
                        next,
                        every: Duration::from_secs(spec.seconds),
                        left: spec.repeats,
                        command: spec.command,
                        server: self.scope(),
                        window,
                        origin: self.provenance,
                        chat,
                        reply,
                    });
                    self.note_status(trf(
                        "Timer every {0}s, {1} times.",
                        &[&spec.seconds, &spec.repeats],
                    ));
                }
                None => self.note_active("usage: /timer <seconds> <repeats> <command>".into()),
            },
            "set" => match crate::script::parse_set(&line) {
                Some((name, value)) => {
                    match self
                        .settings
                        .vars
                        .iter_mut()
                        .find(|(item, _)| item == &name)
                    {
                        Some(slot) => slot.1 = value,
                        None => self.settings.vars.push((name, value)),
                    }
                    self.persist();
                }
                None => self.note_active("usage: /set <%var> <value>".into()),
            },
            "unset" => match crate::script::parse_unset(&line) {
                Some(name) => {
                    self.settings.vars.retain(|(item, _)| item != &name);
                    self.persist();
                }
                None => self.note_active("usage: /unset <%var>".into()),
            },
            "dialog" => {
                if self
                    .settings
                    .dialogs
                    .iter()
                    .any(|dialog| dialog.name == rest)
                {
                    self.open_dialog = Some(rest.to_string());
                } else {
                    self.note_status(trf("No dialog named {0}.", &[&rest]));
                }
            }
            "ignore" if !rest.is_empty() => {
                match crate::rules::parse_ignore(rest, crate::rules::unix_now()) {
                    Ok(rule) => {
                        let mask = rule.full_mask();
                        self.settings
                            .ignores
                            .retain(|item| item.full_mask() != mask);
                        self.settings.ignores.push(rule.clone());
                        self.persist();
                        self.note_active(trf(
                            "Ignoring {0}. Options, Ignore lists all rules.",
                            &[&mask],
                        ));
                        self.broadcast(ignore_command(&rule));
                    }
                    Err(err) => self.note_active(err),
                }
            }
            "unignore" if !rest.is_empty() => {
                let mask = crate::rules::full_mask(rest);
                self.settings
                    .ignores
                    .retain(|item| !item.full_mask().eq_ignore_ascii_case(&mask));
                self.persist();
                self.note_active(trf("No longer ignoring {0}.", &[&mask]));
                self.broadcast(format!("/unignore {mask}"));
            }
            "echo" => self.echo_command(rest),
            "debug" => {
                let Some(id) = self.scope() else {
                    self.note_active(
                        tr("Connect first; /debug shows one server's raw lines.").into(),
                    );
                    return;
                };
                let Some(link) = self
                    .sessions
                    .iter()
                    .find(|session| session.id == id)
                    .and_then(|session| session.link.as_ref())
                else {
                    self.note_active(tr("That server is not connected.").into());
                    return;
                };
                let on = match rest.to_ascii_lowercase().as_str() {
                    "on" => true,
                    "off" => false,
                    _ => !link.debug.load(Ordering::Relaxed),
                };
                link.debug.store(on, Ordering::Relaxed);
                // The rest of a timer or alias carries on with its own
                // server, so the routing comes back as it was.
                self.on_server(Some(id), |app| {
                    let index = app.ensure(Kind::List, DEBUG);
                    app.push(
                        index,
                        note(if on {
                            tr("Showing raw lines (-> sent, <- received) and script events; /debug off stops.")
                        } else {
                            tr("Stopped showing raw lines.")
                        }),
                    );
                    app.bring_forward(index);
                });
            }
            "channel" => {
                if !rest.is_empty()
                    && let Some(index) = self.find(Kind::Channel, rest)
                {
                    self.activate(index);
                }
                self.open_channel_central();
            }
            "dcc" => {
                let mut words = rest.splitn(3, char::is_whitespace);
                let kind = words.next().unwrap_or("").to_ascii_lowercase();
                let nick = words.next().unwrap_or("").trim().to_string();
                let path = words
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .to_string();
                match kind.as_str() {
                    "send" if !nick.is_empty() && !path.is_empty() => {
                        self.dcc_nick = nick;
                        self.send_path = path;
                        self.offer_send();
                    }
                    "chat" if !nick.is_empty() => {
                        self.dcc_nick = nick;
                        self.offer_chat();
                    }
                    _ => self
                        .note_active("usage: /dcc send <nick> <file> or /dcc chat <nick>".into()),
                }
            }
            "clear" | "cls" => {
                if let Some(window) = self.windows.get_mut(self.active) {
                    window.lines.clear();
                    window.generation += 1;
                }
            }
            "help" => {
                for help in crate::pump::help_lines() {
                    self.note_active(help.trim_start_matches("* ").to_string());
                }
                self.note_active(
                    "/echo <text>, /clear, /dialog <name>, /unset <%var> work offline too".into(),
                );
                self.note_active(
                    "/channel [#chan] opens Channel central; /dcc send <nick> <file>, /dcc chat <nick>; /debug [on|off] shows raw lines"
                        .into(),
                );
                for help in [
                    "/server [-m] [-e] [-p password] [-j #channels] host [[+]port]: -m adds a connection, + or -e uses TLS, no host reconnects",
                    "/disconnect, /exit, /scon -a <command> (on every connected server)",
                    "/list [filter], /url, /close [-m|window], /closemsg <nick>, /clearall, /savebuf [file]",
                    "/notify [-r] [nick], /alias [name [command]], /timers [off], /log on|off, /fullname, /anick",
                    "/auser <levels> <mask>, /ruser <mask>, /ulist, /load <file>, /unload <name>",
                    "/ban -u<seconds> [#channel] <nick>, /dns <host|nick>, /run <address>, /play <file>, /beep, /flash, /tip, /editbox",
                ] {
                    self.note_active(help.into());
                }
            }
            "join" | "j" if rest.is_empty() => {
                if origin == Origin::Input {
                    self.open_prompt(prompts::PromptKind::Join);
                } else {
                    self.note_active("usage: /join #channel".into());
                }
            }
            "me" if !rest.is_empty() => self.speak(rest, Speech::Action),
            "say" if !rest.is_empty() => self.speak(rest, Speech::Say),
            "quit" => {
                // The same server the QUIT goes to.
                if let Some(id) = self.scope()
                    && let Some(session) = self.sessions.iter_mut().find(|session| session.id == id)
                {
                    session.hangup = true;
                    session.reconnect_at = None;
                    session.away = None;
                }
                self.send_user(line);
            }
            _ => {
                if self.run_script_alias(name, rest) || self.local_command(name, rest) {
                    return;
                }
                self.send_user(line);
            }
        }
    }

    /// `/echo [-s] [window] text`: to the active window, the status window
    /// with `-s`, or a named channel or query window.
    fn echo_command(&mut self, rest: &str) {
        if let Some(text) = rest.strip_prefix("-s ") {
            self.note_status(text.trim().to_string());
            return;
        }
        if let Some((first, text)) = rest.split_once(' ')
            && let Some(index) = self
                .windows
                .iter()
                .position(|window| window.kind != Kind::Status && same_name(&window.title, first))
        {
            self.push(index, note(text.to_string()));
            return;
        }
        self.note_active(rest.to_string());
    }

    /// Plain text: to the DCC peer in a DCC window, otherwise to the server.
    fn say(&mut self, text: String) {
        self.speak(&text, Speech::Plain);
    }

    /// Send words without a target of their own (plain text, `/say`, `/me`)
    /// to where they belong: see `destination`.
    fn speak(&mut self, text: &str, speech: Speech) {
        let line = match speech {
            Speech::Action => format!("/me {text}"),
            _ => format!("/say {text}"),
        };
        match self.destination() {
            // A line of this run chose a target on purpose (`/query alice |
            // /say …`): the words go to that, never to the chat or window
            // the run started in, and nowhere if the choice did not take.
            Destination::Dcc(..) | Destination::NoReply(_) | Destination::Left(..)
                if self.run_picks =>
            {
                self.send_for(line, "-");
            }
            Destination::Dcc(index, link, sender) => {
                let (wire, shown) = match speech {
                    Speech::Action => (
                        format!("\u{1}ACTION {text}\u{1}"),
                        format!("{} {text}", self.me()),
                    ),
                    _ => (text.to_string(), format!("> {text}")),
                };
                let sent = sender.try_send(wire);
                if sent.is_ok() {
                    self.push(index, note(shown));
                } else if let Err(mpsc::TrySendError::Full(_)) = sent {
                    // The peer is not reading what was sent before.
                    self.note_status(trf(
                        "{0} is not keeping up, so a line was not sent.",
                        &[&self.windows[index].title],
                    ));
                } else {
                    // The chat's thread is gone. Nothing more can be sent,
                    // but what the peer sent before it went is still read,
                    // and the window says the chat ended once it has been.
                    if let Some(link) = self.dcc_links.iter_mut().find(|item| item.id == link) {
                        link.outgoing = None;
                    }
                    self.note_status(trf(
                        "The DCC chat with {0} has ended, so a line was not sent.",
                        &[&self.windows[index].title],
                    ));
                }
            }
            Destination::Server => self.send_user(match speech {
                Speech::Plain => text.to_string(),
                Speech::Say => format!("/say {text}"),
                Speech::Action => format!("/me {text}"),
            }),
            Destination::NoReply(index) => {
                self.push(
                    index,
                    note(tr("Nobody to answer yet; use /msg <nick> <text>.")),
                );
            }
            Destination::Left(index, nick) => {
                // The words themselves are never shown: they may hold a
                // password.
                self.push(
                    index,
                    note(trf("{0} left, so a line was not sent.", &[&nick])),
                );
            }
            Destination::Gone => {
                // The words themselves are never shown: they may hold a
                // password.
                self.note_status(
                    tr("The window this was set in is closed, so a line was not sent.").into(),
                );
            }
            Destination::ChatStarting(index) => {
                let shown = tr(
                    "The new chat begins once the old chat's last lines are shown, so a line was not sent.",
                );
                self.push(index, note(shown));
            }
            Destination::ChatOver => {
                self.note_status(
                    tr("The DCC chat this was meant for has ended, so a line was not sent.").into(),
                );
            }
        }
    }

    /// Where words without a target go, decided once for plain text, `/say`
    /// and `/me`:
    ///
    /// - typed: the window typed in. A channel or query there leaves it to
    ///   the server, whose target follows that window.
    /// - a timer: the window it was set in, by name, whatever is in front
    ///   now; nowhere at all once that window is closed.
    /// - an event, `/scon` or anything else bound to a server: that server's
    ///   own target, never the focused chat.
    fn destination(&self) -> Destination {
        if let Some(uid) = self.context {
            return match self.windows.iter().position(|window| window.uid == uid) {
                Some(index) => self.destination_of(index, true),
                None => Destination::Gone,
            };
        }
        if self.routing.is_some() || self.active >= self.windows.len() {
            return Destination::Server;
        }
        self.destination_of(self.active, false)
    }

    fn destination_of(&self, index: usize, deferred: bool) -> Destination {
        let window = &self.windows[index];
        match window.kind {
            // The window's live chat; for a timer, only the very chat it was
            // set during.
            Kind::Dcc => self
                .dcc_links
                .iter()
                .find(|link| {
                    link.window == window.uid
                        && link.usable()
                        && (!deferred || self.context_chat == Some(link.id))
                })
                .and_then(|link| {
                    let sender = link.outgoing.clone()?;
                    Some(Destination::Dcc(index, link.id, sender))
                })
                .unwrap_or_else(|| {
                    // Typed while a new chat waits for the old one's last
                    // lines: neither chat may take it.
                    let starting = self
                        .dcc_links
                        .iter()
                        .any(|link| link.window == window.uid && link.after.is_some());
                    if starting && !deferred {
                        Destination::ChatStarting(index)
                    } else {
                        Destination::ChatOver
                    }
                }),
            // The person answered, as `implicit_target` reads it (for a paste
            // or timer, the one it was made for), and nobody if there is
            // nobody.
            Kind::List if window.title == MESSAGES => {
                let reply = if deferred {
                    match &self.context_reply {
                        Some(reply) if reply.left => {
                            return Destination::Left(index, reply.nick.clone());
                        }
                        reply => reply.is_some(),
                    }
                } else {
                    self.last_private.contains_key(&window.server)
                };
                if reply {
                    Destination::Server
                } else {
                    Destination::NoReply(index)
                }
            }
            // A channel or query goes in the `/via` the line travels in,
            // where a target chosen earlier in the same run (`/query alice
            // | /say …`) can take its place.
            _ => Destination::Server,
        }
    }

    /// The window a script alias runs for, giving its `$chan`: the timer's
    /// window, the window typed in, or none for an event's alias.
    fn script_window(&self) -> Option<usize> {
        match self.context {
            Some(uid) => self.windows.iter().position(|window| window.uid == uid),
            None if self.routing.is_none() => {
                (self.active < self.windows.len()).then_some(self.active)
            }
            None => None,
        }
    }

    /// Run a script `alias` block named `name`. Returns false when there is
    /// no such alias.
    fn run_script_alias(&mut self, name: &str, rest: &str) -> bool {
        if self.settings.script_text.trim().is_empty() {
            return false;
        }
        let args: Vec<&str> = rest.split_whitespace().collect();
        let chan = self
            .script_window()
            .map(|index| &self.windows[index])
            .filter(|window| window.kind == Kind::Channel)
            .map(|window| window.title.clone())
            .unwrap_or_default();
        let nicks = self.channel_nicks(&chan);
        let nick_refs: Vec<&str> = nicks.iter().map(String::as_str).collect();
        let time = crate::clock::hm();
        let me = self.me().to_string();
        let fields = crate::slang::Fields {
            nick: &me,
            chan: &chan,
            me: &me,
            text: rest,
            time: &time,
            args: &args,
            nicks: &nick_refs,
            levels: &[],
        };
        let program = match self.script.program(&self.settings.script_text) {
            Ok(program) => program,
            Err(err) => {
                self.note_status(format!("Script: {err}"));
                return true;
            }
        };
        match crate::slang::run_alias_in(&program, name, &fields, &mut self.settings.vars) {
            Some(commands) => {
                for command in commands {
                    self.dispatch(command, Origin::Script);
                }
                true
            }
            None => false,
        }
    }

    fn channel_nicks(&self, chan: &str) -> Vec<String> {
        let scope = self.scope();
        self.windows
            .iter()
            .find(|window| {
                window.kind == Kind::Channel
                    && window.server == scope
                    && same_name(&window.title, chan)
            })
            .map(|window| window.nicks.iter().map(|nick| bare_nick(nick)).collect())
            .unwrap_or_default()
    }

    /// Save the settings and say so when that fails.
    fn persist(&mut self) {
        if let Err(err) = self.settings.save() {
            self.note_status(trf("Settings were not saved: {0}", &[&err]));
        }
    }

    /// Send a command line to the server of the current window (or the
    /// server an event came from).
    fn send_user(&mut self, line: String) {
        self.watch_secret(&line);
        // Run numbers are the window side's own: one written by hand could
        // make a later run follow a target chosen for it.
        if crate::pump::via_command(&line).is_some_and(|rest| rest.starts_with('=')) {
            self.note_status(tr("A /via target cannot start with “=”.").into());
            return;
        }
        let target = self.implicit_target().unwrap_or_else(|| "*".into());
        self.send_for(line, &target);
    }

    /// Send `line` to the server for `target` (`*`: the server side's own;
    /// `-`: only what this run chose), as a line of the current run. It
    /// travels as `/via =<run>:<renames> <target> <line>`: the server side
    /// reads a `/via` the line has of its own for the target, the run for a
    /// target chosen earlier in it, and the nick changes seen here for a
    /// nick that changed since.
    fn send_for(&mut self, line: String, target: &str) {
        let Some(id) = self.routing.or_else(|| self.active_server()) else {
            self.note_status(tr("Not connected. Choose File, then Connect.").into());
            return;
        };
        let Some((renames, outgoing)) = self
            .sessions
            .iter()
            .find(|session| session.id == id)
            .and_then(|session| Some((session.renames, session.link.as_ref()?.outgoing.clone())))
        else {
            self.note_status(tr("That server is not connected.").into());
            return;
        };
        if crate::pump::picks_target(&line) {
            self.run_picks = true;
        }
        let run = self.run.unwrap_or_else(|| {
            self.next_run += 1;
            self.next_run
        });
        let wrapped = format!("/via ={run}:{renames} {target} {line}");
        // Read with its target: words to a service may be a password.
        self.watch_secret(&wrapped);
        // Words for a target only the server side knows (its own, or what
        // the run chose) might be for a service: kept out of saved history.
        if matches!(target, "*" | "-") && input::is_speech(&line) {
            self.hold_back();
        }
        let _ = outgoing.send(wrapped);
    }

    fn schedule_reconnect(&mut self, id: u64) {
        let Some(session) = self.sessions.iter().find(|session| session.id == id) else {
            return;
        };
        if session.hangup || !self.settings.reconnect {
            self.note_status(tr("Disconnected.").into());
            return;
        }
        let attempts = session.attempts + 1;
        let limit = self.settings.retry_attempts;
        if limit > 0 && attempts > limit {
            self.note_status(trf(
                "Disconnected. Gave up after {0} tries; use File, Connect to try again.",
                &[&limit],
            ));
            if let Some(session) = self.sessions.iter_mut().find(|session| session.id == id) {
                session.reconnect_at = None;
            }
            return;
        }
        let next = if self.settings.retry_next {
            crate::servers::next_in_group(&self.settings.servers, &session.entry).cloned()
        } else {
            None
        };
        let delay = u64::from(self.settings.retry_delay.clamp(1, 600));
        let mut note_text = trf("Disconnected. Trying again in {0} seconds", &[&delay]);
        if let Some(next) = &next {
            note_text.push_str(&format!(" with {}:{}", next.host, next.port));
        }
        if limit > 0 {
            note_text.push_str(&format!(" (try {attempts} of {limit})"));
        }
        note_text.push('.');
        if let Some(session) = self.sessions.iter_mut().find(|session| session.id == id) {
            session.attempts = attempts;
            session.reconnect_at = Some(std::time::Instant::now() + Duration::from_secs(delay));
            if let Some(next) = next {
                session.host = next.host.clone();
                session.port = next.port;
                session.tls = next.tls;
                let channels = session.entry.channels.clone();
                session.entry = next;
                if session.entry.channels.trim().is_empty() {
                    session.entry.channels = channels;
                }
            }
        }
        self.note_status(note_text);
    }
    /// Open the conversation with `nick` on the current server and put it in
    /// front.
    fn open_query(&mut self, nick: &str) {
        let index = self.ensure(Kind::Query, nick);
        self.activate(index);
    }

    fn run_nick_action(&mut self, channel: &str, nick: &str, command: &str) {
        match command {
            "query" => self.open_query(nick),
            "whois" => self.send_user(format!("/whois {nick}")),
            "op" => self.send_user(format!("/mode {channel} +o {nick}")),
            "deop" => self.send_user(format!("/mode {channel} -o {nick}")),
            "voice" => self.send_user(format!("/mode {channel} +v {nick}")),
            "halfop" => self.send_user(format!("/halfop {channel} {nick}")),
            "owner" => self.send_user(format!("/owner {channel} {nick}")),
            "devoice" => self.send_user(format!("/mode {channel} -v {nick}")),
            "kick" => self.send_user(format!("/kick {channel} {nick}")),
            "ban" => self.send_user(format!("/ban {channel} {nick}")),
            "except" => self.send_user(format!("/except {channel} {nick}")),
            "ignore" => self.dispatch(format!("/ignore {nick}"), Origin::Script),
            _ => {}
        }
    }

    fn drop_preview(&mut self) {
        if !self.preview {
            return;
        }
        self.preview = false;
        self.windows.clear();
        self.active = 0;
    }

    fn ensure(&mut self, kind: Kind, title: &str) -> usize {
        if let Some(index) = self.find(kind, title) {
            return index;
        }
        let window = match kind {
            Kind::Channel => ChatWindow::channel(title),
            Kind::Query => ChatWindow::query(title),
            Kind::Status => ChatWindow::status(),
            Kind::List => ChatWindow::list(title),
            Kind::Dcc => ChatWindow::dcc(title),
        };
        let index = if matches!(kind, Kind::Query | Kind::List | Kind::Dcc) {
            self.windows.len()
        } else {
            self.windows
                .iter()
                .position(|window| matches!(window.kind, Kind::Query | Kind::List | Kind::Dcc))
                .unwrap_or(self.windows.len())
        };
        let had_active = self.active < self.windows.len();
        self.windows.insert(index, window);
        // The window the user is looking at stays in front.
        if had_active && index <= self.active {
            self.active += 1;
        }
        if let Some(server) = self.scope() {
            self.windows[index].server = Some(server);
        }
        if matches!(kind, Kind::Channel | Kind::Query) {
            self.reload_log(index);
        }
        if self.settings.window_order == "name" {
            self.sort_windows();
            return self.find(kind, title).unwrap_or(index);
        }
        index
    }

    /// With windows ordered by name: status windows first, then channels,
    /// queries, and the rest, each by name. The active window stays active.
    fn sort_windows(&mut self) {
        if self.settings.window_order != "name" || self.preview {
            return;
        }
        let rank = |kind: Kind| match kind {
            Kind::Status => 0,
            Kind::Channel => 1,
            Kind::Query => 2,
            Kind::List => 3,
            Kind::Dcc => 4,
        };
        let active = self
            .windows
            .get(self.active)
            .map(|window| (window.kind, window.title.clone(), window.server));
        self.windows.sort_by(|left, right| {
            rank(left.kind)
                .cmp(&rank(right.kind))
                .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
        });
        if let Some((kind, title, server)) = active {
            self.active = self
                .windows
                .iter()
                .position(|window| {
                    window.kind == kind && window.title == title && window.server == server
                })
                .unwrap_or(0);
        }
        self.find_cache = None;
    }

    fn find(&self, kind: Kind, title: &str) -> Option<usize> {
        let server = self.scope();
        let map = self.casemap_of(server);
        self.windows.iter().position(|window| {
            window.kind == kind && window.server == server && map.same(&window.title, title)
        })
    }

    fn run_perform(&mut self) {
        let Some(id) = self.routing else {
            return;
        };
        let host = self
            .sessions
            .iter()
            .find(|session| session.id == id)
            .map(|session| session.host.clone());
        let Some(host) = host else {
            return;
        };
        let commands: Vec<String> = self
            .settings
            .perform
            .iter()
            .filter(|item| item.server.is_empty() || same_name(&item.server, &host))
            .map(|item| item.command.clone())
            .collect();
        if commands.is_empty() {
            return;
        }
        self.in_run(|app| {
            for command in commands {
                app.dispatch(command, Origin::Script);
            }
        });
        self.note_status(trf("Ran perform for {0}.", &[&host]));
    }

    /// The user levels of `nick`: those of every user entry whose nick or
    /// mask matches, else the default level.
    fn user_levels(&self, nick: &str) -> Vec<String> {
        let address = self
            .addresses
            .get(&(self.scope(), nick_key(nick)))
            .cloned()
            .unwrap_or_else(|| "*@*".into());
        let full = format!("{nick}!{address}");
        let mut levels: Vec<String> = self
            .settings
            .users
            .iter()
            .filter(|(_, mask)| {
                let mask = mask.trim();
                if mask.contains(['!', '@']) {
                    crate::script::mask_match(&crate::rules::full_mask(mask), &full)
                } else {
                    crate::script::mask_match(mask, nick)
                }
            })
            .flat_map(|(levels, _)| levels.split(',').map(|level| level.trim().to_string()))
            .filter(|level| !level.is_empty())
            .collect();
        if levels.is_empty() {
            levels.push(self.settings.default_level.trim().to_string());
        }
        levels
    }

    fn fire_scripts(&mut self, event: &str, nick: &str, chan: &str, text: &str) {
        if self.script_depth > 3 {
            return;
        }
        let debugging = self.scope().is_some_and(|id| {
            self.sessions
                .iter()
                .find(|session| session.id == id)
                .and_then(|session| session.link.as_ref())
                .is_some_and(|link| link.debug.load(Ordering::Relaxed))
        });
        if debugging && let Some(index) = self.find(Kind::List, DEBUG) {
            let mut shown = format!("event {event}");
            // A mode change's text is `#channel +k key`: hide the key as the
            // raw lines above it do.
            let text = if event == "mode" {
                crate::connection::mask_secrets(&format!("MODE {text}"))
                    .trim_start_matches("MODE ")
                    .to_string()
            } else {
                text.to_string()
            };
            for (name, value) in [("nick", nick), ("chan", chan), ("text", text.as_str())] {
                if !value.is_empty() {
                    shown.push_str(&format!(" {name}={value}"));
                }
            }
            self.push(index, event_line(Ev::Info, shown));
        }
        let time = crate::clock::hm();
        let me = self.me().to_string();
        let words: Vec<&str> = text.split_whitespace().collect();
        let fields = crate::script::Fields {
            nick,
            chan,
            me: &me,
            text,
            time: &time,
        };
        let separator = self.settings.split_token();
        let mut commands = crate::script::commands(
            &self.settings.remotes,
            &self.settings.vars,
            event,
            &fields,
            &separator,
        );
        if !self.settings.script_text.trim().is_empty() {
            let nick_list = self.channel_nicks(chan);
            let nick_refs: Vec<&str> = nick_list.iter().map(String::as_str).collect();
            let levels = self.user_levels(nick);
            let level_refs: Vec<&str> = levels.iter().map(String::as_str).collect();
            let slang_fields = crate::slang::Fields {
                nick,
                chan,
                me: &me,
                text,
                time: &time,
                args: &words,
                nicks: &nick_refs,
                levels: &level_refs,
            };
            match self.script.program(&self.settings.script_text) {
                Ok(program) => commands.extend(crate::slang::run_event_in(
                    &program,
                    event,
                    &slang_fields,
                    &mut self.settings.vars,
                )),
                Err(err) => {
                    if self.script_fault != err {
                        self.script_fault = err.clone();
                        self.note_status(format!("Script: {err}"));
                    }
                }
            }
        }
        let held = self.budget_held;
        if self.script_depth == 0 && !held {
            self.dispatch_budget = DISPATCH_BUDGET;
            self.budget_held = true;
        }
        self.in_run(|app| {
            for command in commands {
                app.dispatch(command, Origin::Script);
            }
        });
        self.budget_held = held;
    }

    fn cue(&mut self, event: &str) {
        if event == "highlight" {
            self.flash = true;
        }
        let enabled = if event == "highlight" {
            self.settings.beep || self.settings.sounds
        } else {
            self.settings.sounds
        };
        if !enabled {
            return;
        }
        let path = self.settings.event_sounds.path(event);
        if path.is_empty() {
            crate::beep::play();
        } else {
            crate::beep::play_file(path);
        }
    }

    fn tick_timers(&mut self) {
        let now = std::time::Instant::now();
        let sessions: Vec<u64> = self.sessions.iter().map(|session| session.id).collect();
        // A timer set on a server that has since been closed ends quietly.
        self.timers
            .retain(|timer| timer.server.is_none_or(|id| sessions.contains(&id)));
        type Due = (
            Option<u64>,
            (Option<u64>, Option<u64>, Option<Reply>),
            Option<u64>,
            String,
        );
        let due: Vec<Due> = self
            .timers
            .iter_mut()
            .filter(|timer| timer.left > 0 && now >= timer.next)
            .map(|timer| {
                timer.left -= 1;
                // A next time past what the clock can hold ends the timer.
                match now.checked_add(timer.every) {
                    Some(next) => timer.next = next,
                    None => timer.left = 0,
                }
                (
                    timer.server,
                    (timer.window, timer.chat, timer.reply.clone()),
                    timer.origin,
                    timer.command.clone(),
                )
            })
            .collect();
        self.timers.retain(|timer| timer.left > 0);
        // Out of the timer list but not run yet: their lines stay out of
        // saved history until each has run, whatever an earlier one saves.
        let queued = self.running.len();
        self.running
            .extend(due.iter().filter_map(|(_, _, origin, _)| *origin));
        for (server, (window, chat, reply), origin, command) in due {
            let outer = std::mem::replace(&mut self.context, window);
            let outer_chat = std::mem::replace(&mut self.context_chat, chat);
            let outer_reply = std::mem::replace(&mut self.context_reply, reply);
            let outer_origin = std::mem::replace(&mut self.provenance, origin);
            self.on_server(server, |app| {
                app.new_run(|app| app.dispatch(command, Origin::Script));
            });
            self.context = outer;
            self.context_chat = outer_chat;
            self.context_reply = outer_reply;
            self.provenance = outer_origin;
            if let Some(origin) = origin
                && let Some(at) = self.running[queued..].iter().position(|id| *id == origin)
            {
                self.running.remove(queued + at);
            }
        }
    }

    /// Rebuild the words spell checking skips when the window or its nick
    /// list changes. Nothing is built without a dictionary.
    fn update_spell_skip(&mut self) {
        if self.dictionary.is_empty() {
            return;
        }
        let Some(window) = self.windows.get(self.active) else {
            return;
        };
        // The list's contents, not only its length: a rename keeps the count.
        let nicks = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            window.nicks.hash(&mut hasher);
            window.title.hash(&mut hasher);
            hasher.finish()
        };
        let key = (self.active, nicks, self.me().to_string());
        if key == self.spell_skip_key {
            return;
        }
        self.spell_skip = window
            .nicks
            .iter()
            .map(|nick| bare_nick(nick).to_lowercase())
            .chain([
                bare_nick(self.me()).to_lowercase(),
                window.title.to_lowercase(),
            ])
            .collect();
        self.spell_skip_key = key;
    }

    fn spell(&self) -> Spell<'_> {
        Spell {
            words: &self.dictionary,
            skip: &self.spell_skip,
        }
    }

    fn load_dictionary(&mut self) {
        if self.dictionary_path == self.settings.dictionary {
            return;
        }
        self.dictionary_path = self.settings.dictionary.clone();
        if self.dictionary_path.trim().is_empty() {
            self.dictionary.clear();
            return;
        }
        match crate::spell::read(std::path::Path::new(&self.dictionary_path)) {
            Ok(words) => self.dictionary = words,
            Err(err) => {
                self.dictionary.clear();
                self.note_status(trf(
                    "Spell checking is off: {0} could not be read ({1}).",
                    &[&self.dictionary_path, &err],
                ));
            }
        }
    }

    /// Look for a system font for any script in `text` that the bundled
    /// fonts lack, once per script, on a background thread.
    fn want_fonts(&mut self, text: &str) {
        let mut found = Vec::new();
        crate::fonts::scripts_in(text, &mut found);
        for script in found {
            if self.font_scripts.contains(&script) {
                continue;
            }
            self.font_scripts.push(script);
            let tx = self.font_tx.clone();
            std::thread::spawn(move || {
                let loaded = crate::fonts::find(script).and_then(|found| {
                    let bytes = std::fs::read(&found.path).ok()?;
                    Some((found.path, found.index, bytes))
                });
                let _ = tx.send(FontLoad { script, loaded });
            });
        }
    }

    /// Switch the menus and dialogs to the chosen language when it changes.
    fn apply_language(&mut self) {
        if self.applied_language.as_deref() == Some(self.settings.language.as_str()) {
            return;
        }
        self.applied_language = Some(self.settings.language.clone());
        if let Some(problem) = crate::i18n::set_language(&self.settings.language) {
            self.note_status(problem);
        }
    }

    /// Install fonts the background loads have read.
    fn poll_fonts(&mut self, ctx: &egui::Context) {
        let mut changed = false;
        while let Ok(load) = self.font_rx.try_recv() {
            let Some((path, index, bytes)) = load.loaded else {
                self.note_status(trf(
                    "No font for {0} was found on this computer, so that text shows as boxes. Installing a font for it (for example Noto Sans) fixes this.",
                    &[&tr(load.script.name())],
                ));
                continue;
            };
            // A font is its file and the face in it: two faces of one
            // collection are two fonts.
            let name = if index == 0 {
                path.display().to_string()
            } else {
                format!("{}#{index}", path.display())
            };
            if let Some(at) = self
                .extra_fonts
                .iter()
                .position(|(known, _)| *known == name)
            {
                // Shared by two scripts: order it by the earlier one, so the
                // result never depends on which load finished first.
                if (load.script as u8) < (self.extra_scripts[at] as u8) {
                    self.extra_scripts[at] = load.script;
                    changed = true;
                }
                continue;
            }
            let mut data = egui::FontData::from_owned(bytes);
            data.index = index;
            self.extra_fonts.push((name, Arc::new(data)));
            self.extra_scripts.push(load.script);
            changed = true;
        }
        if changed {
            // Loads finish in any order; install them in a fixed one, so a
            // character two fonts share (Chinese and Japanese share most)
            // always comes from the same font.
            let mut order: Vec<usize> = (0..self.extra_fonts.len()).collect();
            order.sort_by_key(|&at| self.extra_scripts[at] as u8);
            self.extra_fonts = order
                .iter()
                .map(|&at| self.extra_fonts[at].clone())
                .collect();
            self.extra_scripts = order.iter().map(|&at| self.extra_scripts[at]).collect();
            ctx.set_fonts(font_definitions(&self.extra_fonts));
            FONT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Add a word to the user's dictionary file and to the loaded list.
    fn add_to_dictionary(&mut self, word: &str) {
        let word = crate::spell::normalize(word.trim());
        if word.is_empty() || self.dictionary_path.is_empty() {
            return;
        }
        use std::io::Write as _;
        // Added words go to a UTF-8 file of their own beside the list.
        let added = crate::spell::added_path(std::path::Path::new(&self.dictionary_path));
        let written = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&added)
            .and_then(|mut file| {
                // Start on a new line if the file does not end with one.
                let ends = std::fs::read(&added)
                    .map(|bytes| bytes.last().is_none_or(|last| *last == b'\n'))
                    .unwrap_or(true);
                let lead = if ends { "" } else { "\n" };
                writeln!(file, "{lead}{word}")
            });
        match written {
            Ok(()) => {
                self.dictionary.insert(word);
            }
            Err(err) => self.note_active(trf(
                "Could not add {0} to {1}: {2}",
                &[&word, &self.dictionary_path, &err],
            )),
        }
    }

    fn sync_ident(&mut self) {
        // With "only while connecting", identd runs while a server is being
        // dialed or registered, and stops once every server is connected.
        let connecting = self
            .sessions
            .iter()
            .any(|session| session.link.is_some() && session.connected_at.is_none());
        let wanted = self.settings.ident && (!self.settings.ident_connecting_only || connecting);
        if !wanted {
            self.ident_server = None;
            self.ident_for = None;
            self.ident_failed = false;
            return;
        }
        let user = crate::ident::userid(if self.settings.ident_user.trim().is_empty() {
            &self.settings.user
        } else {
            self.settings.ident_user.trim()
        });
        let port = self.settings.ident_port.max(1);
        let key = format!("{user}@{port}/{}", self.settings.ident_system);
        if self.ident_for.as_deref() == Some(key.as_str())
            && (self.ident_server.is_some() || self.ident_failed)
        {
            return;
        }
        self.ident_server = None;
        match crate::ident::listen_as(("0.0.0.0", port), user, self.settings.ident_system.clone()) {
            Ok((server, _)) => {
                self.ident_server = Some(server);
                self.ident_for = Some(key);
                self.ident_failed = false;
                self.note_status(trf("Identd listening on port {0}.", &[&port]));
            }
            Err(err) => {
                self.ident_for = Some(key);
                self.ident_failed = true;
                self.note_status(trf(
                    "Identd could not listen on port {0}: {1}",
                    &[&port, &err],
                ));
            }
        }
    }

    /// Hand the network settings to the connection code.
    fn apply_net_prefs(&self) {
        let settings = &self.settings;
        let proxy_host = settings.proxy_host.trim();
        let proxy =
            (settings.proxy && !proxy_host.is_empty() && settings.proxy_port > 0).then(|| {
                crate::socks::Endpoint {
                    kind: if settings.proxy_kind.is_empty() {
                        "socks5".into()
                    } else {
                        settings.proxy_kind.clone()
                    },
                    host: proxy_host.to_string(),
                    port: settings.proxy_port,
                    user: settings.proxy_user.clone(),
                    pass: settings.proxy_pass.clone(),
                }
            });
        let dcc_ports = (settings.dcc_port_first > 0).then(|| {
            (
                settings.dcc_port_first,
                settings.dcc_port_last.max(settings.dcc_port_first),
            )
        });
        crate::net::configure(crate::net::NetPrefs {
            order: crate::net::AddressOrder::from_key(&settings.address_order),
            bind: settings.bind_address.trim().parse().ok(),
            bind_fallback: settings.bind_fallback,
            dcc_ports,
            dcc_ip: (settings.dcc_ip_method == "manual")
                .then(|| settings.dcc_ip.trim().parse().ok())
                .flatten(),
            learned_ip: None,
            proxy,
            proxy_scope: crate::net::ProxyScope::from_key(&settings.proxy_scope),
            proxy_except: settings.proxy_except.clone(),
        });
    }

    /// Server numerics: lists go to their windows, the rest to Status.
    fn numeric(&mut self, code: u16, text: String) {
        match code {
            352 | 315 => self.note_active(text),
            // The message of the day.
            372 | 375 | 376 | 422 if self.settings.skip_motd => {}
            // Channel list replies, from the server the list belongs to.
            321..=323
                if self.channel_list.server.is_some()
                    && self.channel_list.server != self.scope() => {}
            321 => {
                self.channel_list.clear();
                self.channel_list.listing = true;
            }
            322 => {
                self.want_fonts(&text);
                if let Some(entry) = channel::parse_listed(&text) {
                    self.channel_list.push(entry);
                }
            }
            323 => self.channel_list.listing = false,
            // Ban, exception, and invite list entries, then their ends.
            367 | 348 | 346 => {
                let letter = match code {
                    367 => 'b',
                    348 => 'e',
                    _ => 'I',
                };
                if let Some((channel, entry)) = channel::parse_list_entry(&text)
                    && let Some(index) = self.find(Kind::Channel, &channel)
                {
                    let list = self.windows[index].mode_lists.entry(letter).or_default();
                    if !list.contains(&entry) {
                        list.push(entry);
                    }
                }
            }
            368 | 349 | 347 => {
                let letter = match code {
                    368 => 'b',
                    349 => 'e',
                    _ => 'I',
                };
                let channel = text.split_whitespace().next().unwrap_or("").to_string();
                if let Some(index) = self.find(Kind::Channel, &channel) {
                    let count = self.windows[index]
                        .mode_lists
                        .entry(letter)
                        .or_default()
                        .len();
                    if self.central.is_none() {
                        let text = match letter {
                            'b' => tr_key("{0} has {1} bans; Channel central lists them."),
                            'e' => tr_key("{0} has {1} exceptions; Channel central lists them."),
                            _ => tr_key("{0} has {1} invites; Channel central lists them."),
                        };
                        self.push(index, event_line(Ev::Mode, trf(text, &[&channel, &count])));
                    }
                }
            }
            302 if self.settings.dcc_ip_method == "server" => {
                // Our own address as the server sees it, for DCC offers.
                if let Some(host) = crate::net::userhost_host(&text).map(str::to_string) {
                    thread::spawn(move || {
                        use std::net::ToSocketAddrs;
                        let found =
                            (host.as_str(), 0)
                                .to_socket_addrs()
                                .ok()
                                .and_then(|mut addrs| {
                                    addrs.find_map(|addr| match addr.ip() {
                                        std::net::IpAddr::V4(ip) => Some(ip),
                                        std::net::IpAddr::V6(_) => None,
                                    })
                                });
                        if let Some(ip) = found {
                            crate::net::learn_ip(ip);
                        }
                    });
                }
            }
            329 | 333 => {}
            // Whois replies, then away replies.
            311 | 312 | 313 | 317 | 318 | 319 | 320 | 330 | 335 | 338 | 378 | 379 | 671 | 276
            | 307 | 314 | 369 => self.note_in(self.settings.active_whois, text),
            301 | 305 | 306 => self.note_in(self.settings.active_away, text),
            _ => self.note_status(text),
        }
    }

    /// Keep the platform window title in step: nick, network, and the
    /// active window, like "alice on Libera.Chat — #rust".
    fn update_title(&mut self, ctx: &egui::Context) {
        let window = self.windows.get(self.active);
        let session = self
            .active_server()
            .and_then(|id| self.sessions.iter().find(|session| session.id == id))
            .filter(|session| session.connected_at.is_some());
        let mut title = String::from("OpenIRC");
        if let Some(session) = session {
            let network = session
                .network
                .clone()
                .unwrap_or_else(|| session.host.clone());
            title = if self.settings.show_network {
                format!("{} on {network}", session.nick)
            } else {
                session.nick.clone()
            };
        }
        if let Some(window) = window
            && window.kind != Kind::Status
        {
            title.push_str(" — ");
            title.push_str(&window.title);
        }
        if session.is_some() {
            title.push_str(" · OpenIRC");
        }
        let template = self.settings.title_text.trim();
        if !template.is_empty() {
            let network = session
                .map(|session| {
                    session
                        .network
                        .clone()
                        .unwrap_or_else(|| session.host.clone())
                })
                .unwrap_or_default();
            let me = session
                .map(|session| session.nick.clone())
                .unwrap_or_default();
            let here = window
                .map(|window| window.title.clone())
                .unwrap_or_default();
            title = template
                .replace("$network", &network)
                .replace("$window", &here)
                .replace("$me", &me);
        }
        if title != self.shown_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.shown_title = title;
        }
    }

    /// Whether we hold operator (or, for voice, halfop) status in window
    /// `index`.
    fn can_set(&self, index: usize, mode: char) -> bool {
        let Some(window) = self.windows.get(index) else {
            return false;
        };
        window
            .nicks
            .iter()
            .find(|nick| same_nick(nick, self.me()))
            .is_some_and(|nick| {
                let prefixes: String = nick
                    .chars()
                    .take_while(|ch| "@+%&~".contains(*ch))
                    .collect();
                prefixes.contains(['@', '&', '~']) || (mode == 'v' && prefixes.contains('%'))
            })
    }

    fn network_of_scope(&self) -> String {
        self.scope()
            .and_then(|id| self.sessions.iter().find(|session| session.id == id))
            .and_then(|session| session.network.clone())
            .unwrap_or_default()
    }

    /// Give the matching automatic status to someone who just joined.
    fn auto_mode_on_join(&mut self, index: usize, who: &str, address: &str, channel: &str) {
        let source = format!("{who}!{}", if address.is_empty() { "*@*" } else { address });
        let network = self.network_of_scope();
        let Some(rule) = self
            .settings
            .auto_modes
            .iter()
            .find(|rule| rule.applies(&source, channel, &network))
            .cloned()
        else {
            return;
        };
        if !self.can_set(index, rule.mode) {
            return;
        }
        let command = format!("/raw MODE {channel} +{} {who}", rule.mode);
        let delay = if self.settings.auto_mode_delay {
            // Two to six seconds, spread by the clock's low bits.
            let spread = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.subsec_millis()) as u64;
            Duration::from_millis(2_000 + spread * 4)
        } else {
            Duration::ZERO
        };
        self.timers.push(UiTimer {
            next: std::time::Instant::now() + delay,
            every: delay,
            left: 1,
            command,
            server: self.scope(),
            window: None,
            origin: None,
            chat: None,
            reply: None,
        });
    }

    /// Give status back to protected people when someone else takes it.
    fn protect_on_mode(
        &mut self,
        by: &str,
        channel: &str,
        changes: &[(bool, char, Option<String>)],
    ) {
        if same_nick(by, self.me()) {
            return;
        }
        let Some(index) = self.find(Kind::Channel, channel) else {
            return;
        };
        let network = self.network_of_scope();
        for (adding, letter, arg) in changes {
            let Some(target) = arg else {
                continue;
            };
            if *adding || !matches!(letter, 'o' | 'h' | 'v') || same_nick(target, by) {
                continue;
            }
            let address = self
                .addresses
                .get(&(self.scope(), nick_key(target)))
                .cloned()
                .unwrap_or_else(|| "*@*".into());
            let source = format!("{target}!{address}");
            let protected = self.settings.auto_modes.iter().any(|rule| {
                rule.protect && rule.mode == *letter && rule.applies(&source, channel, &network)
            });
            if protected && self.can_set(index, *letter) {
                self.send_user(format!("/raw MODE {channel} +{letter} {target}"));
            }
        }
    }

    /// Send a command to every connected server.
    fn broadcast(&mut self, line: String) {
        let ids: Vec<u64> = self
            .sessions
            .iter()
            .filter(|session| session.link.is_some())
            .map(|session| session.id)
            .collect();
        for id in ids {
            self.send_to(id, line.clone());
        }
    }

    /// Color for `nick` in window `index`: the first matching color rule,
    /// else the automatic color, else the ink color.
    fn nick_tone(&self, index: usize, nick: &str) -> Tone {
        let prefix: String = self
            .windows
            .get(index)
            .and_then(|window| window.nicks.iter().find(|item| same_nick(item, nick)))
            .map(|item| {
                item.chars()
                    .take_while(|ch| "@+%&~".contains(*ch))
                    .collect()
            })
            .unwrap_or_default();
        self.nick_tone_with(nick, &prefix)
    }

    /// Put the chosen text around the nick of a chat line and, when asked,
    /// the speaker's highest channel rank (`@`, `+`, ...) before it.
    fn dress_nick(&self, line: &mut RichLine, index: usize, nick: &str, action: bool) {
        if line.spans.len() < 3 {
            return;
        }
        let rank: String = if self.settings.nick_rank {
            self.windows
                .get(index)
                .filter(|window| window.kind == Kind::Channel)
                .and_then(|window| window.nicks.iter().find(|item| same_nick(item, nick)))
                .and_then(|item| item.chars().next())
                .filter(|ch| "@+%&~".contains(*ch))
                .map(String::from)
                .unwrap_or_default()
        } else {
            String::new()
        };
        if action {
            line.spans[0].text = format!("• {rank}");
        } else {
            line.spans[0].text = format!("{}{rank}", self.settings.nick_open);
            line.spans[2].text = format!("{} ", self.settings.nick_close);
        }
    }

    fn nick_tone_with(&self, nick: &str, prefix: &str) -> Tone {
        let bare = bare_nick(nick);
        if let Some(rule) = self
            .settings
            .nick_colors
            .iter()
            .find(|rule| rule.matches(&bare, prefix))
        {
            return Tone::Palette(rule.color);
        }
        if self.settings.auto_nick_colors {
            Tone::Nick(nick_slot(&bare))
        } else {
            Tone::Ink
        }
    }

    fn active_connected(&self) -> bool {
        self.active_server()
            .is_some_and(|id| self.session_connected(id))
    }

    /// Close a window: part a channel, close a query, list, or DCC chat; a
    /// status window disconnects its server and closes all of its windows.
    fn close_window(&mut self, index: usize) {
        let Some(window) = self.windows.get(index) else {
            return;
        };
        let kind = window.kind;
        let server = window.server;
        let title = window.title.clone();
        match kind {
            Kind::Channel => {
                if server.is_some_and(|id| self.session_connected(id)) {
                    let previous = self.routing;
                    self.routing = server;
                    self.send_user(format!("/part {title}"));
                    self.routing = previous;
                }
            }
            Kind::Dcc => {
                let uid = self.windows[index].uid;
                self.end_dcc(uid);
            }
            Kind::Status => {
                if let Some(id) = server {
                    if let Some(session) = self.sessions.iter_mut().find(|session| session.id == id)
                    {
                        session.hangup = true;
                        session.reconnect_at = None;
                        if let Some(link) = &session.link {
                            let _ = link.outgoing.send("/quit".into());
                        }
                    }
                    self.sessions.retain(|session| session.id != id);
                    // The server's DCC chats go with their windows: hang up.
                    let owned: Vec<u64> = self
                        .windows
                        .iter()
                        .filter(|window| window.server == Some(id))
                        .map(|window| window.uid)
                        .collect();
                    for uid in owned {
                        self.end_dcc(uid);
                    }
                    // Chats of this server still being set up stop too: an
                    // offer's listener frees its port at once.
                    self.outgoing_dcc.retain(|(_, owner, _)| *owner != Some(id));
                    self.dcc_connecting
                        .retain(|(_, owner, _)| *owner != Some(id));
                    let active_title = self
                        .windows
                        .get(self.active)
                        .map(|window| (window.kind, window.title.clone(), window.server));
                    self.windows.retain(|window| window.server != Some(id));
                    if self.windows.is_empty() {
                        self.windows.push(ChatWindow::status());
                    }
                    self.active = active_title
                        .and_then(|(kind, title, server)| {
                            self.windows.iter().position(|window| {
                                window.kind == kind
                                    && window.title == title
                                    && window.server == server
                            })
                        })
                        .unwrap_or(0);
                    self.find_cache = None;
                    return;
                }
                if self
                    .windows
                    .iter()
                    .filter(|window| window.kind == Kind::Status)
                    .count()
                    < 2
                {
                    // The last status window stays.
                    return;
                }
            }
            Kind::Query | Kind::List => {}
        }
        let closed_kind = self.windows[index].kind;
        self.windows.remove(index);
        if self.windows.is_empty() {
            self.windows.push(ChatWindow::status());
        }
        if matches!(closed_kind, Kind::Channel | Kind::Query) {
            self.remember_windows();
        }
        if self.active > index || self.active >= self.windows.len() {
            self.active = self.active.saturating_sub(1).min(self.windows.len() - 1);
        }
        self.find_cache = None;
        self.windows[self.active].activity = Activity::None;
    }

    /// Note unread activity in a window other than the active one.
    fn mark(&mut self, index: usize, activity: Activity) {
        if index != self.active && index < self.windows.len() {
            let window = &mut self.windows[index];
            window.activity = window.activity.max(activity);
        }
    }

    fn status_index(&self) -> Option<usize> {
        let server = self.scope();
        self.windows
            .iter()
            .position(|window| window.kind == Kind::Status && window.server == server)
    }

    fn activate(&mut self, index: usize) {
        if index >= self.windows.len() {
            return;
        }
        if index != self.active
            && let Some((id, target, _)) = self.typing_sent.take()
        {
            self.typing_to(id, &target, "done");
        }
        // Remember where the window being left was read up to.
        if index != self.active
            && let Some(left) = self.windows.get_mut(self.active)
        {
            left.marker = Some(left.lines.len());
        }
        self.active = index;
        self.windows[index].activity = Activity::None;
        // A draft carried to another window is meant for that window now.
        if self.draft_for.is_some() {
            self.draft_for = Some(self.recipient());
        }
        let server = self.windows[index].server;
        if self.routing.is_none() {
            // Later lookups this frame compare names the new server's way.
            set_casemap(self.casemap_of(server));
        }
        let kind = self.windows[index].kind;
        let title = self.windows[index].title.clone();
        if matches!(kind, Kind::Channel | Kind::Query)
            && let Some(id) = server.filter(|id| self.session_connected(*id))
        {
            // As a run of its own, and with the nick changes seen here, so a
            // nick that changed before the server side reads this is followed.
            let renames = self
                .sessions
                .iter()
                .find(|session| session.id == id)
                .map_or(0, |session| session.renames);
            // No run of its own: inside one (an alias's `/channel`), it must
            // not end what the run chose.
            self.send_to(id, format!("/via =:{renames} {title} /target -q"));
        }
    }

    /// Add one line to a window: log it when logging is on and keep the
    /// scrollback within `MAX_SCROLLBACK`.
    fn push(&mut self, index: usize, line: RichLine) {
        for piece in &line.spans {
            self.want_fonts(&piece.text);
        }
        let mut line = line;
        if let Some(time) = self.line_time {
            line.time = Some(time);
        }
        if self.windows[index].kind != Kind::List || self.windows[index].title == MESSAGES {
            self.log(index, &line);
        }
        self.windows[index].generation += 1;
        let lines = &mut self.windows[index].lines;
        lines.push(line);
        if lines.len() > MAX_SCROLLBACK {
            let extra = lines.len() - MAX_SCROLLBACK;
            lines.drain(..extra);
            let window = &mut self.windows[index];
            window.marker = window.marker.map(|marker| marker.saturating_sub(extra));
        }
    }

    fn log(&mut self, index: usize, line: &RichLine) {
        if !self.settings.logging || self.preview {
            return;
        }
        let text = if self.settings.log_keep_codes {
            line.coded()
        } else {
            line.plain()
        };
        let path = self.log_path_for(index);
        let time = line.time.unwrap_or_else(crate::clock::now);
        let stamped = format!(
            "[{:04}-{:02}-{:02} {:02}:{:02}:{:02}] {text}",
            time.year, time.month, time.day, time.hour, time.minute, time.second
        );
        let limit = u64::from(self.settings.log_max_kb) * 1024;
        if let Err(err) = settings::append_log_file(&path, &stamped, limit) {
            // Say so, but at most once a minute: a full disk would otherwise
            // add a note for every line.
            if self
                .log_failed
                .is_none_or(|when| when.elapsed() >= Duration::from_secs(60))
            {
                self.log_failed = Some(std::time::Instant::now());
                let note_text = trf("Could not write the log {0}: {1}", &[&path.display(), &err]);
                if let Some(status) = self.status_index() {
                    self.windows[status].lines.push(note(note_text));
                }
            }
        }
    }

    /// The log file of a window, after the folder and file-name options.
    fn log_path_for(&self, index: usize) -> std::path::PathBuf {
        let window = &self.windows[index];
        let network = self
            .settings
            .log_by_network
            .then(|| {
                self.sessions
                    .iter()
                    .find(|session| Some(session.id) == window.server)
                    .map(|session| {
                        session
                            .network
                            .clone()
                            .unwrap_or_else(|| session.host.clone())
                    })
            })
            .flatten();
        settings::log_file(
            &self.settings.log_dir,
            network.as_deref(),
            &window.title,
            &self.settings.log_dated,
            &crate::clock::now(),
        )
    }

    /// Show the end of a window's log when it opens, so the conversation
    /// picks up where it left off.
    fn reload_log(&mut self, index: usize) {
        let count = self.settings.log_reload as usize;
        if count == 0 || self.preview {
            return;
        }
        let path = self.log_path_for(index);
        let lines = settings::last_log_lines(&path, count);
        if lines.is_empty() {
            return;
        }
        let window = &mut self.windows[index];
        for raw in lines {
            let (time, text) = split_log_stamp(&raw);
            let mut line = event_line(Ev::Info, crate::colors::strip(text));
            line.spans.remove(0);
            line.time = time;
            window.lines.push(line);
        }
        window
            .lines
            .push(note(tr("(earlier lines are from the log)")));
        window.generation += 1;
    }

    fn note_status(&mut self, text: String) {
        let server = self.scope();
        let index = self
            .windows
            .iter()
            .position(|window| window.kind == Kind::Status && window.server == server)
            .or_else(|| {
                self.windows
                    .iter()
                    .position(|window| window.kind == Kind::Status)
            });
        if let Some(index) = index {
            self.push(index, note(text));
        }
    }

    fn note_active(&mut self, text: String) {
        if self.preview {
            // The preview channel is dropped on the first connect; keep
            // anything worth reading in Status.
            self.note_status(text);
            return;
        }
        if self.active < self.windows.len() {
            self.push(self.active, note(text));
        }
    }

    fn session_mut(&mut self) -> Option<&mut Session> {
        let id = self.scope()?;
        self.sessions.iter_mut().find(|session| session.id == id)
    }

    /// The server asked for TLS (IRCv3 STS). On a plain connection, switch
    /// to TLS on the given port and connect again; on a TLS connection,
    /// remember the policy for as long as the server says.
    fn on_sts(&mut self, port: Option<u16>, duration: Option<u64>) {
        let Some(id) = self.scope() else {
            return;
        };
        let Some(session) = self.sessions.iter_mut().find(|session| session.id == id) else {
            return;
        };
        let host = session.host.clone();
        if session.tls {
            let Some(duration) = duration else {
                return;
            };
            let port = session.port;
            self.settings
                .sts_policies
                .retain(|policy| !same_name(&policy.0, &host));
            if duration > 0 {
                let until = crate::rules::unix_now().saturating_add(duration);
                self.settings.sts_policies.push((host, port, until));
            }
            self.persist();
            return;
        }
        let Some(port) = port else {
            return;
        };
        session.tls = true;
        session.port = port;
        session.entry.tls = true;
        session.entry.port = port;
        if let Some(link) = session.link.take() {
            let _ = link.outgoing.send("/quit".into());
        }
        self.note_status(trf(
            "{0} asks for a secure connection; reconnecting with TLS on port {1}.",
            &[&host, &port],
        ));
        self.reconnect(id);
    }

    /// The port and TLS setting to use for `host`, after any remembered
    /// STS policy.
    fn sts_upgrade(&mut self, host: &str, port: u16, tls: bool) -> (u16, bool) {
        let now = crate::rules::unix_now();
        self.settings.sts_policies.retain(|policy| policy.2 > now);
        if tls {
            return (port, tls);
        }
        match self
            .settings
            .sts_policies
            .iter()
            .find(|policy| same_name(&policy.0, host))
        {
            Some(policy) => (policy.1, true),
            None => (port, tls),
        }
    }

    /// Who is typing in the active window, for the line above the input.
    fn typing_label(&mut self) -> Option<String> {
        let window = self.windows.get_mut(self.active)?;
        window.typing.retain(|(_, paused, at)| {
            at.elapsed() < Duration::from_secs(if *paused { 30 } else { 6 })
        });
        let active: Vec<&str> = window
            .typing
            .iter()
            .filter(|(_, paused, _)| !paused)
            .map(|(nick, _, _)| nick.as_str())
            .collect();
        match active.as_slice() {
            [] => None,
            [one] => Some(trf("{0} is typing…", &[&one])),
            [one, two] => Some(trf("{0} and {1} are typing…", &[&one, &two])),
            _ => Some(tr("Several people are typing…").into()),
        }
    }

    /// Tell the other side we are typing, at most every three seconds, and
    /// that we stopped when the input is cleared.
    fn send_typing(&mut self) {
        if !self.settings.send_typing {
            return;
        }
        let Some(window) = self.windows.get(self.active) else {
            return;
        };
        let target =
            (matches!(window.kind, Kind::Channel | Kind::Query)).then(|| window.title.clone());
        let server = window.server;
        let typing = !self.input.trim().is_empty() && !self.input.starts_with('/');
        match (&self.typing_sent, target, server) {
            (Some((id, sent_to, at)), Some(target), Some(server))
                if *id == server && same_name(sent_to, &target) =>
            {
                if !typing {
                    self.typing_sent = None;
                    self.typing_to(server, &target, "done");
                } else if at.elapsed() >= Duration::from_secs(3) {
                    self.typing_sent = Some((server, target.clone(), std::time::Instant::now()));
                    self.typing_to(server, &target, "active");
                }
            }
            (previous, target, server) => {
                if let Some((id, sent_to, _)) = previous.clone() {
                    self.typing_to(id, &sent_to, "done");
                    self.typing_sent = None;
                }
                if typing && let (Some(target), Some(server)) = (target, server) {
                    self.typing_sent = Some((server, target.clone(), std::time::Instant::now()));
                    self.typing_to(server, &target, "active");
                }
            }
        }
    }

    /// Where an event line goes: the active window when `active` is set and
    /// that window belongs to the same server, else the status window.
    fn event_window(&self, active: bool) -> Option<usize> {
        if active
            && !self.preview
            && self
                .windows
                .get(self.active)
                .is_some_and(|window| window.server == self.scope())
        {
            return Some(self.active);
        }
        self.status_index()
    }

    fn note_in(&mut self, active: bool, text: String) {
        match self.event_window(active) {
            Some(index) => self.push(index, note(text)),
            None => self.note_status(text),
        }
    }

    /// The window for a private message with `nick`, and whether a query
    /// was just opened for it. An open query is used as it is. Otherwise
    /// the message goes to the Messages window when that is on, or when
    /// other people have opened too many queries within the flood window.
    fn private_window(&mut self, nick: &str, own: bool) -> (usize, bool) {
        if let Some(index) = self.find(Kind::Query, nick) {
            return (index, false);
        }
        if !own {
            let window = Duration::from_secs(self.settings.flood_window_secs.max(1));
            self.query_opens.retain(|opened| opened.elapsed() < window);
            let limit = self.settings.query_limit as usize;
            let flooded = limit > 0 && self.query_opens.len() >= limit;
            if self.settings.message_window || flooded {
                let fresh = self.find(Kind::List, MESSAGES).is_none();
                let index = self.ensure(Kind::List, MESSAGES);
                if flooded && fresh && !self.settings.message_window {
                    self.push(
                        index,
                        note(
                            tr("Many people opened queries at once; their messages are collected here for now."),
                        ),
                    );
                }
                let server = self.scope();
                self.last_private.insert(server, nick.to_string());
                return (index, false);
            }
            self.query_opens.push(std::time::Instant::now());
        }
        (self.ensure(Kind::Query, nick), true)
    }

    /// Run `act` as if `server` were the active server, so commands it sends
    /// go there whatever window is in front now.
    fn on_server<R>(&mut self, server: Option<u64>, act: impl FnOnce(&mut Self) -> R) -> R {
        let previous = self.routing;
        let outer_case = casemap();
        self.routing = server.or(previous);
        set_casemap(self.casemap_of(self.scope()));
        let result = act(self);
        self.routing = previous;
        set_casemap(outer_case);
        result
    }

    fn casemap_of(&self, server: Option<u64>) -> crate::state::CaseMap {
        server
            .and_then(|id| self.sessions.iter().find(|session| session.id == id))
            .map_or(crate::state::CaseMap::Rfc1459, |session| session.casemap)
    }

    /// Our nick on the server being handled now (the event's server, or the
    /// one in front), else the nick from the options.
    fn me(&self) -> &str {
        self.scope()
            .and_then(|id| self.sessions.iter().find(|session| session.id == id))
            .map(|session| session.nick.as_str())
            .filter(|nick| !nick.is_empty())
            .unwrap_or(&self.nick)
    }

    fn scope(&self) -> Option<u64> {
        self.routing.or_else(|| self.active_server())
    }

    fn active_server(&self) -> Option<u64> {
        self.windows
            .get(self.active)
            .and_then(|window| window.server)
    }

    fn session_connected(&self, id: u64) -> bool {
        self.sessions
            .iter()
            .any(|session| session.id == id && session.link.is_some())
    }

    /// Tell `target` on server `id` whether we are typing to them. It goes
    /// with the nick changes seen here, so a nick that changed before the
    /// server side reads it is followed, never told to whoever took it.
    fn typing_to(&mut self, id: u64, target: &str, state: &str) {
        let renames = self
            .sessions
            .iter()
            .find(|session| session.id == id)
            .map_or(0, |session| session.renames);
        self.send_to(id, format!("/via =:{renames} {target} /typing {state}"));
    }

    fn send_to(&mut self, id: u64, line: String) {
        self.watch_secret(&line);
        if let Some(link) = self
            .sessions
            .iter()
            .find(|session| session.id == id)
            .and_then(|session| session.link.as_ref())
        {
            let _ = link.outgoing.send(line);
        }
    }

    fn reconnect(&mut self, id: u64) {
        let Some(session) = self.sessions.iter().find(|session| session.id == id) else {
            return;
        };
        if session.hangup || session.link.is_some() {
            return;
        }
        let mut entry = session.entry.clone();
        if !session.nick.trim().is_empty() {
            entry.nick = session.nick.clone();
        }
        let (host, port, tls) = (session.host.clone(), session.port, session.tls);
        let mut channels = session.autojoin.clone();
        let (port, tls) = self.sts_upgrade(&host, port, tls);
        for window in &self.windows {
            if window.server == Some(id)
                && window.kind == Kind::Channel
                && !channels
                    .iter()
                    .any(|channel| same_name(channel, &window.title))
            {
                channels.push(window.title.clone());
            }
        }
        let config = self.session_config(&entry, channels);
        let proxy = self.proxy_choice(&host);
        let options = self.connect_options(&host);
        let link = spawn_link(host.clone(), port, tls, config, proxy, options);
        if let Some(session) = self.sessions.iter_mut().find(|session| session.id == id) {
            session.reconnect_at = None;
            session.away = None;
            session.link = Some(link);
            // A new connection's server side starts counting again.
            session.renames = 0;
            session.port = port;
            session.tls = tls;
            session.caps.clear();
            session.away_nicks.clear();
            session.accounts.clear();
            session.umodes.clear();
            session.notify_online.clear();
        }
        self.on_server(Some(id), |app| {
            app.note_status(trf("Reconnecting to {0}:{1}", &[&host, &port]));
        });
    }
    fn switch_label(&self, index: usize) -> String {
        let window = &self.windows[index];
        if self.sessions.len() < 2 {
            return window.title.clone();
        }
        let host = self
            .sessions
            .iter()
            .find(|session| Some(session.id) == window.server)
            .map(|session| {
                session
                    .network
                    .as_deref()
                    .filter(|network| !network.trim().is_empty())
                    .unwrap_or_else(|| short_host(&session.host))
            })
            .unwrap_or("");
        if window.kind == Kind::Status {
            host.to_string()
        } else if host.is_empty() {
            window.title.clone()
        } else {
            format!("{}  {host}", window.title)
        }
    }
}

impl eframe::App for OpenIrcApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let started = self.profile.start();
        self.tick(ctx);
        self.profile.stop("logic", started);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let started = self.profile.start();
        self.draw(ui);
        self.profile.stop("ui", started);
    }

    fn on_exit(&mut self) {
        self.shut_down();
        if self.profile.enabled() {
            let report = self.profile.report();
            eprintln!("OpenIRC profile\n{report}");
            let _ = std::fs::write(settings::config_dir().join("profile.txt"), report);
        }
    }
}

impl OpenIrcApp {
    fn tick(&mut self, ctx: &egui::Context) {
        // First, so everything built this frame (the tray menu too) uses it.
        self.apply_language();
        theme::install(self.view_settings().theme);
        theme::install_palette(self.view_settings().palette);
        self.sync_ident();
        self.poll_tray(ctx);
        self.track_place(ctx);
        self.poll_fonts(ctx);
        self.tick_timers();
        self.load_dictionary();
        self.update_spell_skip();
        self.poll_networks(ctx);
        if !self.script_started {
            self.script_started = true;
            let me = self.me().to_string();
            self.fire_scripts("start", &me, "", "");
        }
        if self.autoconnect {
            self.autoconnect = false;
            self.started_connect = true;
            self.connect();
        }
        if !self.started_connect {
            self.started_connect = true;
            self.apply_net_prefs();
            if self.settings.connect_on_start && self.sessions.is_empty() {
                self.connect();
            }
        }
        // Between servers' lines, names compare the way the server in front
        // does.
        set_casemap(self.casemap_of(self.active_server()));
        let started = self.profile.start();
        self.poll();
        self.profile.stop("logic.network", started);
        self.poll_dcc();
        self.poll_lookups();
        self.check_auto_away();
        let transferring =
            self.settings.dcc_keep_awake && self.transfers.iter().any(transfers::Transfer::running);
        if transferring != self.awake.is_some() {
            self.awake = transferring.then(crate::awake::Awake::start);
        }
        if self.notify_polled.elapsed() >= Duration::from_secs(60) {
            // Servers with MONITOR report changes themselves and ignore this.
            self.notify_polled = std::time::Instant::now();
            self.broadcast("/notifypoll".into());
        }
        if self.exit_requested {
            self.exit_requested = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let dropped: Vec<std::path::PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .filter(|path| !path.as_os_str().is_empty())
                .collect()
        });
        if !dropped.is_empty() {
            self.drop_files(dropped);
        }
        let due: Vec<u64> = self
            .sessions
            .iter()
            .filter(|session| {
                session.link.is_none()
                    && session
                        .reconnect_at
                        .is_some_and(|at| std::time::Instant::now() >= at)
            })
            .map(|session| session.id)
            .collect();
        for id in due {
            self.reconnect(id);
        }
        let busy = self
            .sessions
            .iter()
            .any(|session| session.link.is_some() || session.reconnect_at.is_some())
            || !self.outgoing_dcc.is_empty()
            || !self.dcc_connecting.is_empty()
            || !self.dcc_links.is_empty()
            || self.transfers.iter().any(transfers::Transfer::running);
        if busy {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        self.handle_shortcuts(ui.ctx());
        self.update_title(ui.ctx());
        if self.flash {
            self.flash = false;
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                    egui::UserAttentionType::Informational,
                ));
        }
        if let Some(text) = self.copy_request.take() {
            ui.ctx().copy_text(text);
        }
        let view = self.view_settings();
        let wanted = (view.theme, view.font_size);
        theme::install(view.theme);
        // Rebuilding the egui style is not free; only do it when the colors
        // or the font size change.
        if self.applied_style != Some(wanted) {
            theme(ui.ctx(), wanted.1 as f32);
            self.applied_style = Some(wanted);
        }
        self.menu_bar(ui);
        self.toolbar(ui);
        if self.show_switchbar {
            self.switchbar(ui);
        }
        self.status_bar(ui);
        if self.settings.monitor_panel {
            self.monitor_panel(ui);
        }
        if self.show_tree {
            self.tree(ui);
        }
        let started = self.profile.start();
        self.client(ui);
        self.profile.stop("ui.client", started);
        self.server_list_window(ui.ctx());
        self.trust_dialog(ui.ctx());
        self.prompt_dialog(ui.ctx());
        self.options_dialog(ui.ctx());
        self.favorites_dialog(ui.ctx());
        self.scripts_dialog(ui.ctx());
        self.address_dialog(ui.ctx());
        self.dcc_dialog(ui.ctx());
        self.about_dialog(ui.ctx());
        self.log_browser(ui.ctx());
        self.clear_history_dialog(ui.ctx());
        self.search_window(ui.ctx());
        self.get_prompt_dialog(ui.ctx());
        self.transfers_window(ui.ctx());
        self.certificates_window(ui.ctx());
        self.dialog_editor(ui.ctx());
        self.live_dialog(ui.ctx());
        self.channel_central(ui.ctx());
        self.channel_list_window(ui.ctx());
        self.profiler_window(ui.ctx());
        self.toasts(ui.ctx());
    }

    /// Remember each server's open channels and queries for the next start.
    fn remember_windows(&mut self) {
        if self.preview {
            return;
        }
        let mut remembered: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();
        for session in &self.sessions {
            let titles = |kind: Kind| -> Vec<String> {
                self.windows
                    .iter()
                    .filter(|window| window.server == Some(session.id) && window.kind == kind)
                    .map(|window| window.title.clone())
                    .collect()
            };
            remembered.push((
                session.host.clone(),
                titles(Kind::Channel),
                titles(Kind::Query),
            ));
        }
        let hosts: Vec<String> = remembered.iter().map(|(host, _, _)| host.clone()).collect();
        // Servers not connected now keep what was remembered for them.
        self.settings
            .last_windows
            .retain(|(host, _, _)| !hosts.iter().any(|item| same_name(item, host)));
        self.settings.last_windows.extend(remembered);
        if self.settings.restore_windows {
            self.persist();
        }
    }

    /// The channels and queries to reopen for `host`.
    fn windows_to_restore(&self, host: &str) -> (Vec<String>, Vec<String>) {
        if !self.settings.restore_windows {
            return (Vec::new(), Vec::new());
        }
        self.settings
            .last_windows
            .iter()
            .find(|(item, _, _)| same_name(item, host))
            .map(|(_, channels, queries)| (channels.clone(), queries.clone()))
            .unwrap_or_default()
    }

    /// Copy the input history into the settings, leaving out lines that
    /// sent a password when they ran, lines whose timers or paste have not
    /// run yet, and lines that look as if they may hold one.
    fn save_history(&mut self) {
        if !self.settings.keep_history || self.preview {
            return;
        }
        // Lines whose work is still to come or still running.
        let waiting: std::collections::HashSet<u64> = self
            .timers
            .iter()
            .filter_map(|timer| timer.origin)
            .chain(self.running.iter().copied())
            .chain(self.paste_entry)
            .chain(self.provenance)
            .collect();
        self.settings.history = self
            .history
            .shareable(&waiting)
            .filter(|line| !self.may_hold_secret(line))
            .cloned()
            .collect();
        #[cfg(test)]
        self.history_saves.push(self.settings.history.clone());
        self.persist();
    }

    /// A line on its way to running or to a server: when it may hold a
    /// password, the history entry it came from (`provenance`) is marked,
    /// and history already saved is written again without it.
    /// Keep the typed entry that is running now out of saved history.
    fn hold_back(&mut self) {
        if let Some(entry) = self.provenance
            && self.history.mark_secret(entry)
        {
            self.save_history();
        }
    }

    fn watch_secret(&mut self, line: &str) {
        let Some(entry) = self.provenance else {
            return;
        };
        if input::may_hold_secret(line, "/") && self.history.mark_secret(entry) {
            self.save_history();
        }
    }

    /// Whether a typed entry may hold a password, by reading it: with the
    /// configured command prefix, and through the user's quick aliases as
    /// deep as commands may nest. Script aliases are not read here; what
    /// they send is caught when they run (`watch_secret`).
    fn may_hold_secret(&self, entry: &str) -> bool {
        let prefix = self.settings.command_prefix.trim();
        entry.lines().any(|line| {
            let line = line.trim();
            if input::may_hold_secret(line, prefix) {
                return true;
            }
            let typed = match line.strip_prefix(prefix) {
                Some(command) if !prefix.is_empty() && prefix != "/" => format!("/{command}"),
                _ => line.to_string(),
            };
            self.alias_may_hold_secret(&typed, 0, &mut Vec::new())
        })
    }

    /// One quick-alias call and the calls it makes, to the same depth
    /// `dispatch` allows, never through one alias twice.
    fn alias_may_hold_secret(&self, line: &str, depth: u8, seen: &mut Vec<String>) -> bool {
        let name = line
            .strip_prefix('/')
            .and_then(|rest| rest.split_whitespace().next())
            .unwrap_or("")
            .to_ascii_lowercase();
        if depth >= MAX_DEPTH || name.is_empty() || seen.contains(&name) {
            return false;
        }
        let Some(parts) = self.settings.expand_alias(line) else {
            return false;
        };
        seen.push(name);
        parts.iter().any(|part| {
            input::may_hold_secret(part, "/") || self.alias_may_hold_secret(part, depth + 1, seen)
        })
    }

    /// Note where the main window is, so the next start opens it there.
    /// A maximized, full-screen, or minimized window keeps the last normal
    /// place, so leaving that state goes back to it.
    fn track_place(&mut self, ctx: &egui::Context) {
        let (outer, inner, maximized, full, minimized) = ctx.input(|input| {
            let viewport = input.viewport();
            (
                viewport.outer_rect,
                viewport.inner_rect,
                viewport.maximized,
                viewport.fullscreen,
                viewport.minimized,
            )
        });
        if full == Some(true) || minimized == Some(true) {
            return;
        }
        if let Some(maximized) = maximized {
            self.settings.window_maximized = maximized;
            if maximized {
                return;
            }
        }
        if let (Some(outer), Some(inner)) = (outer, inner)
            && inner.width() >= 200.0
            && inner.height() >= 150.0
        {
            // Some window managers report a frame that differs from the one
            // they add when placing a window, so saving the frame's corner
            // makes the window creep on every start. Instead, learn how far
            // the content landed from the place asked for (once the frame is
            // on) and save the content corner less that.
            if self.place_offset.is_none() && outer != inner {
                self.place_offset = Some(
                    self.place_request
                        .map_or(egui::Vec2::ZERO, |asked| inner.min - asked),
                );
            }
            let at = inner.min - self.place_offset.unwrap_or_default();
            self.settings.window_place = Some([
                at.x.round() as i32,
                at.y.round() as i32,
                inner.width().round() as i32,
                inner.height().round() as i32,
            ]);
        }
    }

    fn shut_down(&mut self) {
        self.save_history();
        self.remember_windows();
        // The window's place (and anything else changed since the last save).
        self.persist();
        // Say goodbye on every server, then give the network threads a
        // moment to write QUIT before the process ends.
        for session in &self.sessions {
            if let Some(link) = &session.link {
                let _ = link.outgoing.send("/quit".into());
            }
        }
        let deadline = std::time::Instant::now() + Duration::from_millis(500);
        while std::time::Instant::now() < deadline {
            let running = self.sessions.iter().any(|session| {
                session.link.as_ref().is_some_and(|link| {
                    loop {
                        match link.incoming.try_recv() {
                            Ok(_) => continue,
                            Err(mpsc::TryRecvError::Empty) => break true,
                            Err(mpsc::TryRecvError::Disconnected) => break false,
                        }
                    }
                })
            });
            if !running {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        for session in &self.sessions {
            if let Some(link) = &session.link {
                link.stop.store(true, Ordering::Relaxed);
            }
        }
    }
}

impl OpenIrcApp {
    fn find_matches(&mut self) -> Option<std::rc::Rc<[usize]>> {
        if self.find_text.is_empty() {
            return None;
        }
        let window = self.windows.get(self.active)?;
        let key = (
            self.active,
            window.title.clone(),
            window.generation,
            self.find_text.clone(),
        );
        if let Some((cached, found)) = &self.find_cache
            && *cached == key
        {
            return Some(found.clone());
        }
        let query = self.find_text.to_lowercase();
        let found: std::rc::Rc<[usize]> = window
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line_matches(line, &query))
            .map(|(index, _)| index)
            .collect();
        self.find_cache = Some((key, found.clone()));
        Some(found)
    }

    /// Set away on every connected server after the idle time in Options,
    /// unless you set away yourself.
    fn check_auto_away(&mut self) {
        let minutes = self.settings.auto_away_minutes;
        if minutes == 0 || self.auto_away_on {
            return;
        }
        if self.last_input.elapsed() < Duration::from_secs(u64::from(minutes) * 60) {
            return;
        }
        let connected: Vec<u64> = self
            .sessions
            .iter()
            .filter(|session| session.connected_at.is_some() && session.link.is_some())
            .filter(|session| session.away.is_none())
            .map(|session| session.id)
            .collect();
        if connected.is_empty() {
            return;
        }
        self.auto_away_on = true;
        let reason = if self.settings.auto_away_reason.trim().is_empty() {
            tr("Away from the keyboard").to_string()
        } else {
            self.settings.auto_away_reason.trim().to_string()
        };
        for id in connected {
            self.send_to(id, format!("/away {reason}"));
        }
    }

    /// You typed something: end the automatic away.
    fn user_active(&mut self) {
        self.last_input = std::time::Instant::now();
        if self.auto_away_on {
            self.auto_away_on = false;
            if self.settings.auto_back {
                self.broadcast("/back".into());
            }
        }
    }

    /// The script popups for one right-click menu.
    fn popups_for(&self, menu: &str) -> Vec<crate::settings::Popup> {
        self.settings
            .popups
            .iter()
            .filter(|popup| popup.menu == menu || (menu == "line" && popup.menu.is_empty()))
            .cloned()
            .collect()
    }

    /// Send the notify list to every connected server; each works out
    /// what changed.
    fn sync_notify(&mut self) {
        let list = self.settings.notify.join(",");
        self.broadcast(format!("/notifylist {list}"));
    }

    /// Whether a window is shown in the fixed-width font: all of them when
    /// that option is on, else the ones picked one by one.
    fn window_mono(&self, index: usize) -> bool {
        self.view_settings().transcript_mono
            || self.windows.get(index).is_some_and(|window| {
                self.settings
                    .mono_windows
                    .iter()
                    .any(|title| same_name(title, &window.title))
            })
    }

    fn toggle_mono(&mut self, index: usize) {
        let Some(title) = self.windows.get(index).map(|window| window.title.clone()) else {
            return;
        };
        let before = self.settings.mono_windows.len();
        self.settings
            .mono_windows
            .retain(|item| !same_name(item, &title));
        if before == self.settings.mono_windows.len() {
            self.settings.mono_windows.push(title);
        }
        self.persist();
    }

    /// Run a configured command with `$nick`, `$chan`, and `$file` filled
    /// in; the separator splits it into several commands.
    fn run_with_fields(&mut self, command: &str, nick: &str, channel: &str, file: &str) {
        if command.trim().is_empty() {
            return;
        }
        let time = crate::clock::hm();
        let fields = crate::script::Fields {
            nick,
            chan: channel,
            me: self.me(),
            text: "",
            time: &time,
        };
        let separator = self.settings.split_token();
        let commands: Vec<String> = command
            .split(separator.as_str())
            .map(|part| part.trim().replace("$file", file))
            .map(|part| crate::script::expand(&part, &fields, &self.settings.vars))
            .filter(|part| !part.is_empty())
            .collect();
        // One menu choice, click or drop is one run (see `run`).
        self.in_run(|app| {
            for command in commands {
                app.dispatch(command, Origin::Script);
            }
        });
    }

    /// Files dropped on the window: run the drop command for each, with the
    /// nick of a query or the one picked in the nick list.
    fn drop_files(&mut self, paths: Vec<std::path::PathBuf>) {
        let Some(window) = self.windows.get(self.active) else {
            return;
        };
        let nick = match window.kind {
            Kind::Query => window.title.clone(),
            Kind::Channel => self
                .drop_nick
                .take()
                .or_else(|| self.selected_nick.clone())
                .unwrap_or_default(),
            _ => String::new(),
        };
        let channel = if window.kind == Kind::Channel {
            window.title.clone()
        } else {
            String::new()
        };
        if nick.is_empty() && self.settings.drop_command.contains("$nick") {
            self.note_active(tr("Drop the file on a nick in the nick list, or in a query.").into());
            return;
        }
        let command = self.settings.drop_command.clone();
        for path in paths {
            let file = path.display().to_string();
            self.run_with_fields(&command, &nick, &channel, &file);
        }
    }

    fn run_line_action(&mut self, channel: &str, action: LineAction) {
        // Every line one click or drop runs is one run (see `run`).
        self.in_run(|app| app.run_one_action(channel, action));
    }

    fn run_one_action(&mut self, channel: &str, action: LineAction) {
        match action {
            LineAction::Nick(command, nick) => self.run_nick_action(channel, &nick, command),
            LineAction::Select(nick) => self.selected_nick = Some(nick),
            LineAction::NickDouble(nick) => {
                let command = self.settings.dclick_nick.clone();
                self.run_with_fields(&command, &nick, channel, "");
            }
            LineAction::ToggleMono => {
                let active = self.active;
                self.toggle_mono(active);
            }
            LineAction::WindowDouble => {
                let Some(window) = self.windows.get(self.active) else {
                    return;
                };
                let (command, nick) = match window.kind {
                    Kind::Channel => (self.settings.dclick_channel.clone(), String::new()),
                    Kind::Query => (self.settings.dclick_query.clone(), window.title.clone()),
                    Kind::Status => (self.settings.dclick_status.clone(), String::new()),
                    Kind::List | Kind::Dcc => return,
                };
                self.run_with_fields(&command, &nick, channel, "");
            }
            LineAction::Copy(text) => self.copy_request = Some(text),
            LineAction::Open(url) => open_url(&url),
            LineAction::MenuWord(word) => self.menu_word = word,
            LineAction::Search(words) => {
                open_url(&search_address(&self.settings.search_url, &words));
            }
            LineAction::Popup { body, nick, text } => {
                let time = crate::clock::hm();
                let fields = crate::script::Fields {
                    nick: &nick,
                    chan: channel,
                    me: self.me(),
                    text: &text,
                    time: &time,
                };
                let separator = self.settings.split_token();
                let commands: Vec<String> = body
                    .split(separator.as_str())
                    .map(|part| crate::script::expand(part.trim(), &fields, &self.settings.vars))
                    .filter(|part| !part.is_empty())
                    .collect();
                for command in commands {
                    self.dispatch(command, Origin::Script);
                }
            }
        }
    }

    fn refresh_networks(&mut self) {
        if self.network_rx.is_some() {
            return;
        }
        self.network_note = tr("Updating the network list...").into();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result = crate::networks::fetch_directory().map_err(|err| err.to_string());
            let _ = tx.send(result);
        });
        self.network_rx = Some(rx);
    }

    fn poll_networks(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.network_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(list)) => {
                let count = list.len();
                self.networks = crate::networks::merge(crate::networks::builtin(), list);
                self.network_note = trf("{0} networks from the directory.", &[&count]);
                ctx.request_repaint();
            }
            Ok(Err(err)) => {
                self.network_note = trf("Network list: {0}", &[&err]);
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.network_rx = Some(rx);
                ctx.request_repaint_after(Duration::from_millis(200));
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.network_note = tr("Network list update stopped.").into();
            }
        }
    }

    fn poll_dcc(&mut self) {
        // Every chat being set up is followed on its own: a second one never
        // drops the first.
        let mut done = Vec::new();
        for (at, (_, _, pending)) in self.dcc_connecting.iter().enumerate() {
            match pending.try_recv() {
                Ok(result) => done.push((at, Some(result))),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => done.push((at, None)),
            }
        }
        for (at, result) in done.into_iter().rev() {
            let (nick, server, _) = self.dcc_connecting.remove(at);
            let failed = match result {
                Some(Ok(chat)) => {
                    self.attach_dcc(nick, server, chat);
                    continue;
                }
                Some(Err(err)) => trf("The DCC chat with {0} failed: {1}", &[&nick, &err]),
                None => trf("The DCC chat with {0} failed.", &[&nick]),
            };
            self.dcc_note(server, failed);
        }
        let mut done = Vec::new();
        for (at, (_, _, pending)) in self.outgoing_dcc.iter().enumerate() {
            match pending.chat.try_recv() {
                Ok(chat) => done.push((at, Some(chat))),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => done.push((at, None)),
            }
        }
        for (at, chat) in done.into_iter().rev() {
            let (nick, server, _) = self.outgoing_dcc.remove(at);
            match chat {
                Some(chat) => self.attach_dcc(nick, server, chat),
                None => self.dcc_note(server, trf("{0} did not accept the DCC chat.", &[&nick])),
            }
        }
        // A connection whose window is gone (its server was closed, say)
        // is let go, which hangs up.
        let windows: std::collections::HashSet<u64> =
            self.windows.iter().map(|window| window.uid).collect();
        self.dcc_links.retain(|link| windows.contains(&link.window));
        // The frame's lines are shared among the chats: each gets a share,
        // never more than what is left of the whole, and the next frame
        // starts with the first chat not reached, so a busy peer cannot
        // hold back the others or hide that one of them hung up, however
        // many chats there are. What quiet chats leave over goes, in
        // further rounds, to those that still have lines.
        let count = self.dcc_links.len();
        let first = self.dcc_turn % count.max(1);
        let mut left = FRAME_MESSAGES;
        let mut reached = 0;
        let mut incoming: Vec<(u64, String)> = Vec::new();
        let mut ended: Vec<u64> = Vec::new();
        // Read up to `share` lines from the chat at `at`; true when it had
        // that many, so it may have more.
        let mut read = |link: &DccLink, share: usize, left: &mut usize| {
            let mut took = 0;
            while took < share.min(*left) {
                match link.incoming.try_recv() {
                    Ok(line) => {
                        incoming.push((link.window, line));
                        took += 1;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    // The chat's thread ended, after everything it read
                    // above: the peer hung up or the connection broke.
                    Err(mpsc::TryRecvError::Disconnected) => {
                        ended.push(link.id);
                        break;
                    }
                }
            }
            *left -= took;
            took == share
        };
        // First round: every chat in turn, from where the last frame
        // stopped.
        let share = (FRAME_MESSAGES / count.max(1)).max(1);
        let mut more = Vec::new();
        for offset in 0..count {
            if left == 0 {
                break;
            }
            reached = offset + 1;
            let at = (first + offset) % count;
            // A new chat waits until the old one in its window is done.
            if self.dcc_links[at].after.is_some() {
                continue;
            }
            if read(&self.dcc_links[at], share, &mut left) {
                more.push(at);
            }
        }
        // Further rounds: what is left, among the chats that had more.
        while left > 0 && !more.is_empty() {
            let share = (left / more.len()).max(1);
            let mut still = Vec::new();
            for &at in &more {
                if left == 0 {
                    break;
                }
                if read(&self.dcc_links[at], share, &mut left) {
                    still.push(at);
                }
            }
            more = still;
        }
        // Everyone reached: the next frame starts one further on.
        self.dcc_turn = first + if reached == count { 1 } else { reached };
        for (window, line) in incoming {
            if let Some(index) = self.window_by_uid(window) {
                self.push(index, note(line));
                self.mark(index, Activity::Message);
            }
        }
        for id in ended {
            let Some(at) = self.dcc_links.iter().position(|link| link.id == id) else {
                continue;
            };
            let link = self.dcc_links.remove(at);
            if let Some(index) = self.window_by_uid(link.window) {
                self.push(index, note(tr("The DCC chat ended.")));
            }
            // A chat waiting for this one to finish begins now.
            let waiting = self
                .dcc_links
                .iter_mut()
                .find(|next| next.after.as_ref().is_some_and(|(old, _)| *old == id));
            if let Some(next) = waiting
                && let Some((_, opening)) = next.after.take()
            {
                let window = next.window;
                if let Some(index) = self.window_by_uid(window) {
                    self.push(index, note(opening));
                }
            }
        }
        self.poll_transfers();
    }

    /// A chat connected: give it a window on the server it was offered on,
    /// whatever is in front now. A window that had a chat gets this one in
    /// place of the old, under a new connection id.
    fn attach_dcc(&mut self, nick: String, server: Option<u64>, chat: DccChat) {
        if let Some(id) = server
            && !self.sessions.iter().any(|session| session.id == id)
        {
            // Its server was closed meanwhile: hang up rather than give it
            // to another one.
            self.note_status(trf(
                "The DCC chat with {0} connected after its server was closed; it was hung up.",
                &[&nick],
            ));
            return;
        }
        let title = format!("DCC {nick}");
        let index = self.on_server(server, |app| app.ensure(Kind::Dcc, &title));
        let window = self.windows[index].uid;
        let opening = trf("DCC chat with {0}", &[&nick]);
        // A chat already in this window is hung up now, so its peer can send
        // nothing more. What it sent and the window has not shown yet is
        // still read, within the frame's budget like any other chat's lines;
        // then its end, and only then does this chat begin.
        let mut previous = None;
        for link in self
            .dcc_links
            .iter_mut()
            .filter(|link| link.window == window)
        {
            link.outgoing = None;
            link.hangup = None;
            previous = Some(link.id);
        }
        let after = match previous {
            Some(id) => Some((id, opening)),
            None => {
                self.windows[index].lines.push(note(opening));
                None
            }
        };
        self.dcc_links.push(DccLink {
            window,
            id: next_window_uid(),
            outgoing: Some(chat.outgoing),
            incoming: chat.incoming,
            hangup: Some(chat.hangup),
            after,
        });
        self.bring_forward(index);
    }

    /// A note about a DCC chat of `server`: in that server's Status, or in
    /// the window in front when that server is gone.
    fn dcc_note(&mut self, server: Option<u64>, text: String) {
        if server.is_none_or(|id| self.sessions.iter().any(|session| session.id == id)) {
            self.on_server(server, |app| app.note_status(text));
        } else {
            self.note_active(text);
        }
    }

    /// Let go of the DCC chat connection of window `uid`, which hangs up.
    fn end_dcc(&mut self, uid: u64) {
        self.dcc_links.retain(|link| link.window != uid);
    }

    fn window_by_uid(&self, uid: u64) -> Option<usize> {
        self.windows.iter().position(|window| window.uid == uid)
    }

    /// Settings to draw with: the Options draft while it is open, so color
    /// and font changes preview at once.
    fn view_settings(&self) -> &Settings {
        self.options_draft
            .as_ref()
            .map(|draft| &draft.settings)
            .unwrap_or(&self.settings)
    }

    fn open_options(&mut self, page: usize) {
        self.options_page = page;
        if self.options_draft.is_none() {
            self.options_draft = Some(OptionsDraft::from(&self.settings, &self.form));
        }
    }

    /// Copy the fields the Options window owns from the draft. Lists that
    /// other windows also edit (ignore, notify) are changed in place instead,
    /// so applying a draft never undoes them.
    fn apply_options(&mut self) {
        let Some(draft) = &self.options_draft else {
            return;
        };
        let flood_before = (
            self.settings.flood_messages,
            self.settings.flood_joins,
            self.settings.flood_window_secs,
            self.settings.flood_ctcp,
        );
        let from = &draft.settings;
        let to = &mut self.settings;
        to.user = from.user.trim().to_string();
        to.realname = from.realname.clone();
        to.proxy = from.proxy;
        to.proxy_kind = from.proxy_kind.trim().to_ascii_lowercase();
        to.proxy_host = from.proxy_host.trim().to_string();
        to.proxy_port = from.proxy_port;
        to.proxy_user = from.proxy_user.clone();
        to.proxy_pass = from.proxy_pass.clone();
        to.sasl_password = from.sasl_password.clone();
        to.sasl_user = from.sasl_user.trim().to_string();
        to.ident = from.ident;
        to.ident_user = from.ident_user.trim().to_string();
        to.show_joins = from.show_joins;
        to.timestamps = from.timestamps;
        to.reconnect = from.reconnect;
        to.flood_messages = from.flood_messages;
        to.flood_joins = from.flood_joins;
        to.flood_window_secs = from.flood_window_secs;
        to.flood_ctcp = from.flood_ctcp;
        to.query_limit = from.query_limit;
        to.active_notices = from.active_notices;
        to.active_ctcp = from.active_ctcp;
        to.active_invites = from.active_invites;
        to.active_whois = from.active_whois;
        to.active_away = from.active_away;
        to.message_window = from.message_window;
        to.ircv3 = from.ircv3;
        to.minimize_to_tray = from.minimize_to_tray;
        to.restore_windows = from.restore_windows;
        to.channels_on_connect = from.channels_on_connect;
        to.keep_history = from.keep_history;
        to.toolbar_hidden = from.toolbar_hidden.clone();
        if !to.keep_history {
            to.history.clear();
        }
        to.nick_color_line = from.nick_color_line;
        to.nick_rank = from.nick_rank;
        to.nick_open = from.nick_open.clone();
        to.nick_close = from.nick_close.clone();
        to.spell_input = from.spell_input;
        to.search_url = from.search_url.trim().to_string();
        to.language = from.language.clone();
        to.title_text = from.title_text.clone();
        to.nicklist_hide_badges = from.nicklist_hide_badges;
        to.tree_hide_hash = from.tree_hide_hash;
        to.auto_away_minutes = from.auto_away_minutes;
        to.auto_away_reason = from.auto_away_reason.clone();
        to.auto_back = from.auto_back;
        to.window_order = from.window_order.clone();
        to.dcc_folder = from.dcc_folder.trim().to_string();
        to.dcc_folder_rules = from
            .dcc_folder_rules
            .iter()
            .filter(|(types, _)| !types.trim().is_empty())
            .cloned()
            .collect();
        to.dcc_auto_accept = from.dcc_auto_accept.clone();
        to.dcc_ignore_types = from.dcc_ignore_types.clone();
        to.dcc_exists = from.dcc_exists.clone();
        to.dcc_keep_awake = from.dcc_keep_awake;
        to.users = from
            .users
            .iter()
            .filter(|(_, mask)| !mask.trim().is_empty())
            .cloned()
            .collect();
        to.default_level = if from.default_level.trim().is_empty() {
            "1".into()
        } else {
            from.default_level.trim().to_string()
        };
        to.tray_blink = from.tray_blink;
        to.multiline_input = from.multiline_input;
        to.command_prefix = if from.command_prefix.trim().is_empty() {
            "/".into()
        } else {
            from.command_prefix.trim().to_string()
        };
        to.separator = if from.separator.trim().is_empty() {
            "|".into()
        } else {
            from.separator.trim().to_string()
        };
        to.dclick_nick = from.dclick_nick.clone();
        to.dclick_channel = from.dclick_channel.clone();
        to.dclick_query = from.dclick_query.clone();
        to.dclick_status = from.dclick_status.clone();
        to.drop_command = from.drop_command.clone();
        to.show_typing = from.show_typing;
        to.send_typing = from.send_typing;
        to.beep = from.beep;
        to.sounds = from.sounds;
        to.font_size = from.font_size;
        to.dictionary = from.dictionary.trim().to_string();
        to.highlights = from.highlights.clone();
        to.highlight_nick = from.highlight_nick;
        to.nick_colors = from.nick_colors.clone();
        to.auto_nick_colors = from.auto_nick_colors;
        to.auto_modes = from.auto_modes.clone();
        to.auto_mode_delay = from.auto_mode_delay;
        let old_ignores = std::mem::replace(&mut to.ignores, from.ignores.clone());
        let new_ignores = to.ignores.clone();
        // Connected servers get the changed ignores below; the pumps keep
        // their own copy for filtering.
        let mut ignore_sync: Vec<String> = old_ignores
            .iter()
            .filter(|rule| !new_ignores.contains(rule))
            .map(|rule| format!("/unignore {}", rule.full_mask()))
            .collect();
        ignore_sync.extend(
            new_ignores
                .iter()
                .filter(|rule| !old_ignores.contains(rule))
                .map(ignore_command),
        );
        to.logging = from.logging;
        to.log_dir = from.log_dir.trim().to_string();
        to.log_keep_codes = from.log_keep_codes;
        to.log_dated = from.log_dated.clone();
        to.log_by_network = from.log_by_network;
        to.log_max_kb = from.log_max_kb;
        to.log_reload = from.log_reload;
        to.perform = from.perform.clone();
        to.theme = from.theme;
        to.palette = from.palette;
        to.alt_nick = from.alt_nick.trim().to_string();
        to.connect_on_start = from.connect_on_start;
        to.nicklist = from.nicklist.clone();
        to.short_joins = from.short_joins;
        to.hide_ping = from.hide_ping;
        to.skip_motd = from.skip_motd;
        to.timestamp_format = if from.timestamp_format.trim().is_empty() {
            "HH:MM".into()
        } else {
            from.timestamp_format.clone()
        };
        to.line_marker = from.line_marker;
        to.rejoin_on_kick = from.rejoin_on_kick;
        to.join_on_invite = from.join_on_invite;
        to.whois_on_query = from.whois_on_query;
        to.quit_message = from.quit_message.clone();
        to.finger_reply = from.finger_reply.clone();
        to.strip_codes = from.strip_codes;
        to.send_queue = from.send_queue;
        to.paste_confirm = from.paste_confirm;
        to.transcript_mono = from.transcript_mono;
        to.line_shading = from.line_shading;
        to.tips = from.tips;
        to.show_network = from.show_network;
        to.connect_timeout = from.connect_timeout;
        to.retry_attempts = from.retry_attempts;
        to.retry_delay = from.retry_delay;
        to.retry_next = from.retry_next;
        to.address_order = from.address_order.clone();
        to.bind_address = from.bind_address.trim().to_string();
        to.bind_fallback = from.bind_fallback;
        to.dcc_port_first = from.dcc_port_first;
        to.dcc_port_last = from.dcc_port_last;
        to.dcc_ip_method = from.dcc_ip_method.clone();
        to.dcc_ip = from.dcc_ip.trim().to_string();
        to.proxy_scope = from.proxy_scope.clone();
        to.proxy_except = from.proxy_except.clone();
        to.ident_system = from.ident_system.trim().to_string();
        to.ident_port = from.ident_port;
        to.ident_connecting_only = from.ident_connecting_only;
        for (event, path) in SOUND_EVENTS.iter().zip(&draft.sounds) {
            to.event_sounds.set(event, path.trim().to_string());
        }
        let form = &draft.form;
        let port = form.port.trim().parse().unwrap_or(to.port);
        to.server = form.server.trim().to_string();
        to.port = port;
        to.tls = form.tls;
        to.nick = form.nick.trim().to_string();
        to.autojoin = form.channel.trim().to_string();
        self.form = ConnectForm {
            port: port.to_string(),
            ..form.clone()
        };
        self.persist();
        self.apply_net_prefs();
        self.sort_windows();
        for command in ignore_sync {
            self.broadcast(command);
        }
        let flood_after = (
            self.settings.flood_messages,
            self.settings.flood_joins,
            self.settings.flood_window_secs,
            self.settings.flood_ctcp,
        );
        if flood_before == flood_after {
            self.note_status(tr("Options saved.").into());
            return;
        }
        let flood = format!(
            "/flood {} {} {} {}",
            self.settings.flood_messages,
            self.settings.flood_joins,
            self.settings.flood_window_secs,
            self.settings.flood_ctcp
        );
        let connected: Vec<u64> = self
            .sessions
            .iter()
            .filter(|session| session.link.is_some())
            .map(|session| session.id)
            .collect();
        for id in connected {
            self.send_to(id, flood.clone());
        }
        self.note_status(tr("Options saved.").into());
    }

    fn options_dialog(&mut self, ctx: &egui::Context) {
        if self.options_draft.is_none() {
            return;
        }
        let mut open = true;
        let mut action = None;
        let max_height = (ctx.content_rect().height() - 80.0).max(240.0);
        egui::Window::new(tr("Options"))
            .collapsible(false)
            .resizable([true, true])
            .default_size([560.0, 420.0])
            .max_height(max_height)
            .default_pos(dialog_pos(ctx, 0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        for (index, label) in [
                            tr("Connect"),
                            tr("Messages"),
                            tr("Logging"),
                            tr("Notify"),
                            tr("Perform"),
                            tr("Colors"),
                            tr("Sounds"),
                            tr("Display"),
                            tr("IRC"),
                            tr("Network"),
                            tr("Highlight"),
                            tr("Ignore"),
                            tr("Nick colors"),
                            tr("Auto modes"),
                            tr("DCC"),
                            tr("Users"),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            if ui
                                .selectable_label(self.options_page == index, label)
                                .clicked()
                            {
                                self.options_page = index;
                            }
                        }
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        // One width for every page, so the window does not
                        // jump in size when another page is picked.
                        ui.set_width(540.0);
                        egui::ScrollArea::vertical()
                            .id_salt("options-page")
                            .max_height(max_height - 90.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| self.options_page_ui(ui));
                        ui.separator();
                        ui.horizontal(|ui| {
                            if ui.button(tr("OK")).clicked() {
                                action = Some(true);
                            }
                            if ui.button(tr("Apply")).clicked() {
                                action = Some(false);
                            }
                            if ui.button(tr("Cancel")).clicked() {
                                self.options_draft = None;
                            }
                        });
                    });
                });
            });
        if let Some(close) = action {
            self.apply_options();
            if close {
                self.options_draft = None;
            }
        }
        if !open {
            self.options_draft = None;
        }
    }

    fn options_page_ui(&mut self, ui: &mut egui::Ui) {
        let Some(draft) = self.options_draft.as_mut() else {
            return;
        };
        let mut immediate: Vec<String> = Vec::new();
        match self.options_page {
            0 => {
                form_grid("options-connect")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        let rows: [(&str, &mut String); 7] = [
                            (tr("Server"), &mut draft.form.server),
                            (tr("Port"), &mut draft.form.port),
                            (tr("Nick"), &mut draft.form.nick),
                            (tr("Alternative nick"), &mut draft.settings.alt_nick),
                            (tr("Username"), &mut draft.settings.user),
                            (tr("Real name"), &mut draft.settings.realname),
                            (tr("Auto-join"), &mut draft.form.channel),
                        ];
                        for (label, value) in rows {
                            ui.label(label);
                            ui.add(field(value).desired_width(260.0));
                            ui.end_row();
                        }
                        ui.label("");
                        ui.checkbox(&mut draft.form.tls, tr("TLS"));
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(
                            &mut draft.settings.connect_on_start,
                            tr("Connect when OpenIRC starts"),
                        );
                        ui.end_row();
                    });
                if draft.form.port.trim().parse::<u16>().is_err() {
                    ui.colored_label(tone_color(Tone::Attention), tr("Port must be 1 to 65535."));
                }
                ui.separator();
                ui.checkbox(&mut draft.settings.proxy, tr("Use a proxy"));
                form_grid("options-proxy")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(tr("Kind"));
                        egui::ComboBox::from_id_salt("proxy-kind")
                            .selected_text(draft.settings.proxy_kind.clone())
                            .show_ui(ui, |ui| {
                                for kind in ["socks5", "socks4", "http"] {
                                    ui.selectable_value(
                                        &mut draft.settings.proxy_kind,
                                        kind.to_string(),
                                        kind,
                                    );
                                }
                            });
                        ui.end_row();
                        ui.label(tr("Host"));
                        ui.add(field(&mut draft.settings.proxy_host).desired_width(260.0));
                        ui.end_row();
                        ui.label(tr("Port"));
                        ui.add(
                            egui::DragValue::new(&mut draft.settings.proxy_port).range(1..=65535),
                        );
                        ui.end_row();
                        ui.label(tr("User"));
                        ui.add(field(&mut draft.settings.proxy_user).desired_width(260.0));
                        ui.end_row();
                        ui.label(tr("Password"));
                        ui.add(
                            field(&mut draft.settings.proxy_pass)
                                .password(true)
                                .desired_width(260.0),
                        );
                        ui.end_row();
                    });
                ui.separator();
                form_grid("options-sasl")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(tr("SASL account"));
                        ui.add(
                            field(&mut draft.settings.sasl_user)
                                .hint_text(tr("blank uses the nick"))
                                .desired_width(260.0),
                        );
                        ui.end_row();
                        ui.label(tr("SASL password"));
                        ui.add(
                            field(&mut draft.settings.sasl_password)
                                .password(true)
                                .desired_width(260.0),
                        );
                        ui.end_row();
                    });
                ui.separator();
                ui.checkbox(&mut draft.settings.ident, tr("Identd on port 113"));
                form_grid("options-ident")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(tr("Ident user ID"));
                        ui.add(
                            field(&mut draft.settings.ident_user)
                                .hint_text(tr("blank uses the IRC username"))
                                .desired_width(260.0),
                        );
                        ui.end_row();
                    });
                ui.separator();
                section(ui, tr("Connecting and retrying"));
                form_grid("options-retry")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(tr("Give up connecting after"));
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut draft.settings.connect_timeout)
                                    .range(5..=300),
                            );
                            ui.label(tr("seconds"));
                        });
                        ui.end_row();
                        ui.label(tr("Retry"));
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut draft.settings.retry_attempts)
                                    .range(0..=1000),
                            );
                            ui.label(tr("times (0 keeps trying)"));
                        });
                        ui.end_row();
                        ui.label(tr("Wait between tries"));
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut draft.settings.retry_delay)
                                    .range(1..=600),
                            );
                            ui.label(tr("seconds"));
                        });
                        ui.end_row();
                    });
                ui.checkbox(
                    &mut draft.settings.retry_next,
                    tr("On a retry, try the next saved server of the same network"),
                );
                ui.checkbox(
                    &mut draft.settings.ircv3,
                    tr("Use IRCv3 capabilities (away and account tracking, server times, typing, TLS upgrade)"),
                );
            }
            1 => {
                ui.checkbox(&mut draft.settings.show_joins, tr("Show joins and parts"));
                ui.checkbox(&mut draft.settings.timestamps, tr("Show timestamps"));
                ui.checkbox(
                    &mut draft.settings.reconnect,
                    tr("Reconnect after a dropped connection"),
                );
                ui.separator();
                ui.label(tr("Flood limits. Zero turns that limit off."));
                form_grid("options-flood")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(tr("Messages from one nick"));
                        ui.add(
                            egui::DragValue::new(&mut draft.settings.flood_messages).range(0..=100),
                        );
                        ui.end_row();
                        ui.label(tr("Joins in one channel"));
                        ui.add(
                            egui::DragValue::new(&mut draft.settings.flood_joins).range(0..=100),
                        );
                        ui.end_row();
                        ui.label(tr("CTCP requests answered"));
                        ui.add(egui::DragValue::new(&mut draft.settings.flood_ctcp).range(0..=100));
                        ui.end_row();
                        ui.label(tr("New queries opened by others"));
                        ui.add(egui::DragValue::new(&mut draft.settings.query_limit).range(0..=100))
                            .on_hover_text(
                                tr("Past this, private messages from new people go to the Messages window."),
                            );
                        ui.end_row();
                        ui.label(tr("Window in seconds"));
                        ui.add(
                            egui::DragValue::new(&mut draft.settings.flood_window_secs)
                                .range(1..=120),
                        );
                        ui.end_row();
                    });
                ui.label(
                    egui::RichText::new(tr(
                        "Ignore rules and highlight rules have their own pages.",
                    ))
                    .small(),
                );
                ui.separator();
                ui.checkbox(
                    &mut draft.settings.beep,
                    tr("Beep when a highlight word arrives"),
                );
                ui.checkbox(
                    &mut draft.settings.sounds,
                    tr("Play a sound for incoming messages"),
                );
                form_grid("options-text")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(tr("Font size"));
                        ui.add(
                            egui::DragValue::new(&mut draft.settings.font_size)
                                .range(9..=28)
                                .speed(0.2),
                        );
                        ui.end_row();
                        ui.label(tr("Dictionary file"));
                        ui.add(
                            field(&mut draft.settings.dictionary)
                                .hint_text(tr("one word per line"))
                                .desired_width(320.0),
                        );
                        ui.end_row();
                        ui.label(tr("Web search address"));
                        ui.add(
                            field(&mut draft.settings.search_url)
                                .hint_text("https://example.com/search?q=%s")
                                .desired_width(320.0),
                        );
                        ui.end_row();
                    });
                ui.checkbox(
                    &mut draft.settings.spell_input,
                    tr("Underline unknown words in the input line as you type"),
                );
                if !draft.settings.dictionary.trim().is_empty()
                    && !std::path::Path::new(draft.settings.dictionary.trim()).is_file()
                {
                    ui.colored_label(tone_color(Tone::Attention), tr("That file does not exist."));
                }
            }
            2 => {
                ui.checkbox(&mut draft.settings.logging, tr("Log open windows"));
                ui.label(tr(
                    "Each window is logged to its own file with a date and time on every line.",
                ));
                ui.checkbox(
                    &mut draft.settings.log_by_network,
                    tr("Keep each network's logs in its own folder"),
                );
                ui.checkbox(
                    &mut draft.settings.log_keep_codes,
                    tr("Keep color and style codes"),
                );
                ui.add_space(4.0);
                form_grid("options-logging")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(tr("Log folder"));
                        ui.add(field(&mut draft.settings.log_dir).desired_width(320.0));
                        ui.end_row();
                        ui.label(tr("New file every"));
                        egui::ComboBox::from_id_salt("log-dated")
                            .selected_text(match draft.settings.log_dated.as_str() {
                                "day" => tr("day"),
                                "month" => tr("month"),
                                _ => tr("never (one file)"),
                            })
                            .show_ui(ui, |ui| {
                                for (key, label) in [
                                    ("none", tr("never (one file)")),
                                    ("day", tr("day")),
                                    ("month", tr("month")),
                                ] {
                                    ui.selectable_value(
                                        &mut draft.settings.log_dated,
                                        key.into(),
                                        label,
                                    );
                                }
                            });
                        ui.end_row();
                        ui.label(tr("Trim logs larger than"));
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut draft.settings.log_max_kb)
                                    .range(0..=1_048_576),
                            );
                            ui.label(tr("KiB (0 never trims)"));
                        });
                        ui.end_row();
                        ui.label(tr("Lines shown on reopening"));
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut draft.settings.log_reload).range(0..=500),
                            );
                            ui.label(tr("from the log when a channel or query opens"));
                        });
                        ui.end_row();
                    });
            }
            3 => {
                ui.label(tr(
                    "Nicks to watch with MONITOR after connect (changes apply at once)",
                ));
                let mut remove = None;
                egui::Grid::new("notify-list")
                    .num_columns(2)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        for (index, nick) in self.settings.notify.iter().enumerate() {
                            ui.label(nick);
                            if ui.button(tr("Remove")).clicked() {
                                remove = Some(index);
                            }
                            ui.end_row();
                        }
                    });
                if let Some(index) = remove {
                    self.settings.notify.remove(index);
                    immediate.push(String::new());
                }
                ui.horizontal(|ui| {
                    ui.add(field(&mut self.notify_nick).desired_width(180.0));
                    if ui.button(tr("Add")).clicked() {
                        let nick = self.notify_nick.trim().to_string();
                        if !nick.is_empty() {
                            self.settings.notify.push(nick);
                            self.notify_nick.clear();
                            immediate.push(String::new());
                        }
                    }
                });
            }
            4 => {
                ui.label(tr("Commands run after this server finishes registering"));
                form_grid("options-perform")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(tr("Server"));
                        ui.add(
                            field(&mut self.perform_server)
                                .hint_text(tr("blank for every server"))
                                .desired_width(260.0),
                        );
                        ui.end_row();
                        ui.label(tr("Command"));
                        ui.add(
                            field(&mut self.perform_command)
                                .hint_text("/msg NickServ ...")
                                .desired_width(260.0),
                        );
                        ui.end_row();
                    });
                if ui.button(tr("Add")).clicked() {
                    let command = self.perform_command.trim().to_string();
                    if !command.is_empty() {
                        draft.settings.perform.push(Perform {
                            server: self.perform_server.trim().to_string(),
                            command,
                        });
                        self.perform_command.clear();
                    }
                }
                let mut remove = None;
                egui::Grid::new("perform-list")
                    .num_columns(3)
                    .spacing([10.0, 4.0])
                    .striped(true)
                    .show(ui, |ui| {
                        for (index, item) in draft.settings.perform.iter().enumerate() {
                            let server = if item.server.is_empty() {
                                "every server".to_string()
                            } else {
                                item.server.clone()
                            };
                            ui.label(egui::RichText::new(server).color(muted_on_face()));
                            ui.label(&item.command);
                            if ui.button(tr("Remove")).clicked() {
                                remove = Some(index);
                            }
                            ui.end_row();
                        }
                    });
                if let Some(index) = remove {
                    draft.settings.perform.remove(index);
                }
            }
            5 => {
                theme_editor(ui, &mut draft.settings.theme);
                ui.add_space(8.0);
                palette_editor(ui, &mut draft.settings.palette);
                ui.add_space(6.0);
                ui.checkbox(
                    &mut draft.settings.transcript_mono,
                    tr("Monospace font in the transcript"),
                );
            }
            7 => {
                section(ui, tr("Language"));
                form_grid("options-language")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(tr("Menus and dialogs"));
                        let languages = crate::i18n::available();
                        let system = tr("Same as the computer");
                        let shown = languages
                            .iter()
                            .find(|(code, _)| *code == draft.settings.language)
                            .map_or(system, |(_, name)| name.as_str());
                        egui::ComboBox::from_id_salt("language")
                            .selected_text(shown)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut draft.settings.language,
                                    String::new(),
                                    system,
                                );
                                for (code, name) in &languages {
                                    ui.selectable_value(
                                        &mut draft.settings.language,
                                        code.clone(),
                                        name,
                                    );
                                }
                            });
                        ui.end_row();
                    });
                ui.label(
                    egui::RichText::new(trf(
                        "More languages: put a translation file in {0}",
                        &[&settings::config_dir().join("lang").display()],
                    ))
                    .small()
                    .color(muted_on_face()),
                );
                ui.add_space(6.0);
                section(ui, tr("Nickname list"));
                ui.horizontal(|ui| {
                    for (label, value) in [
                        (tr("Right"), "right"),
                        (tr("Left"), "left"),
                        (tr("Hidden"), "hidden"),
                    ] {
                        ui.radio_value(&mut draft.settings.nicklist, value.to_string(), label);
                    }
                });
                ui.add_space(6.0);
                section(ui, tr("Transcript"));
                ui.checkbox(&mut draft.settings.timestamps, tr("Show timestamps"));
                form_grid("options-stamp")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(tr("Timestamp format"));
                        ui.horizontal(|ui| {
                            ui.add(
                                field(&mut draft.settings.timestamp_format).desired_width(120.0),
                            );
                            ui.label(
                                egui::RichText::new(trf(
                                    "shows {0}",
                                    &[&format_stamp(
                                        &draft.settings.timestamp_format,
                                        crate::clock::now(),
                                    )],
                                ))
                                .color(muted_on_face()),
                            );
                        });
                        ui.end_row();
                    });
                ui.label(
                    egui::RichText::new(tr("HH, MM, and SS are replaced; for example [HH:MM:SS]."))
                        .small(),
                );
                ui.checkbox(
                    &mut draft.settings.line_marker,
                    tr("Mark the last line read when leaving a window"),
                );
                ui.checkbox(
                    &mut draft.settings.line_shading,
                    tr("Shade every other line"),
                );
                ui.checkbox(&mut draft.settings.transcript_mono, tr("Monospace font"));
                ui.checkbox(
                    &mut draft.settings.nick_color_line,
                    tr("Color whole messages in the speaker's nick color"),
                );
                ui.checkbox(
                    &mut draft.settings.nick_rank,
                    tr("Show the speaker's @ or + before the nick in messages"),
                );
                form_grid("options-brackets")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(tr("Text around nicks"));
                        ui.horizontal(|ui| {
                            ui.add(field(&mut draft.settings.nick_open).desired_width(36.0));
                            ui.label(egui::RichText::new(tr("nick")).color(muted_on_face()));
                            ui.add(field(&mut draft.settings.nick_close).desired_width(36.0));
                        });
                        ui.end_row();
                    });
                ui.checkbox(
                    &mut draft.settings.nicklist_hide_badges,
                    tr("Leave out @ and + badges in the nick list"),
                );
                ui.checkbox(
                    &mut draft.settings.tree_hide_hash,
                    tr("Leave out # before channel names in the treebar"),
                );
                ui.add_space(6.0);
                form_grid("options-order")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(tr("Order windows"));
                        egui::ComboBox::from_id_salt("window-order")
                            .selected_text(if draft.settings.window_order == "name" {
                                tr("by kind and name")
                            } else {
                                tr("as they open")
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut draft.settings.window_order,
                                    "opened".into(),
                                    tr("as they open"),
                                );
                                ui.selectable_value(
                                    &mut draft.settings.window_order,
                                    "name".into(),
                                    tr("by kind and name"),
                                );
                            });
                        ui.end_row();
                    });
                ui.add_space(6.0);
                section(ui, tr("Notices and titles"));
                ui.checkbox(
                    &mut draft.settings.tips,
                    tr("Show notices for highlights, private messages, and invites"),
                );
                ui.checkbox(
                    &mut draft.settings.show_network,
                    tr("Show the network name in the title bar"),
                );
                form_grid("options-title")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(tr("Title bar text"));
                        ui.add(
                            field(&mut draft.settings.title_text)
                                .hint_text("$me on $network — $window")
                                .desired_width(260.0),
                        );
                        ui.end_row();
                    });
                ui.add_space(6.0);
                section(ui, tr("Toolbar buttons"));
                egui::Grid::new("toolbar-choices")
                    .num_columns(3)
                    .spacing([16.0, 2.0])
                    .show(ui, |ui| {
                        for (index, (icon, label)) in chrome::TOOLBAR_CHOICES.iter().enumerate() {
                            let name = format!("{icon:?}");
                            let mut on = !draft.settings.toolbar_hidden.contains(&name);
                            if ui.checkbox(&mut on, tr(label)).changed() {
                                draft.settings.toolbar_hidden.retain(|item| item != &name);
                                if !on {
                                    draft.settings.toolbar_hidden.push(name);
                                }
                            }
                            if index % 3 == 2 {
                                ui.end_row();
                            }
                        }
                    });
                ui.add_space(6.0);
                section(ui, tr("Tray"));
                ui.checkbox(
                    &mut draft.settings.minimize_to_tray,
                    tr("Minimize to the tray icon instead of the taskbar"),
                );
                ui.checkbox(
                    &mut draft.settings.tray_blink,
                    tr("Blink the tray icon while a highlight or private message waits"),
                );
                if self.tray.is_none() {
                    ui.label(
                        egui::RichText::new(tr(
                            "No tray icon on this desktop, so these have no effect.",
                        ))
                        .small()
                        .color(muted_on_face()),
                    );
                }
            }
            14 => Self::dcc_options(ui, &mut draft.settings),
            15 => users_page(ui, &mut draft.settings),
            10 => rules::highlight_page(ui, &mut draft.settings, &mut draft.picks),
            11 => rules::ignore_page(ui, &mut draft.settings, &mut draft.picks),
            12 => rules::nick_color_page(ui, &mut draft.settings, &mut draft.picks),
            13 => rules::auto_mode_page(ui, &mut draft.settings, &mut draft.picks),
            9 => {
                section(ui, tr("Connecting"));
                form_grid("options-network")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(tr("Address order"));
                        egui::ComboBox::from_id_salt("address-order")
                            .selected_text(match draft.settings.address_order.as_str() {
                                "ipv4" => tr("IPv4 first"),
                                "ipv6" => tr("IPv6 first"),
                                _ => tr("As the system returns them"),
                            })
                            .show_ui(ui, |ui| {
                                for (order, label) in [
                                    (
                                        crate::net::AddressOrder::System,
                                        tr("As the system returns them"),
                                    ),
                                    (crate::net::AddressOrder::Ipv4First, tr("IPv4 first")),
                                    (crate::net::AddressOrder::Ipv6First, tr("IPv6 first")),
                                ] {
                                    ui.selectable_value(
                                        &mut draft.settings.address_order,
                                        order.key().to_string(),
                                        label,
                                    );
                                }
                            });
                        ui.end_row();
                        ui.label(tr("Connect from address"));
                        ui.add(
                            field(&mut draft.settings.bind_address)
                                .hint_text(tr("blank lets the system choose"))
                                .desired_width(200.0),
                        );
                        ui.end_row();
                    });
                ui.checkbox(
                    &mut draft.settings.bind_fallback,
                    tr("If that address cannot be used, connect anyway"),
                );
                if !draft.settings.bind_address.trim().is_empty()
                    && draft
                        .settings
                        .bind_address
                        .trim()
                        .parse::<std::net::IpAddr>()
                        .is_err()
                {
                    ui.colored_label(
                        tone_color(Tone::Attention),
                        tr("That is not an IP address."),
                    );
                }
                ui.separator();
                section(ui, tr("DCC"));
                form_grid("options-dcc-net")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(tr("Listen on ports"));
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut draft.settings.dcc_port_first).range(0..=65535));
                            ui.label(tr("to"));
                            ui.add(egui::DragValue::new(&mut draft.settings.dcc_port_last).range(0..=65535));
                        });
                        ui.end_row();
                        ui.label("");
                        ui.label(egui::RichText::new(tr("0 uses any free port. Forward the range on your router to receive files.")).small());
                        ui.end_row();
                        ui.label(tr("My address in offers"));
                        ui.vertical(|ui| {
                            for (value, label) in [
                                ("local", tr("This computer's address")),
                                ("server", tr("The address the IRC server sees")),
                                ("manual", tr("This address:")),
                            ] {
                                ui.radio_value(&mut draft.settings.dcc_ip_method, value.to_string(), label);
                            }
                            ui.add_enabled(
                                draft.settings.dcc_ip_method == "manual",
                                field(&mut draft.settings.dcc_ip).hint_text("203.0.113.5").desired_width(160.0),
                            );
                        });
                        ui.end_row();
                    });
                ui.separator();
                section(ui, tr("Proxy"));
                form_grid("options-proxy-use")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(tr("Use the proxy for"));
                        ui.horizontal(|ui| {
                            for (scope, label) in [
                                (crate::net::ProxyScope::Servers, tr("servers")),
                                (crate::net::ProxyScope::Dcc, "DCC"),
                                (crate::net::ProxyScope::Both, tr("both")),
                            ] {
                                ui.radio_value(
                                    &mut draft.settings.proxy_scope,
                                    scope.key().to_string(),
                                    label,
                                );
                            }
                        });
                        ui.end_row();
                        ui.label(tr("Except these hosts"));
                        ui.add(
                            field(&mut draft.settings.proxy_except)
                                .hint_text(tr("*.lan, 192.168.*"))
                                .desired_width(240.0),
                        );
                        ui.end_row();
                    });
                ui.label(
                    egui::RichText::new(tr("The proxy itself is set on the Connect page.")).small(),
                );
                ui.separator();
                section(ui, tr("Identd"));
                form_grid("options-identd")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(tr("System name"));
                        ui.add(field(&mut draft.settings.ident_system).desired_width(100.0));
                        ui.end_row();
                        ui.label(tr("Port"));
                        ui.add(
                            egui::DragValue::new(&mut draft.settings.ident_port).range(1..=65535),
                        );
                        ui.end_row();
                    });
                ui.checkbox(
                    &mut draft.settings.ident_connecting_only,
                    tr("Run identd only while connecting"),
                );
            }
            8 => {
                section(ui, tr("Events"));
                ui.checkbox(
                    &mut draft.settings.short_joins,
                    tr("Short join, part, and quit lines"),
                );
                ui.checkbox(&mut draft.settings.hide_ping, tr("Hide PING and PONG"));
                ui.checkbox(
                    &mut draft.settings.skip_motd,
                    tr("Skip the message of the day"),
                );
                ui.checkbox(
                    &mut draft.settings.strip_codes,
                    tr("Remove colors and styles from incoming messages"),
                );
                ui.add_space(6.0);
                section(ui, tr("Channels"));
                ui.checkbox(
                    &mut draft.settings.channels_on_connect,
                    tr("Show the channels list when a server connects"),
                );
                ui.checkbox(
                    &mut draft.settings.restore_windows,
                    tr("Reopen the last session's channels and queries when connecting"),
                );
                ui.checkbox(
                    &mut draft.settings.rejoin_on_kick,
                    tr("Join again after being kicked"),
                );
                ui.checkbox(
                    &mut draft.settings.join_on_invite,
                    tr("Join a channel when invited"),
                );
                ui.checkbox(
                    &mut draft.settings.whois_on_query,
                    tr("Look up (whois) people who open a query"),
                );
                ui.checkbox(
                    &mut draft.settings.message_window,
                    tr("Collect private messages in one Messages window (open queries still get theirs)"),
                );
                ui.add_space(6.0);
                section(ui, tr("Show in the active window"));
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut draft.settings.active_notices, tr("Notices"));
                    ui.checkbox(&mut draft.settings.active_ctcp, tr("CTCP"));
                    ui.checkbox(&mut draft.settings.active_invites, tr("Invites"));
                    ui.checkbox(&mut draft.settings.active_whois, tr("Whois"));
                    ui.checkbox(&mut draft.settings.active_away, tr("Away"));
                });
                ui.label(
                    egui::RichText::new(tr("The rest goes to the server's status window.")).small(),
                );
                ui.add_space(6.0);
                section(ui, tr("Away"));
                form_grid("options-away")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(tr("Set away when idle for"));
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut draft.settings.auto_away_minutes)
                                    .range(0..=1440),
                            );
                            ui.label(tr("minutes (0 never)"));
                        });
                        ui.end_row();
                        ui.label(tr("Away reason"));
                        ui.add(field(&mut draft.settings.auto_away_reason).desired_width(260.0));
                        ui.end_row();
                    });
                ui.checkbox(
                    &mut draft.settings.auto_back,
                    tr("Come back when I type again"),
                );
                ui.checkbox(
                    &mut draft.settings.keep_history,
                    tr("Keep typed lines for the next session (never ones with passwords)"),
                );
                ui.add_space(6.0);
                section(ui, tr("Input"));
                ui.checkbox(
                    &mut draft.settings.multiline_input,
                    tr("Multi-line input (Enter sends, Shift+Enter starts a new line)"),
                );
                form_grid("options-input")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(tr("Command prefix"));
                        ui.add(field(&mut draft.settings.command_prefix).desired_width(40.0))
                            .on_hover_text(tr("Starts a typed command; / always works too."));
                        ui.end_row();
                        ui.label(tr("Command separator"));
                        ui.add(field(&mut draft.settings.separator).desired_width(40.0))
                            .on_hover_text(tr(
                                "Splits commands in aliases, popups, events, and dialog buttons.",
                            ));
                        ui.end_row();
                    });
                ui.add_space(6.0);
                section(ui, tr("Double-click and drop"));
                form_grid("options-dclick")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        for (label, value) in [
                            (tr("Nick in the nick list"), &mut draft.settings.dclick_nick),
                            (tr("Channel window"), &mut draft.settings.dclick_channel),
                            (tr("Query window"), &mut draft.settings.dclick_query),
                            (tr("Status window"), &mut draft.settings.dclick_status),
                            (
                                tr("File dropped on a window"),
                                &mut draft.settings.drop_command,
                            ),
                        ] {
                            ui.label(label);
                            ui.add(field(value).desired_width(220.0).hint_text(tr("nothing")));
                            ui.end_row();
                        }
                    });
                ui.label(
                    egui::RichText::new(tr(
                        "$nick is the nick, $chan the channel, $file the dropped file.",
                    ))
                    .small(),
                );
                ui.add_space(6.0);
                section(ui, tr("Typing"));
                ui.checkbox(&mut draft.settings.show_typing, tr("Show who is typing"));
                ui.checkbox(
                    &mut draft.settings.send_typing,
                    tr("Let others see when I am typing"),
                );
                ui.add_space(6.0);
                section(ui, tr("Sending"));
                ui.checkbox(
                    &mut draft.settings.send_queue,
                    tr("Pace my messages so the server does not disconnect me for flooding"),
                );
                ui.horizontal(|ui| {
                    ui.label(tr("Ask before pasting"));
                    ui.add(egui::DragValue::new(&mut draft.settings.paste_confirm).range(0..=100));
                    ui.label(tr("lines or more (0 never asks)"));
                });
                ui.add_space(6.0);
                form_grid("options-replies")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(tr("Quit message"));
                        ui.add(
                            field(&mut draft.settings.quit_message)
                                .hint_text(tr("OpenIRC version"))
                                .desired_width(260.0),
                        );
                        ui.end_row();
                        ui.label(tr("Finger reply"));
                        ui.add(
                            field(&mut draft.settings.finger_reply)
                                .hint_text(tr("your real name"))
                                .desired_width(260.0),
                        );
                        ui.end_row();
                    });
            }
            6 => {
                ui.label(tr(
                    "WAV file for each event. Turn on sounds under Messages.",
                ));
                let mut play = None;
                form_grid("options-sounds")
                    .num_columns(3)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        for (index, event) in SOUND_EVENTS.iter().enumerate() {
                            ui.label(sound_event_name(event));
                            ui.add(field(&mut draft.sounds[index]).desired_width(300.0));
                            if ui.button(tr("Play")).clicked() {
                                play = Some(index);
                            }
                            ui.end_row();
                        }
                    });
                if let Some(index) = play {
                    let path = draft.sounds[index].trim().to_string();
                    draft.sound_note = match std::fs::read(&path) {
                        _ if path.is_empty() => {
                            tr("No file chosen; the built-in beep is used.").into()
                        }
                        Ok(bytes) if crate::wav::decode(&bytes).is_some() => {
                            crate::beep::play_file(&path);
                            trf("Playing {0}.", &[&path])
                        }
                        Ok(_) => trf("{0} is not a PCM WAV file.", &[&path]),
                        Err(err) => trf("Cannot read {0}: {1}", &[&path, &err]),
                    };
                }
                if !draft.sound_note.is_empty() {
                    ui.label(&draft.sound_note);
                }
            }
            _ => {}
        }
        if !immediate.is_empty() {
            self.persist();
            self.sync_notify();
            for command in immediate.into_iter().filter(|command| !command.is_empty()) {
                self.send_user(command);
            }
        }
    }

    fn favorites_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_favorites {
            return;
        }
        let mut open = true;
        let mut connect_index = None;
        egui::Window::new(tr("Favorites"))
            .default_pos(dialog_pos(ctx, 5))
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                let mut remove = None;
                if self.settings.favorites.is_empty() {
                    ui.label(egui::RichText::new(tr("No favorites yet.")).color(muted_on_face()));
                } else {
                    // One row per favorite; the buttons stay in their own
                    // columns whatever the names' lengths.
                    egui::Grid::new("favorites")
                        .num_columns(4)
                        .spacing([10.0, 4.0])
                        .striped(true)
                        .show(ui, |ui| {
                            for (index, favorite) in self.settings.favorites.iter().enumerate() {
                                ui.label(egui::RichText::new(&favorite.name).strong());
                                ui.label(format!(
                                    "{}:{}{}",
                                    favorite.server,
                                    favorite.port,
                                    if favorite.tls { " (TLS)" } else { "" }
                                ));
                                if ui.button(tr("Connect")).clicked() {
                                    connect_index = Some(index);
                                }
                                if ui.button(tr("Remove")).clicked() {
                                    remove = Some(index);
                                }
                                ui.end_row();
                            }
                        });
                }
                ui.separator();
                form_grid("favorite-form")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(tr("Name"));
                        ui.add(
                            field(&mut self.favorite_name)
                                .hint_text(tr("blank uses the server name"))
                                .desired_width(220.0),
                        );
                        ui.end_row();
                    });
                if ui.button(tr("Save current server")).clicked() {
                    let name = if self.favorite_name.trim().is_empty() {
                        self.form.server.clone()
                    } else {
                        self.favorite_name.trim().to_string()
                    };
                    self.settings.favorites.push(Favorite {
                        name,
                        server: self.form.server.clone(),
                        port: self.form.port.parse().unwrap_or(6667),
                        tls: self.form.tls,
                        channels: self.form.channel.clone(),
                    });
                    self.favorite_name.clear();
                    let _ = self.settings.save();
                }
                if let Some(index) = remove {
                    self.settings.favorites.remove(index);
                    let _ = self.settings.save();
                }
            });
        if let Some(index) = connect_index
            && let Some(favorite) = self.settings.favorites.get(index).cloned()
        {
            self.form.server = favorite.server;
            self.form.port = favorite.port.to_string();
            self.form.tls = favorite.tls;
            self.form.channel = favorite.channels;
            self.show_favorites = false;
            self.connect();
        }
        if !open {
            self.show_favorites = false;
        }
    }

    fn address_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_address {
            return;
        }
        let mut open = true;
        let mut changed = false;
        egui::Window::new(tr("Address book"))
            .default_pos(dialog_pos(ctx, 6))
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(tr("You are told when these nicks come online or leave."));
                ui.horizontal(|ui| {
                    ui.add(field(&mut self.notify_nick));
                    if ui.button(tr("Add")).clicked() {
                        let nick = self.notify_nick.trim().to_string();
                        if !nick.is_empty() {
                            self.settings.notify.push(nick);
                            self.notify_nick.clear();
                            changed = true;
                        }
                    }
                });
                let nicks = self.settings.notify.clone();
                let online: Vec<String> = self
                    .sessions
                    .iter()
                    .flat_map(|session| session.notify_online.iter().cloned())
                    .collect();
                let mut remove = None;
                let mut query = None;
                egui::Grid::new("address-list")
                    .num_columns(3)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        for (index, nick) in nicks.iter().enumerate() {
                            let here = online.iter().any(|item| same_nick(item, nick));
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(10.0, 14.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().circle_filled(
                                    rect.center(),
                                    4.0,
                                    if here {
                                        egui::Color32::from_rgb(40, 170, 80)
                                    } else {
                                        egui::Color32::from_gray(150)
                                    },
                                );
                                ui.label(nick).on_hover_text(if here {
                                    "Online"
                                } else {
                                    tr("Not seen online")
                                });
                            });
                            if ui.button(tr("Query")).clicked() {
                                query = Some(nick.clone());
                            }
                            if ui.button(tr("Remove")).clicked() {
                                remove = Some(index);
                            }
                            ui.end_row();
                        }
                    });
                if let Some(nick) = query {
                    let index = self.ensure(Kind::Query, &nick);
                    self.activate(index);
                }
                if let Some(index) = remove {
                    self.settings.notify.remove(index);
                    changed = true;
                }
            });
        if changed {
            self.persist();
            self.sync_notify();
        }
        if !open {
            self.show_address = false;
        }
    }

    fn dcc_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_dcc {
            return;
        }
        let mut open = true;
        let mut accepted = None;
        let mut accepted_file = None;
        let mut offer_file = false;
        let mut offer_chat = false;
        egui::Window::new(tr("DCC"))
            .default_pos(dialog_pos(ctx, 7))
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                form_grid("dcc-offer")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(tr("Nick"));
                        ui.add(field(&mut self.dcc_nick).desired_width(240.0));
                        ui.end_row();
                        ui.label(tr("File"));
                        ui.add(
                            field(&mut self.send_path)
                                .hint_text(tr("path of the file to send"))
                                .desired_width(240.0),
                        );
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(
                            &mut self.dcc_passive,
                            tr("Passive send (the other side listens)"),
                        );
                        ui.end_row();
                    });
                ui.horizontal(|ui| {
                    if ui.button(tr("Offer chat")).clicked() {
                        offer_chat = true;
                    }
                    if ui.button(tr("Offer file")).clicked() {
                        offer_file = true;
                    }
                });
                ui.separator();
                section(ui, tr("Incoming chat"));
                if self.offers.is_empty() {
                    ui.label(egui::RichText::new(tr("None")).color(muted_on_face()));
                }
                egui::Grid::new("dcc-chats")
                    .num_columns(3)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        for (index, offer) in self.offers.iter().enumerate() {
                            ui.label(egui::RichText::new(&offer.nick).strong());
                            ui.label(format!("{}:{}", offer.host, offer.port));
                            if ui.button(tr("Accept")).clicked() {
                                accepted = Some(index);
                            }
                            ui.end_row();
                        }
                    });
                ui.separator();
                section(ui, tr("Incoming files"));
                if self.send_offers.is_empty() {
                    ui.label(egui::RichText::new(tr("None")).color(muted_on_face()));
                }
                egui::Grid::new("dcc-files")
                    .num_columns(4)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        for (index, queued) in self.send_offers.iter().enumerate() {
                            ui.label(egui::RichText::new(&queued.from).strong());
                            ui.label(&queued.offer.filename);
                            ui.label(
                                queued
                                    .offer
                                    .size
                                    .map(transfers::human_bytes)
                                    .unwrap_or_else(|| "size not given".into()),
                            );
                            if ui.button(tr("Accept...")).clicked() {
                                accepted_file = Some(index);
                            }
                            ui.end_row();
                        }
                    });
            });
        if offer_file {
            self.offer_send();
        }
        if offer_chat {
            self.offer_chat();
        }
        if let Some(index) = accepted {
            self.accept_chat_offer(index);
        }
        if let Some(index) = accepted_file {
            self.review_offer(index);
        }
        if !open {
            self.show_dcc = false;
        }
    }

    /// Connect to the chat offer at `index` in the DCC window's list. It
    /// stays with the server the offer came on.
    fn accept_chat_offer(&mut self, index: usize) {
        if index >= self.offers.len() {
            return;
        }
        let offer = self.offers.remove(index);
        let nick = offer.nick.clone();
        let host = offer.host.clone();
        let port = offer.port;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result = dcc::connect_chat(&host, port).map_err(|err| err.to_string());
            let _ = tx.send(result);
        });
        self.dcc_connecting.push((nick, offer.server, rx));
        let text = trf(
            "Connecting DCC chat to {0}:{1}",
            &[&offer.host, &offer.port],
        );
        self.on_server(offer.server, |app| app.note_status(text));
    }

    fn offer_chat(&mut self) {
        let nick = self.dcc_nick.trim().to_string();
        if nick.is_empty() {
            return;
        }
        match dcc::listen_chat() {
            Ok(pending) => {
                let ip = dcc::encode_dcc_ip(crate::net::dcc_ip());
                self.send_user(format!(
                    "/raw PRIVMSG {nick} :\u{1}DCC CHAT chat {ip} {}\u{1}",
                    pending.port
                ));
                self.outgoing_dcc.push((nick, self.scope(), pending));
                self.note_status(tr("Offered DCC chat. Waiting for the other side.").into());
            }
            Err(err) => self.note_status(trf("DCC listen failed: {0}", &[&err])),
        }
    }

    fn poll_tray(&mut self, ctx: &egui::Context) {
        if self.tray.is_none() {
            // Try once. A platform or desktop without a tray will not grow one
            // later, and retrying every frame only repeats the failed setup.
            if self.tray_noted {
                return;
            }
            self.tray_noted = true;
            match crate::tray::install() {
                Ok(tray) => self.tray = Some(tray),
                Err(err) => {
                    self.note_status(trf("Tray icon: {0}", &[&err]));
                    return;
                }
            }
        }
        let action = self.tray.as_ref().and_then(|tray| tray.poll());
        match action {
            Some(crate::tray::Action::Show) => {
                self.tray_hidden = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            Some(crate::tray::Action::Quit) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            None => {}
        }
        let minimized = ctx.input(|input| input.viewport().minimized.unwrap_or(false));
        let in_tray = self.tray.as_ref().is_some_and(crate::tray::Tray::shown);
        match tray_step(
            self.tray_hidden,
            self.settings.minimize_to_tray,
            minimized,
            in_tray,
        ) {
            Some(TrayStep::Hide) => {
                // Out of the taskbar; the tray icon's Show brings it back.
                // The app logic keeps running while the window is hidden.
                self.tray_hidden = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
            Some(TrayStep::Show) => {
                self.tray_hidden = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                self.note_status(
                    tr("The tray went away, so the window is back in the taskbar.").into(),
                );
            }
            None => {}
        }
        let waiting = self
            .windows
            .iter()
            .any(|window| window.activity == Activity::Highlight);
        let blink = Duration::from_millis(600);
        if self.tray_blinked.elapsed() >= blink {
            self.tray_blinked = std::time::Instant::now();
            let animate = self.settings.tray_blink;
            if let Some(tray) = self.tray.as_mut() {
                tray.set_attention(waiting, animate);
            }
        }
        if waiting || self.tray_hidden {
            ctx.request_repaint_after(blink);
        }
    }

    fn dialog_editor(&mut self, ctx: &egui::Context) {
        if !self.show_dialog_editor {
            return;
        }
        let mut open = true;
        let mut add_kind = None;
        let mut apply = false;
        let mut created = None;
        let mut delete = false;
        let mut moved = false;
        egui::Window::new(tr("Dialog editor"))
            .default_size([640.0, 460.0])
            .default_pos(dialog_pos(ctx, 2))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr("Name"));
                    ui.add(field(&mut self.new_dialog_name).desired_width(160.0));
                    let name = self.new_dialog_name.trim().to_string();
                    let taken = self
                        .settings
                        .dialogs
                        .iter()
                        .any(|dialog| dialog.name == name);
                    let valid = !name.is_empty() && !name.contains(char::is_whitespace);
                    if ui
                        .add_enabled(valid && !taken, egui::Button::new(tr("New")))
                        .on_disabled_hover_text(tr("Type a new name without spaces"))
                        .clicked()
                    {
                        created = Some(name);
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if self.settings.dialogs.is_empty() {
                        ui.label(tr("No dialogs yet."));
                    }
                    let names: Vec<String> = self
                        .settings
                        .dialogs
                        .iter()
                        .map(|dialog| dialog.name.clone())
                        .collect();
                    for name in names {
                        if ui
                            .selectable_label(self.editing_dialog == name, &name)
                            .clicked()
                        {
                            self.editing_dialog = name.clone();
                            if let Some(dialog) = self
                                .settings
                                .dialogs
                                .iter()
                                .find(|dialog| dialog.name == name)
                            {
                                self.dialog_script = crate::dialog::emit(dialog);
                            }
                            self.dialog_note.clear();
                        }
                    }
                });
                let selected = self
                    .settings
                    .dialogs
                    .iter()
                    .any(|dialog| dialog.name == self.editing_dialog);
                ui.label(tr("Dialog text"));
                ui.add(
                    field_multi(&mut self.dialog_script)
                        .font(egui::TextStyle::Monospace)
                        .code_editor()
                        .desired_rows(7)
                        .desired_width(f32::INFINITY),
                );
                ui.horizontal(|ui| {
                    if ui.button(tr("Apply text")).clicked() {
                        apply = true;
                    }
                    if ui
                        .add_enabled(selected, egui::Button::new(tr("Delete dialog")))
                        .clicked()
                    {
                        delete = true;
                    }
                    if ui
                        .add_enabled(selected, egui::Button::new(tr("Open")))
                        .on_hover_text(tr("Same as /dialog name"))
                        .clicked()
                    {
                        self.open_dialog = Some(self.editing_dialog.clone());
                    }
                });
                if !self.dialog_note.is_empty() {
                    ui.label(&self.dialog_note);
                }
                ui.horizontal(|ui| {
                    for (kind, label) in [
                        ("text", tr("Add text")),
                        ("edit", tr("Add edit box")),
                        ("check", tr("Add check box")),
                        ("button", tr("Add button")),
                    ] {
                        if ui
                            .add_enabled(selected, egui::Button::new(label))
                            .on_disabled_hover_text(tr("Pick or create a dialog first"))
                            .clicked()
                        {
                            add_kind = Some(kind.to_string());
                        }
                    }
                });
                let name = self.editing_dialog.clone();
                if let Some(dialog) = self
                    .settings
                    .dialogs
                    .iter_mut()
                    .find(|dialog| dialog.name == name)
                {
                    ui.label(tr(
                        "Drag controls to place them. Button commands may use $did(id).",
                    ));
                    let (canvas, _) = ui.allocate_exact_size(
                        egui::vec2(dialog.width.max(40) as f32, dialog.height.max(40) as f32),
                        egui::Sense::hover(),
                    );
                    let origin = canvas.min;
                    ui.painter().rect_filled(canvas, 0.0, color_face());
                    ui.painter().rect_stroke(
                        canvas,
                        0.0,
                        egui::Stroke::new(1.0, color_shadow()),
                        egui::StrokeKind::Inside,
                    );
                    let painter = ui.painter().with_clip_rect(canvas);
                    for control in &mut dialog.controls {
                        let size = egui::vec2(control.w.max(8) as f32, control.h.max(8) as f32);
                        let rect = egui::Rect::from_min_size(
                            origin + egui::vec2(control.x as f32, control.y as f32),
                            size,
                        );
                        let response = ui.interact(
                            rect,
                            egui::Id::new(("dialog-control", dialog.name.clone(), control.id)),
                            egui::Sense::click_and_drag(),
                        );
                        if response.drag_started() {
                            self.drag_origin = egui::pos2(control.x as f32, control.y as f32);
                            self.drag_rest = egui::Vec2::ZERO;
                        }
                        if response.dragged() {
                            // Keep the whole movement since the drag began, so
                            // sub-pixel steps are not lost to rounding.
                            self.drag_rest += response.drag_delta();
                            let at = self.drag_origin + self.drag_rest;
                            let max_x = (dialog.width as f32 - size.x).max(0.0);
                            let max_y = (dialog.height as f32 - size.y).max(0.0);
                            control.x = at.x.clamp(0.0, max_x).round() as i32;
                            control.y = at.y.clamp(0.0, max_y).round() as i32;
                            moved = true;
                        }
                        if response.drag_stopped() {
                            moved = true;
                            self.dialog_dirty = true;
                        }
                        painter.rect_stroke(
                            rect,
                            0.0,
                            egui::Stroke::new(1.0, color_hilite()),
                            egui::StrokeKind::Inside,
                        );
                        painter.with_clip_rect(rect.intersect(canvas)).text(
                            rect.left_top() + egui::vec2(3.0, 1.0),
                            egui::Align2::LEFT_TOP,
                            format!("{} {}", control.kind, control.label),
                            egui::FontId::proportional(12.0),
                            text_on(color_face(), color_ink()),
                        );
                    }
                    if moved {
                        self.dialog_script = crate::dialog::emit(dialog);
                    }
                }
            });
        if self.dialog_dirty {
            self.dialog_dirty = false;
            self.persist();
        }
        if let Some(name) = created {
            let dialog = crate::dialog::Dialog::empty(&name);
            self.dialog_script = crate::dialog::emit(&dialog);
            self.settings.dialogs.push(dialog);
            self.editing_dialog = name;
            self.new_dialog_name.clear();
            self.persist();
        }
        if delete {
            let name = self.editing_dialog.clone();
            self.settings.dialogs.retain(|dialog| dialog.name != name);
            self.editing_dialog.clear();
            self.dialog_script.clear();
            self.dialog_note = trf("Deleted {0}.", &[&name]);
            self.persist();
        }
        if apply {
            match crate::dialog::parse(&self.dialog_script) {
                Some(dialog) => {
                    self.editing_dialog = dialog.name.clone();
                    match self
                        .settings
                        .dialogs
                        .iter_mut()
                        .find(|item| item.name == dialog.name)
                    {
                        Some(slot) => *slot = dialog,
                        None => self.settings.dialogs.push(dialog),
                    }
                    self.dialog_note = tr("Applied.").into();
                    self.persist();
                }
                None => {
                    self.dialog_note =
                        tr("That text could not be read. The first line must be: dialog name {")
                            .into();
                }
            }
        }
        if let Some(kind) = add_kind {
            let name = self.editing_dialog.clone();
            if let Some(dialog) = self
                .settings
                .dialogs
                .iter_mut()
                .find(|dialog| dialog.name == name)
            {
                let id = dialog
                    .controls
                    .iter()
                    .map(|control| control.id)
                    .max()
                    .unwrap_or(0)
                    + 1;
                let y = (8 + i32::from(id) * 22).min((dialog.height - 20).max(0));
                dialog.controls.push(crate::dialog::Control {
                    kind,
                    id,
                    label: "Label".into(),
                    x: 8,
                    y,
                    w: 80,
                    h: 18,
                    command: String::new(),
                });
                self.dialog_script = crate::dialog::emit(dialog);
                self.persist();
            }
        }
        if !open {
            self.show_dialog_editor = false;
        }
    }

    /// A dialog opened with `/dialog name`, laid out at the positions from
    /// its text. Edit and check values live while the dialog is open and can
    /// be used in button commands as `$did(id)`.
    fn live_dialog(&mut self, ctx: &egui::Context) {
        let Some(name) = self.open_dialog.clone() else {
            return;
        };
        let Some(dialog) = self
            .settings
            .dialogs
            .iter()
            .find(|dialog| dialog.name == name)
            .cloned()
        else {
            self.open_dialog = None;
            return;
        };
        let mut open = true;
        let mut command = None;
        egui::Window::new(&dialog.title)
            .id(egui::Id::new(("live-dialog", &dialog.name)))
            .collapsible(false)
            .resizable([false, false])
            .default_pos(dialog_pos(ctx, 3))
            .open(&mut open)
            .show(ctx, |ui| {
                let (canvas, _) = ui.allocate_exact_size(
                    egui::vec2(dialog.width.max(40) as f32, dialog.height.max(40) as f32),
                    egui::Sense::hover(),
                );
                for control in &dialog.controls {
                    let rect = egui::Rect::from_min_size(
                        canvas.min + egui::vec2(control.x as f32, control.y as f32),
                        egui::vec2(control.w.max(8) as f32, control.h.max(8) as f32),
                    )
                    .intersect(canvas);
                    let key = (dialog.name.clone(), control.id);
                    match control.kind.as_str() {
                        "text" => {
                            ui.put(rect, egui::Label::new(&control.label).truncate());
                        }
                        "edit" => {
                            let value = self
                                .dialog_values
                                .entry(key)
                                .or_insert_with(|| control.label.clone());
                            ui.put(rect, field(value).desired_width(rect.width()));
                        }
                        "check" => {
                            let value = self.dialog_values.entry(key).or_insert_with(|| "0".into());
                            let mut checked = value == "1";
                            if ui
                                .put(rect, egui::Checkbox::new(&mut checked, &control.label))
                                .changed()
                            {
                                *value = if checked { "1".into() } else { "0".into() };
                            }
                        }
                        "button" => {
                            let clicked = ui.put(rect, egui::Button::new(&control.label)).clicked();
                            if clicked && !control.command.is_empty() {
                                command = Some(control.command.clone());
                            }
                        }
                        _ => {}
                    }
                }
            });
        if let Some(command) = command {
            let filled = fill_did(&command, &dialog.name, &self.dialog_values);
            self.fire_scripts("dialog", "", &dialog.name, &filled);
            // Split before filling in `$did` values, so a separator typed into
            // an edit box stays text.
            let separator = self.settings.split_token();
            let parts: Vec<String> = command
                .split(separator.as_str())
                .map(|part| fill_did(part, &dialog.name, &self.dialog_values))
                .collect();
            self.in_run(|app| {
                for part in parts {
                    app.dispatch(part, Origin::Script);
                }
            });
        }
        if !open {
            self.open_dialog = None;
            self.dialog_values
                .retain(|(owner, _), _| *owner != dialog.name);
        }
    }

    fn profiler_window(&mut self, ctx: &egui::Context) {
        if !self.show_profiler {
            return;
        }
        let mut open = true;
        let summaries = self.profile.summaries();
        let lines: usize = self.windows.iter().map(|window| window.lines.len()).sum();
        egui::Window::new(tr("Profiler"))
            .default_pos(dialog_pos(ctx, 6))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(trf(
                    "{0} windows, {1} transcript lines",
                    &[&self.windows.len(), &lines],
                ));
                egui::Grid::new("profile-grid")
                    .num_columns(6)
                    .striped(true)
                    .show(ui, |ui| {
                        for head in ["section", "count", "mean", "p50", "p95", "max"] {
                            ui.strong(head);
                        }
                        ui.end_row();
                        for summary in &summaries {
                            ui.label(summary.name);
                            ui.label(summary.count.to_string());
                            ui.label(crate::profile::millis(summary.mean));
                            ui.label(crate::profile::millis(summary.p50));
                            ui.label(crate::profile::millis(summary.p95));
                            ui.label(crate::profile::millis(summary.max));
                            ui.end_row();
                        }
                    });
            });
        ctx.request_repaint_after(Duration::from_millis(250));
        if !open {
            self.show_profiler = false;
        }
    }

    fn clear_history_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut picks) = self.clear_history else {
            return;
        };
        let mut open = true;
        let mut clear = false;
        let mut cancel = false;
        let urls = self
            .find(Kind::List, "URLs")
            .map_or(0, |index| self.windows[index].lines.len());
        let messages = self
            .find(Kind::List, MESSAGES)
            .map_or(0, |index| self.windows[index].lines.len());
        egui::Window::new(tr("Clear history"))
            .default_pos(dialog_pos(ctx, 5))
            .collapsible(false)
            .resizable([false, false])
            .open(&mut open)
            .show(ctx, |ui| {
                let labels = [
                    trf("Typed lines ({0})", &[&self.history.len()]),
                    trf("Caught URLs ({0})", &[&urls]),
                    tr("Find text").to_string(),
                    trf(
                        "Channels list results ({0})",
                        &[&self.channel_list.entries().len()],
                    ),
                    trf("Messages window ({0} lines)", &[&messages]),
                ];
                for (pick, label) in picks.iter_mut().zip(labels) {
                    ui.checkbox(pick, label);
                }
                ui.label(
                    egui::RichText::new(tr("Logs are kept; the Log files window deletes them."))
                        .small()
                        .color(muted_on_face()),
                );
                ui.horizontal(|ui| {
                    if ui.button(tr("Clear")).clicked() {
                        clear = true;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        cancel = true;
                    }
                });
            });
        self.clear_history = (open && !cancel).then_some(picks);
        if clear {
            self.clear_history = None;
            self.clear_picked(picks);
        }
    }

    fn clear_picked(&mut self, picks: [bool; 5]) {
        if picks[0] {
            self.history.clear();
        }
        for (pick, title) in [(picks[1], "URLs"), (picks[4], MESSAGES)] {
            if pick {
                for window in &mut self.windows {
                    if window.kind == Kind::List && window.title == title {
                        window.lines.clear();
                        window.generation += 1;
                    }
                }
            }
        }
        if picks[2] {
            self.find_text.clear();
            self.find_cache = None;
        }
        if picks[3] {
            self.channel_list.clear();
        }
        self.note_status(tr("History cleared.").into());
    }

    fn about_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }
        let mut open = true;
        egui::Window::new(tr("About OpenIRC"))
            .default_pos(dialog_pos(ctx, 9))
            .collapsible(false)
            .resizable([false, false])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(concat!("OpenIRC ", env!("CARGO_PKG_VERSION")));
                ui.label(tr("A native IRC client with the classic window layout."));
                ui.label(tr("Connect, channels, queries, notify, favorites, aliases, logging, and DCC chat are wired to this window."));
            });
        if !open {
            self.show_about = false;
        }
    }

    fn part_active(&mut self) {
        let Some(window) = self.windows.get(self.active) else {
            return;
        };
        if window.kind == Kind::Channel {
            let title = window.title.clone();
            self.send_user(format!("/part {title}"));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrayStep {
    Hide,
    Show,
}

/// What to do with the window for the tray. It hides only with an icon
/// really in a tray, and comes back as soon as that icon is gone (the tray
/// or panel was closed), since nothing else could bring it back.
fn tray_step(hidden: bool, to_tray: bool, minimized: bool, in_tray: bool) -> Option<TrayStep> {
    if hidden {
        (!in_tray).then_some(TrayStep::Show)
    } else {
        (to_tray && minimized && in_tray).then_some(TrayStep::Hide)
    }
}

/// The queue from a server's thread to the window: `INGRESS_LIMIT` long.
fn ingress() -> (mpsc::SyncSender<NetMsg>, Receiver<NetMsg>) {
    mpsc::sync_channel(INGRESS_LIMIT)
}

fn spawn_link(
    host: String,
    port: u16,
    tls: bool,
    config: Config,
    proxy: Option<crate::socks::Endpoint>,
    options: connection::ConnectOptions,
) -> Link {
    let (out_tx, out_rx) = mpsc::channel();
    let (in_tx, in_rx) = ingress();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);
    let debug = Arc::new(AtomicBool::new(false));
    let debug_thread = Arc::clone(&debug);
    thread::spawn(move || {
        drive(
            Dial {
                server: host,
                port,
                tls,
                proxy,
                options,
            },
            config,
            in_tx,
            out_rx,
            [stop_thread, debug_thread],
        )
    });
    Link {
        stop,
        debug,
        outgoing: out_tx,
        incoming: in_rx,
    }
}

struct Dial {
    server: String,
    port: u16,
    tls: bool,
    proxy: Option<crate::socks::Endpoint>,
    options: connection::ConnectOptions,
}

fn drive(
    dial: Dial,
    config: Config,
    incoming: mpsc::SyncSender<NetMsg>,
    outgoing: Receiver<String>,
    [stop, debug]: [Arc<AtomicBool>; 2],
) {
    let socket = match connection::connect(
        &dial.server,
        dial.port,
        dial.tls,
        dial.proxy.as_ref(),
        &dial.options,
    ) {
        Ok(socket) => {
            if let Some(fingerprint) = socket.peer_sha256() {
                let _ = incoming.send(NetMsg::Cert(fingerprint));
            }
            socket
        }
        Err(err) => {
            let untrusted = err
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<crate::tls::UntrustedCert>())
                .cloned();
            let _ = incoming.send(match untrusted {
                Some(cert) => NetMsg::Untrusted(cert),
                None => NetMsg::Failed(err.to_string()),
            });
            return;
        }
    };
    let mut connection = Connection::new(socket, config);
    connection.set_debug(debug);
    let mut reported_queue = 0;
    if let Err(err) = connection.register() {
        let _ = incoming.send(NetMsg::Failed(err.to_string()));
        return;
    }
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let wire = connection.take_wire();
        if !wire.is_empty() && incoming.send(NetMsg::Wire(wire)).is_err() {
            break;
        }
        match connection.read_effect() {
            Ok(Some(effect)) => {
                let quit = effect.quit;
                // Raw lines first, so the Debug window shows a line before
                // the events it caused.
                let wire = connection.take_wire();
                if !wire.is_empty() && incoming.send(NetMsg::Wire(wire)).is_err() {
                    break;
                }
                if incoming.send(NetMsg::Effect(effect)).is_err() {
                    break;
                }
                if quit {
                    break;
                }
            }
            Ok(None) => {}
            Err(err) => {
                let _ = incoming.send(NetMsg::Failed(err.to_string()));
                break;
            }
        }
        while let Ok(line) = outgoing.try_recv() {
            match connection.handle_user(&line) {
                Ok(effect) => {
                    let quit = effect.quit;
                    let wire = connection.take_wire();
                    if !wire.is_empty() && incoming.send(NetMsg::Wire(wire)).is_err() {
                        return;
                    }
                    if incoming.send(NetMsg::Effect(effect)).is_err() {
                        return;
                    }
                    if quit {
                        return;
                    }
                }
                Err(err) => {
                    let _ = incoming.send(NetMsg::Failed(err.to_string()));
                    return;
                }
            }
        }
        if let Err(err) = connection.flush_queue() {
            let _ = incoming.send(NetMsg::Failed(err.to_string()));
            return;
        }
        let queued = connection.queued();
        if queued != reported_queue {
            reported_queue = queued;
            if incoming.send(NetMsg::Queued(queued)).is_err() {
                return;
            }
        }
    }
}

fn paper() -> egui::Frame {
    egui::Frame::new()
        .fill(color_paper())
        .stroke(egui::Stroke::new(1.0, color_shadow()))
        .inner_margin(egui::Margin::same(4))
        .corner_radius(egui::CornerRadius::ZERO)
}

/// A short label for a server in the switchbar: an IP address whole,
/// otherwise the first name after a leading `irc.` (`irc.libera.chat` is
/// `libera`).
fn short_host(host: &str) -> &str {
    if host.parse::<std::net::IpAddr>().is_ok() {
        return host;
    }
    let rest = ["irc.", "chat."]
        .iter()
        .find_map(|prefix| {
            host.get(..prefix.len())
                .filter(|start| start.eq_ignore_ascii_case(prefix))
                .map(|_| &host[prefix.len()..])
        })
        .filter(|rest| rest.contains('.'))
        .unwrap_or(host);
    rest.split('.').next().unwrap_or(rest)
}

fn open_url(url: &str) {
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(not(windows))]
    {
        command.arg(url);
        let _ = command.spawn();
    }
    #[cfg(windows)]
    shell_open(url);
}

/// Open a web address, folder or file with the program Windows has for it.
/// The text goes to the shell as one literal UTF-16 string, never through
/// `cmd`, so `&`, `%` and quotes in it are only characters of the name.
#[cfg(windows)]
fn shell_open(target: &str) {
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            window: *mut core::ffi::c_void,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show: i32,
        ) -> *mut core::ffi::c_void;
    }
    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoInitializeEx(reserved: *mut core::ffi::c_void, flags: u32) -> i32;
        fn CoUninitialize();
    }
    const SW_SHOWNORMAL: i32 = 1;
    const COINIT_APARTMENTTHREADED: u32 = 0x2;
    const COINIT_DISABLE_OLE1DDE: u32 = 0x4;
    let wide = |text: &str| -> Vec<u16> { text.encode_utf16().chain(Some(0)).collect() };
    let operation = wide("open");
    let file = wide(target);
    // The shell may load handlers that expect a thread of their own, and
    // some of them are COM objects: the thread sets COM up first, in the
    // single-threaded mode the shell documents for ShellExecute.
    std::thread::spawn(move || unsafe {
        let com = CoInitializeEx(
            std::ptr::null_mut(),
            COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE,
        );
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
        // S_OK or S_FALSE: this thread's COM is ours to close.
        if com >= 0 {
            CoUninitialize();
        }
    });
}

/// The web search address for `words`: `%s` in the setting is replaced by
/// the encoded words, or they are added at the end.
fn search_address(template: &str, words: &str) -> String {
    let mut encoded = String::new();
    for byte in words.trim().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    let template = if template.trim().is_empty() {
        Settings::new().search_url
    } else {
        template.trim().to_string()
    };
    if template.contains("%s") {
        template.replacen("%s", &encoded, 1)
    } else {
        template + &encoded
    }
}

fn line_matches(line: &RichLine, query: &str) -> bool {
    line.plain().to_lowercase().contains(query)
}

fn note(text: impl Into<String>) -> RichLine {
    event_line(Ev::Info, text)
}

/// A line about something that happened, drawn in that event's color.
fn event_line(ev: Ev, text: impl Into<String>) -> RichLine {
    let mut spans = vec![Piece::new(Tone::Event(ev), "* ")];
    spans.extend(coded(&text.into(), Tone::Event(ev), false));
    RichLine {
        time: Some(crate::clock::now()),
        spans,
        nick: None,
        measured: std::cell::Cell::new((0, 0.0)),
    }
}

/// Text with color and style codes as pieces; `strip` drops the codes.
fn coded(text: &str, base: Tone, strip: bool) -> Vec<Piece> {
    crate::colors::parse(text)
        .into_iter()
        .map(|span| {
            if strip {
                return Piece::new(base, span.text);
            }
            Piece {
                tone: span.color.map(Tone::Palette).unwrap_or(base),
                style: span.style,
                background: span.background,
                text: span.text,
            }
        })
        .collect()
}

fn topic_line(topic: &str) -> RichLine {
    RichLine {
        time: None,
        spans: coded(topic, Tone::Ink, false),
        nick: None,
        measured: std::cell::Cell::new((0, 0.0)),
    }
}

fn chat(from: &str, text: &str, action: bool, highlight: bool) -> RichLine {
    chat_line(from, text, action, highlight, false)
}

/// A chat line. Highlights draw the whole line in the attention color and
/// the nick in bold; `strip` removes incoming color and style codes.
fn chat_line(from: &str, text: &str, action: bool, highlight: bool, strip: bool) -> RichLine {
    let mut nick = Piece::new(Tone::Nick(nick_slot(from)), from);
    if highlight {
        nick = nick.bold();
    }
    let mut spans = if action {
        vec![
            Piece::new(Tone::Ink, "• "),
            nick,
            Piece::new(Tone::Ink, " "),
        ]
    } else {
        vec![
            Piece::new(Tone::Ink, "<"),
            nick,
            Piece::new(Tone::Ink, "> "),
        ]
    };
    let base = if highlight {
        Tone::Attention
    } else {
        Tone::Ink
    };
    let mut body = coded(text, base, strip);
    if highlight {
        for piece in &mut body {
            piece.tone = Tone::Attention;
        }
    }
    spans.extend(body);
    RichLine {
        time: Some(crate::clock::now()),
        spans,
        nick: Some(from.to_string()),
        measured: std::cell::Cell::new((0, 0.0)),
    }
}

/// A bold heading for a group of settings.
fn section(ui: &mut egui::Ui, text: &str) {
    let size = ui.style().text_styles[&egui::TextStyle::Body].size;
    ui.label(egui::RichText::new(text).font(egui::FontId::new(
        size,
        egui::FontFamily::Name("bold".into()),
    )));
}

/// The `/ignore` command that recreates `rule` in a session.
fn ignore_command(rule: &crate::rules::IgnoreRule) -> String {
    let mut command = format!("/ignore -{}", rule.types);
    if rule.expires > 0 {
        let left = rule.expires.saturating_sub(crate::rules::unix_now()).max(1);
        command.push_str(&format!(" -u{left}"));
    }
    if !rule.network.trim().is_empty() {
        command.push_str(&format!(" -w{}", rule.network.trim()));
    }
    command.push(' ');
    command.push_str(&rule.mask);
    command
}

/// "1 user", "3 users".
/// The name of a sound event, for people.
fn sound_event_name(event: &str) -> &'static str {
    match event {
        "message" => tr("Message"),
        "action" => tr("Action"),
        "notice" => tr("Notice"),
        "join" => tr("Someone joins"),
        "part" => tr("Someone leaves"),
        "quit" => tr("Someone quits"),
        "kick" => tr("Someone is kicked"),
        "highlight" => tr("Highlight"),
        _ => "DCC",
    }
}

/// Text in bold, with the IRC bold code around it.
fn bold(text: &str) -> String {
    format!("\u{2}{text}\u{2}")
}

/// A count with the word for it, in the current language: `one` and
/// `many` are texts with `{0}` for the number, such as `"{0} user"`.
fn plural(count: usize, one: &str, many: &str) -> String {
    crate::i18n::trf(if count == 1 { one } else { many }, &[&count])
}

/// Text on the window chrome that should be quieter than labels.
fn muted_on_face() -> egui::Color32 {
    mix(text_on(color_face(), color_ink()), color_face(), 0.45)
}

/// The nick part of a `nick!user@host` setter.
fn bare_setter(setter: &str) -> &str {
    setter.split('!').next().unwrap_or(setter)
}

fn palette_color(index: u8) -> egui::Color32 {
    let palette = crate::theme::palette();
    let rgb = palette[(index as usize) % palette.len()];
    egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

fn nick_slot(nick: &str) -> u8 {
    let mut hash = 0usize;
    for byte in bare_nick(nick).to_ascii_lowercase().bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(byte as usize);
    }
    (hash % NICK_COLORS.len()) as u8
}

/// Paint color for a tone on the current paper. Fixed palette colors are
/// nudged toward the ink until they are readable on the paper.
fn tone_color(tone: Tone) -> egui::Color32 {
    match tone {
        Tone::Ink => color_ink(),
        Tone::Attention => readable(color_attention(), color_paper(), color_ink()),
        Tone::Nick(slot) => readable(
            NICK_COLORS[slot as usize % NICK_COLORS.len()],
            color_paper(),
            color_ink(),
        ),
        Tone::Palette(index) => readable(palette_color(index), color_paper(), color_ink()),
        Tone::Event(ev) => {
            let base = match ev {
                Ev::Join => egui::Color32::from_rgb(0, 140, 60),
                Ev::Part => egui::Color32::from_rgb(140, 90, 0),
                Ev::Kick => egui::Color32::from_rgb(200, 30, 30),
                Ev::Mode => egui::Color32::from_rgb(0, 128, 140),
                Ev::Notice => egui::Color32::from_rgb(150, 60, 140),
                Ev::Info => return mix(color_ink(), color_paper(), 0.25),
            };
            readable(base, color_paper(), color_ink())
        }
    }
}

/// Width of the label column in every label-and-field form, so the fields
/// of different sections on one page start in the same place.
const FORM_LABEL_WIDTH: f32 = 175.0;

/// A grid for a form: labels on the left, fields on the right.
fn form_grid(id: &str) -> egui::Grid {
    egui::Grid::new(id).min_col_width(FORM_LABEL_WIDTH)
}

/// One line of text laid out to fit `max_width`, ending in `…` when it
/// does not.
fn fitted_text(
    painter: &egui::Painter,
    text: &str,
    font: egui::FontId,
    color: egui::Color32,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping {
        max_width: max_width.max(1.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    painter.layout_job(job)
}

/// Timestamps: quieter than the text but still easy to read (about 4:1
/// against the paper).
fn stamp_color() -> egui::Color32 {
    mix(color_paper(), color_ink(), 0.5)
}

fn muted_color() -> egui::Color32 {
    let ink = color_ink();
    let paper = color_paper();
    egui::Color32::from_rgb(
        ((ink.r() as u16 + paper.r() as u16 * 2) / 3) as u8,
        ((ink.g() as u16 + paper.g() as u16 * 2) / 3) as u8,
        ((ink.b() as u16 + paper.b() as u16 * 2) / 3) as u8,
    )
}

/// WCAG relative luminance of an sRGB color.
fn luminance(color: egui::Color32) -> f32 {
    let channel = |value: u8| {
        let value = value as f32 / 255.0;
        if value <= 0.039_28 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
}

fn contrast(left: egui::Color32, right: egui::Color32) -> f32 {
    let (a, b) = (luminance(left), luminance(right));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// `color` if it reads well on `background`, otherwise `color` blended
/// toward `fallback` until it does.
fn readable(
    color: egui::Color32,
    background: egui::Color32,
    fallback: egui::Color32,
) -> egui::Color32 {
    // WCAG AA for normal text.
    const ENOUGH: f32 = 4.5;
    if contrast(color, background) >= ENOUGH {
        return color;
    }
    for step in 1..=10 {
        let mix = step as f32 / 10.0;
        let blended = egui::Color32::from_rgb(
            (color.r() as f32 * (1.0 - mix) + fallback.r() as f32 * mix) as u8,
            (color.g() as f32 * (1.0 - mix) + fallback.g() as f32 * mix) as u8,
            (color.b() as f32 * (1.0 - mix) + fallback.b() as f32 * mix) as u8,
        );
        if contrast(blended, background) >= ENOUGH {
            return blended;
        }
    }
    fallback
}

/// Black or white, whichever reads better on `background`, unless `preferred`
/// already reads well.
fn text_on(background: egui::Color32, preferred: egui::Color32) -> egui::Color32 {
    if contrast(preferred, background) >= 4.5 {
        return preferred;
    }
    if contrast(egui::Color32::BLACK, background) >= contrast(egui::Color32::WHITE, background) {
        egui::Color32::BLACK
    } else {
        egui::Color32::WHITE
    }
}

fn bare_nick(nick: &str) -> String {
    nick.trim_start_matches(['@', '+', '%', '&', '~'])
        .to_string()
}

thread_local! {
    /// How the server being handled compares names. The window sets it for
    /// each server's lines and for the server in front between them.
    static CASEMAP: std::cell::Cell<crate::state::CaseMap> =
        const { std::cell::Cell::new(crate::state::CaseMap::Rfc1459) };
}

fn set_casemap(map: crate::state::CaseMap) {
    CASEMAP.with(|cell| cell.set(map));
}

fn casemap() -> crate::state::CaseMap {
    CASEMAP.with(std::cell::Cell::get)
}

/// A nick as a map key: equal for every spelling the server treats as the
/// same nick.
fn nick_key(nick: &str) -> String {
    casemap().fold(&bare_nick(nick))
}

fn same_nick(left: &str, right: &str) -> bool {
    nick_key(left) == nick_key(right)
}

fn same_name(left: &str, right: &str) -> bool {
    casemap().fold(left) == casemap().fold(right)
}

fn sort_nicks(nicks: &mut [String]) {
    nicks.sort_by(|left, right| {
        nick_rank(left)
            .cmp(&nick_rank(right))
            .then_with(|| bare_nick(left).cmp(&bare_nick(right)))
    });
}

fn nick_rank(nick: &str) -> u8 {
    match nick.chars().next() {
        Some('@' | '~' | '&') => 0,
        Some('%') => 1,
        Some('+') => 2,
        _ => 3,
    }
}

/// A single-line text field drawn in the paper and ink colors.
fn field(text: &mut String) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(text).text_color(color_ink())
}

/// A multi-line text field drawn in the paper and ink colors.
fn field_multi(text: &mut String) -> egui::TextEdit<'_> {
    egui::TextEdit::multiline(text).text_color(color_ink())
}

/// One transcript line laid out for painting, with the character ranges of
/// the URLs in it.
struct LaidLine {
    galley: std::sync::Arc<egui::Galley>,
    urls: Vec<(std::ops::Range<usize>, String)>,
}

/// How transcript text is drawn: size, font, and timestamp format (empty
/// for no timestamps).
#[derive(Clone, Copy)]
struct TextLook<'a> {
    font: f32,
    mono: bool,
    stamp: &'a str,
}

impl TextLook<'_> {
    fn font_id(&self, bold: bool, italic: bool) -> egui::FontId {
        let family = match (self.mono, bold, italic) {
            (true, true, _) => egui::FontFamily::Name("mono-bold".into()),
            (true, false, _) => egui::FontFamily::Monospace,
            (false, true, _) => egui::FontFamily::Name("bold".into()),
            (false, false, true) => egui::FontFamily::Name("italic".into()),
            (false, false, false) => egui::FontFamily::Proportional,
        };
        egui::FontId::new(self.font, family)
    }

    /// Part of the layout cache key: anything that changes line heights.
    fn key(&self, width: f32) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (
            width.round() as i64,
            self.font.round() as i64,
            self.mono,
            self.stamp,
            FONT_GENERATION.load(std::sync::atomic::Ordering::Relaxed),
        )
            .hash(&mut hasher);
        hasher.finish() | 1
    }
}

/// Format a timestamp: `HH`, `MM`, and `SS` are replaced, the rest is kept.
fn format_stamp(format: &str, time: crate::clock::LocalTime) -> String {
    format
        .replace("HH", &format!("{:02}", time.hour))
        .replace("MM", &format!("{:02}", time.minute))
        .replace("SS", &format!("{:02}", time.second))
}

/// Laid-out spans: byte range in the line text and how to draw it.
struct Run {
    start: usize,
    end: usize,
    color: egui::Color32,
    background: Option<egui::Color32>,
    style: crate::colors::Style,
    spellable: bool,
}

/// The word list for spell checking, and the words (nicks, channels) to
/// leave alone, all lowercase.
#[derive(Clone, Copy)]
struct Spell<'a> {
    words: &'a std::collections::HashSet<String>,
    skip: &'a std::collections::HashSet<String>,
}

fn line_galley(
    ui: &egui::Ui,
    line: &RichLine,
    look: TextLook<'_>,
    width: f32,
    spell: Spell<'_>,
) -> LaidLine {
    let mut text = String::new();
    let mut runs: Vec<Run> = Vec::new();
    if !look.stamp.is_empty()
        && let Some(time) = line.time
    {
        let stamp = format!("{} ", format_stamp(look.stamp, time));
        runs.push(Run {
            start: 0,
            end: stamp.len(),
            color: stamp_color(),
            background: None,
            style: crate::colors::Style::default(),
            spellable: false,
        });
        text.push_str(&stamp);
    }
    let body_start = text.len();
    for piece in &line.spans {
        let start = text.len();
        text.push_str(&piece.text);
        let mut color = tone_color(piece.tone);
        let mut background = piece.background.map(palette_color);
        if piece.style.reverse {
            let back = background.unwrap_or_else(color_paper);
            background = Some(color);
            color = back;
        }
        runs.push(Run {
            start,
            end: text.len(),
            color,
            background,
            style: piece.style,
            spellable: matches!(piece.tone, Tone::Ink | Tone::Palette(_) | Tone::Attention),
        });
    }
    let mut urls: Vec<(usize, usize, String)> = Vec::new();
    let mut from = body_start;
    for url in crate::urls::find_urls(&text[body_start..]) {
        if let Some(found) = text[from..].find(&url) {
            let start = from + found;
            urls.push((start, start + url.len(), url.clone()));
            from = start + url.len();
        }
    }
    let mut misses: Vec<(usize, usize)> = Vec::new();
    if !spell.words.is_empty() {
        for run in runs.iter().filter(|run| run.spellable) {
            let piece = &text[run.start..run.end];
            for (from, to) in crate::spell::miss_ranges(piece, spell.words, spell.skip, false) {
                misses.push((run.start + from, run.start + to));
            }
        }
    }
    let mut cuts: Vec<usize> = vec![0, text.len()];
    cuts.extend(runs.iter().flat_map(|run| [run.start, run.end]));
    cuts.extend(urls.iter().flat_map(|url| [url.0, url.1]));
    cuts.extend(misses.iter().flat_map(|miss| [miss.0, miss.1]));
    cuts.sort_unstable();
    cuts.dedup();
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = width;
    let link = readable(ui.visuals().hyperlink_color, color_paper(), color_ink());
    for pair in cuts.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        if start == end {
            continue;
        }
        let run = runs
            .iter()
            .find(|run| run.start <= start && start < run.end);
        let style = run.map(|run| run.style).unwrap_or_default();
        let color = run.map_or_else(color_ink, |run| run.color);
        let in_url = urls.iter().any(|url| url.0 <= start && start < url.1);
        let missed = misses.iter().any(|miss| miss.0 <= start && start < miss.1);
        let mut format = egui::TextFormat::simple(look.font_id(style.bold, style.italic), color);
        if let Some(background) = run.and_then(|run| run.background) {
            format.background = background;
        }
        if style.underline {
            format.underline = egui::Stroke::new(1.0, color);
        }
        if style.strike {
            format.strikethrough = egui::Stroke::new(1.0, color);
        }
        if in_url {
            format.color = link;
            format.underline = egui::Stroke::new(1.0, link);
        } else if missed {
            format.underline = egui::Stroke::new(1.0, tone_color(Tone::Attention));
        }
        job.append(&text[start..end], 0.0, format);
    }
    let char_at = |byte: usize| text[..byte].chars().count();
    let urls = urls
        .into_iter()
        .map(|(start, end, url)| (char_at(start)..char_at(end), url))
        .collect();
    LaidLine {
        galley: ui.painter().layout_job(job),
        urls,
    }
}

/// Pointer cursor over links, and open the link that was clicked.
fn link_clicks(
    ui: &egui::Ui,
    response: &egui::Response,
    rect: egui::Rect,
    laid: &LaidLine,
    actions: &mut Vec<LineAction>,
) {
    if laid.urls.is_empty() {
        return;
    }
    let url_at = |pos: egui::Pos2| {
        let local = pos - rect.min;
        if local.y > laid.galley.size().y || local.x > laid.galley.size().x {
            return None;
        }
        let index = laid.galley.cursor_from_pos(local).index.0;
        laid.urls
            .iter()
            .find(|(range, _)| range.start <= index && index < range.end)
            .map(|(_, url)| url.clone())
    };
    if let Some(pos) = response.hover_pos()
        && url_at(pos).is_some()
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && let Some(url) = url_at(pos)
    {
        actions.push(LineAction::Open(url));
    }
}

fn theme_editor(ui: &mut egui::Ui, theme_slot: &mut Theme) {
    section(ui, tr("Schemes"));
    ui.horizontal_wrapped(|ui| {
        for (name, scheme) in Theme::schemes() {
            let selected = *theme_slot == scheme;
            let size = egui::vec2(104.0, 58.0);
            let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
            let painter = ui.painter();
            painter.rect_filled(rect, egui::CornerRadius::same(4), paint(scheme.face));
            let paper_rect = egui::Rect::from_min_max(
                rect.min + egui::vec2(6.0, 6.0),
                rect.max - egui::vec2(6.0, 22.0),
            );
            painter.rect_filled(paper_rect, egui::CornerRadius::same(2), paint(scheme.paper));
            painter.text(
                paper_rect.left_center() + egui::vec2(4.0, 0.0),
                egui::Align2::LEFT_CENTER,
                "<nick> hello",
                egui::FontId::proportional(11.0),
                paint(scheme.ink),
            );
            painter.text(
                egui::pos2(rect.center().x, rect.bottom() - 10.0),
                egui::Align2::CENTER_CENTER,
                tr(name),
                egui::FontId::proportional(11.0),
                text_on(paint(scheme.face), paint(scheme.ink)),
            );
            let stroke = if selected {
                egui::Stroke::new(2.0, paint(scheme.hilite))
            } else {
                egui::Stroke::new(1.0, paint(scheme.shadow))
            };
            painter.rect_stroke(
                rect,
                egui::CornerRadius::same(4),
                stroke,
                egui::StrokeKind::Inside,
            );
            if response
                .on_hover_text(trf("Use the {0} colors", &[&name]))
                .clicked()
            {
                *theme_slot = scheme;
            }
        }
    });
    ui.add_space(6.0);
    ui.label(tr(
        "Window colors. Changes show at once; OK or Apply keeps them.",
    ));
    egui::Grid::new("options-colors")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for (label, slot) in [
                (tr("Face (window chrome)"), "face"),
                (tr("Paper (text areas)"), "paper"),
                (tr("Ink (text on paper)"), "ink"),
                (tr("Shadow (borders)"), "shadow"),
                (tr("Highlight (selection)"), "hilite"),
                (tr("Attention (activity)"), "attention"),
            ] {
                let current = match slot {
                    "face" => theme_slot.face,
                    "paper" => theme_slot.paper,
                    "ink" => theme_slot.ink,
                    "shadow" => theme_slot.shadow,
                    "hilite" => theme_slot.hilite,
                    _ => theme_slot.attention,
                };
                let mut rgb = [current.0, current.1, current.2];
                egui::Frame::new()
                    .stroke(egui::Stroke::new(1.0, color_shadow()))
                    .inner_margin(egui::Margin::same(1))
                    .show(ui, |ui| {
                        ui.color_edit_button_srgb(&mut rgb);
                    });
                ui.label(label);
                ui.end_row();
                theme_slot.set(slot, Rgb(rgb[0], rgb[1], rgb[2]));
            }
        });
    let ratio = contrast(paint(theme_slot.ink), paint(theme_slot.paper));
    if ratio < 4.5 {
        ui.colored_label(
            egui::Color32::from_rgb(160, 0, 0),
            trf(
                "Ink on Paper contrast is {0}:1; 4.5:1 or more reads well.",
                &[&format!("{ratio:.1}")],
            ),
        );
    }
    if ui.button(tr("Classic colors")).clicked() {
        *theme_slot = Theme::classic();
    }
}

/// User levels for script events.
fn users_page(ui: &mut egui::Ui, settings: &mut Settings) {
    section(ui, tr("User levels"));
    ui.label(tr(
        "A script event such as on 10:TEXT runs only for people with level 10 or more; \
         on friend:TEXT runs for people with the level friend. Levels are separated by commas.",
    ));
    let mut remove = None;
    egui::Grid::new("users")
        .num_columns(3)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            if !settings.users.is_empty() {
                ui.label(egui::RichText::new(tr("Levels")).strong());
                ui.label(egui::RichText::new(tr("Nick or mask")).strong());
                ui.end_row();
            }
            for (index, (levels, mask)) in settings.users.iter_mut().enumerate() {
                ui.add(
                    field(levels)
                        .desired_width(120.0)
                        .hint_text(tr("10,friend")),
                );
                ui.add(
                    field(mask)
                        .desired_width(220.0)
                        .hint_text(tr("nick or *!*@host")),
                );
                if ui.small_button(tr("Remove")).clicked() {
                    remove = Some(index);
                }
                ui.end_row();
            }
        });
    if let Some(index) = remove {
        settings.users.remove(index);
    }
    if ui.button(tr("Add user")).clicked() {
        settings.users.push(("1".into(), String::new()));
    }
    ui.add_space(6.0);
    form_grid("users-default")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label(tr("Everyone else has level"));
            ui.add(field(&mut settings.default_level).desired_width(60.0));
            ui.end_row();
        });
}

/// The 16 message colors that color codes pick, in two rows of eight.
fn palette_editor(ui: &mut egui::Ui, palette: &mut crate::theme::Palette) {
    ui.label(tr("Message colors (color codes 0 to 15)."));
    egui::Grid::new("options-palette")
        .num_columns(8)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            for row in 0..2 {
                for column in 0..8 {
                    let index = row * 8 + column;
                    ui.vertical(|ui| {
                        ui.set_width(40.0);
                        ui.label(egui::RichText::new(index.to_string()).small());
                        egui::Frame::new()
                            .stroke(egui::Stroke::new(1.0, color_shadow()))
                            .inner_margin(egui::Margin::same(1))
                            .show(ui, |ui| {
                                ui.color_edit_button_srgb(&mut palette[index])
                                    .on_hover_text(trf("Color {0}", &[&index]));
                            });
                    });
                }
                ui.end_row();
            }
        });
    if ui.button(tr("Standard message colors")).clicked() {
        *palette = crate::colors::PALETTE;
    }
}

/// Where a dialog opens: near the top left of the window, stepped by
/// `slot` so two open dialogs do not sit exactly on top of each other.
fn dialog_pos(ctx: &egui::Context, slot: u8) -> egui::Pos2 {
    let area = ctx.content_rect();
    let step = 28.0 * f32::from(slot % 8);
    egui::pos2(area.left() + 140.0 + step, area.top() + 60.0 + step)
}

/// Replace `$did(id)` in a dialog button command with that control's value.
fn fill_did(
    command: &str,
    dialog: &str,
    values: &std::collections::HashMap<(String, u16), String>,
) -> String {
    let mut out = String::new();
    let mut rest = command;
    while let Some(at) = rest.find("$did(") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 5..];
        match after.find(')') {
            Some(close) => {
                let id = after[..close].trim().parse::<u16>().ok();
                let value = id
                    .and_then(|id| values.get(&(dialog.to_string(), id)))
                    .map(String::as_str)
                    .unwrap_or("");
                out.push_str(value);
                rest = &after[close + 1..];
            }
            None => {
                out.push_str(&rest[at..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Run one frame without a window. The texture changes a renderer would
/// upload are dropped on purpose.
#[cfg(any(test, feature = "bench"))]
fn headless_frame(ctx: &egui::Context, input: egui::RawInput, run: impl FnMut(&mut egui::Ui)) {
    let mut output = ctx.run_ui(input, run);
    output.textures_delta.clear();
}

/// Headless frame timing for `examples/bench.rs`: a channel window holding
/// `lines` transcript lines is drawn `frames` times through the real
/// `App::ui`. `grow` adds one chat line before every frame; `find` sets the
/// Find text. Returns the time of each frame.
#[cfg(feature = "bench")]
pub(crate) fn bench_frames(
    lines: usize,
    frames: usize,
    grow: bool,
    find: &str,
    resize: bool,
) -> Vec<Duration> {
    bench_frames_with(lines, 300, frames, grow, find, resize, 0)
}

/// `bench_frames` with `nicks` people in the channel, and with the
/// channels list open holding `listed` channels when that is not 0.
#[cfg(feature = "bench")]
pub(crate) fn bench_frames_with(
    lines: usize,
    nicks: usize,
    frames: usize,
    grow: bool,
    find: &str,
    resize: bool,
    listed: usize,
) -> Vec<Duration> {
    use eframe::App;
    let ctx = egui::Context::default();
    install_fonts(&ctx);
    let mut app = OpenIrcApp::new(None);
    app.preview = false;
    app.find_text = find.to_string();
    let channel = 1;
    app.windows[channel].lines.clear();
    app.windows[channel].nicks = (0..nicks.max(1))
        .map(|n| match n % 10 {
            0 => format!("@user{n}"),
            1 => format!("+user{n}"),
            _ => format!("user{n}"),
        })
        .collect();
    for n in 0..lines {
        let text = format!(
            "message {n} with \u{3}04some color\u{3} and a link https://example.com/{n} plus words"
        );
        app.windows[channel]
            .lines
            .push(chat(&format!("user{}", n % 300), &text, false, false));
    }
    if listed > 0 {
        app.channel_list.open = true;
        for n in 0..listed {
            if let Some(entry) = channel::parse_listed(&format!(
                "#chan{n} {} [+nt] topic number {n} about things",
                (n * 7919) % 5000
            )) {
                app.channel_list.push(entry);
            }
        }
    }
    app.active = channel;
    let mut frame = eframe::Frame::_new_kittest();
    let input = |width: f32| egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(width, 640.0),
        )),
        ..Default::default()
    };
    // Warm up until every line has been measured once, so the timed frames
    // show steady state; `bench_first_frames` covers the frames after a load.
    for _ in 0..(lines / 128 + 4) {
        headless_frame(&ctx, input(980.0), |ui| app.ui(ui, &mut frame));
    }
    let mut times = Vec::with_capacity(frames);
    for n in 0..frames {
        if grow {
            let line = chat("bench", &format!("new line {n}"), false, false);
            app.push(channel, line);
        }
        let width = if resize {
            900.0 + (n % 60) as f32
        } else {
            980.0
        };
        let started = std::time::Instant::now();
        headless_frame(&ctx, input(width), |ui| app.ui(ui, &mut frame));
        times.push(started.elapsed());
    }
    times
}

/// The transcript loop as it was before the review fixes, kept only for the
/// benchmark: every frame it cloned all lines and laid out every span of
/// every line, with a spell check and URL scan per span. Used to compare
/// against `bench_frames`.
#[cfg(feature = "bench")]
pub(crate) fn bench_legacy_transcript(lines: usize, frames: usize) -> Vec<Duration> {
    let ctx = egui::Context::default();
    install_fonts(&ctx);
    let dictionary = std::collections::HashSet::new();
    let mut window = ChatWindow::channel("#c");
    for n in 0..lines {
        let text = format!(
            "message {n} with \u{3}04some color\u{3} and a link https://example.com/{n} plus words"
        );
        window
            .lines
            .push(chat(&format!("user{}", n % 300), &text, false, false));
    }
    let popups: Vec<crate::settings::Popup> = Vec::new();
    let vars: Vec<(String, String)> = Vec::new();
    let input = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(980.0, 640.0),
        )),
        ..Default::default()
    };
    let mut draw = |ui: &mut egui::Ui| {
        egui::CentralPanel::default_margins().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        let all = window.lines.clone();
                        for line in all {
                            let response = ui
                                .horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 0.0;
                                    for piece in &line.spans {
                                        let text = &piece.text;
                                        let miss = !crate::spell::miss_ranges(
                                            text,
                                            &dictionary,
                                            &Default::default(),
                                            false,
                                        )
                                        .is_empty();
                                        let mut rich = egui::RichText::new(text)
                                            .color(tone_color(piece.tone))
                                            .size(13.0);
                                        if miss {
                                            rich = rich.underline();
                                        }
                                        if crate::urls::find_urls(text).into_iter().next().is_some()
                                        {
                                            let _ = ui.link(rich);
                                        } else {
                                            ui.label(rich);
                                        }
                                    }
                                })
                                .response;
                            let popups = popups.clone();
                            let vars = vars.clone();
                            response.context_menu(|ui| {
                                for popup in &popups {
                                    let _ = (ui.button(&popup.label), &vars);
                                }
                            });
                        }
                    });
                });
        });
    };
    for _ in 0..2 {
        headless_frame(&ctx, input(), &mut draw);
    }
    let mut times = Vec::with_capacity(frames);
    for _ in 0..frames {
        let started = std::time::Instant::now();
        headless_frame(&ctx, input(), &mut draw);
        times.push(started.elapsed());
    }
    times
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window that never touches the disk: settings are locked so saving
    /// is refused, and logging is off.
    /// The test app, for tests in the window's submodules.
    pub(super) fn test_app() -> OpenIrcApp {
        app()
    }

    pub(super) fn app() -> OpenIrcApp {
        let settings = Settings {
            locked: true,
            logging: false,
            // Tests read English text whatever the computer's language.
            language: "en".into(),
            ..Settings::new()
        };
        let mut app = OpenIrcApp::with_settings(settings, &[], None);
        app.preview = false;
        app.nick = "me".into();
        app
    }

    /// A session with no network link, as after a disconnect.
    fn test_session(id: u64, tls: bool) -> Session {
        Session {
            id,
            host: "irc.example".into(),
            port: if tls { 6697 } else { 6667 },
            tls,
            nick: "me".into(),
            hangup: true,
            reconnect_at: None,
            away: None,
            fingerprint: None,
            link: None,
            network: None,
            connected_at: None,
            queued: 0,
            autojoin: Vec::new(),
            entry: crate::servers::ServerEntry::new("", "irc.example", 6667, tls),
            attempts: 0,
            caps: Vec::new(),
            casemap: crate::state::CaseMap::Rfc1459,
            away_nicks: std::collections::HashMap::new(),
            renames: 0,
            accounts: std::collections::HashMap::new(),
            umodes: String::new(),
            notify_online: Vec::new(),
        }
    }

    pub(super) fn texts(app: &OpenIrcApp, index: usize) -> Vec<String> {
        app.windows[index]
            .lines
            .iter()
            .map(RichLine::plain)
            .collect()
    }

    #[test]
    fn quit_is_shown_only_where_the_nick_was() {
        let mut app = app();
        let rust = app.ensure(Kind::Channel, "#rust");
        app.windows[rust].nicks = vec!["@alice".into(), "bob".into()];
        let other = app.ensure(Kind::Channel, "#other");
        app.windows[other].nicks = vec!["carol".into()];
        app.windows[other]
            .lines
            .push(note("bob said something about bo"));
        app.apply_event(Event::Quit {
            who: "bo".into(),
            reason: None,
        });
        assert!(texts(&app, rust).is_empty());
        assert_eq!(texts(&app, other).len(), 1);
        app.apply_event(Event::Quit {
            who: "Alice".into(),
            reason: Some("bye".into()),
        });
        assert_eq!(texts(&app, rust), vec!["* Alice quit (bye)".to_string()]);
        assert_eq!(app.windows[rust].nicks, vec!["bob".to_string()]);
    }

    #[test]
    fn nick_changes_touch_member_channels_and_rename_the_query() {
        let mut app = app();
        let rust = app.ensure(Kind::Channel, "#rust");
        app.windows[rust].nicks = vec!["+bob".into(), "zed".into()];
        let quiet = app.ensure(Kind::Channel, "#quiet");
        let query = app.ensure(Kind::Query, "bob");
        let urls = app.ensure(Kind::List, "URLs");
        app.apply_event(Event::Nick {
            old: "bob".into(),
            new: "robert".into(),
        });
        assert_eq!(texts(&app, rust), vec!["* bob is now robert".to_string()]);
        assert_eq!(
            app.windows[rust].nicks,
            vec!["+robert".to_string(), "zed".to_string()]
        );
        assert!(texts(&app, quiet).is_empty());
        assert!(texts(&app, urls).is_empty());
        assert_eq!(app.windows[query].title, "robert");
    }

    #[test]
    fn local_commands_work_without_a_server() {
        let mut app = app();
        app.active = 0;
        app.dispatch("/echo hello there".into(), Origin::Script);
        assert_eq!(texts(&app, 0).last().unwrap(), "* hello there");
        app.dispatch("/clear".into(), Origin::Script);
        assert!(texts(&app, 0).is_empty());
        app.dispatch("/timer 5 2 /echo later".into(), Origin::Script);
        assert_eq!(app.timers.len(), 1);
        app.dispatch("/set %greeting hi".into(), Origin::Script);
        assert_eq!(
            app.settings.vars,
            vec![("greeting".to_string(), "hi".to_string())]
        );
        assert!(
            texts(&app, 0)
                .iter()
                .any(|line| line.contains("Settings were not saved")),
            "a locked test app must refuse to save"
        );
    }

    #[test]
    fn script_aliases_can_call_local_commands_and_other_aliases() {
        let mut app = app();
        app.active = 0;
        app.settings.script_text =
            "alias hello {\n  echo hello $1\n  inner\n}\nalias inner {\n  echo inner ran\n}\n"
                .into();
        app.dispatch("/hello world".into(), Origin::Input);
        let lines = texts(&app, 0);
        assert!(lines.contains(&"* hello world".to_string()));
        assert!(lines.contains(&"* inner ran".to_string()));
    }

    #[test]
    fn runaway_aliases_stop() {
        let mut app = app();
        app.active = 0;
        app.settings.script_text = "alias loop {\n  loop\n}\n".into();
        app.dispatch("/loop".into(), Origin::Input);
        assert!(
            texts(&app, 0)
                .iter()
                .any(|line| line.contains("nested too deeply"))
        );
    }

    #[test]
    fn highlight_rules_color_lines_and_ignore_commands_are_saved() {
        let mut app = app();
        let index = app.ensure(Kind::Channel, "#c");
        let mut rule = crate::rules::HighlightRule::words("deploy");
        rule.color = Some(9);
        rule.notice = false;
        app.settings.highlights.push(rule);
        app.apply_event(Event::Chat {
            channel: true,
            target: "#c".into(),
            from: "ops".into(),
            text: "deploy done".into(),
            action: false,
        });
        let line = app.windows[index].lines.last().unwrap();
        assert!(
            line.spans
                .iter()
                .skip(3)
                .all(|piece| piece.tone == Tone::Palette(9))
        );
        assert_eq!(app.windows[index].activity, Activity::Highlight);
        assert!(app.toasts.is_empty(), "notice turned off for this rule");
        app.dispatch("/ignore -nt -u60 troll".into(), Origin::Input);
        let saved = &app.settings.ignores[0];
        assert_eq!(saved.full_mask(), "troll!*@*");
        assert_eq!(saved.types, "nt");
        assert!(saved.expires > crate::rules::unix_now());
        app.dispatch("/unignore troll".into(), Origin::Input);
        assert!(app.settings.ignores.is_empty());
    }

    #[test]
    fn whois_ctcp_and_notices_follow_the_active_window_options() {
        let mut app = app();
        let status = app.status_index().unwrap();
        let chan = app.ensure(Kind::Channel, "#c");
        app.activate(chan);
        app.settings.active_whois = true;
        app.settings.active_notices = false;
        app.numeric(311, "alice a host * :Alice".into());
        assert!(
            app.windows[chan]
                .lines
                .last()
                .unwrap()
                .plain()
                .contains("alice a host")
        );
        app.apply_event(Event::Notice {
            from: "bob".into(),
            text: "hello".into(),
        });
        assert!(
            app.windows[status]
                .lines
                .last()
                .unwrap()
                .plain()
                .contains("-bob- hello")
        );
        app.settings.active_ctcp = true;
        app.apply_event(Event::CtcpReply {
            from: "carol".into(),
            command: "VERSION".into(),
            text: "Other 1.0".into(),
        });
        assert!(
            app.windows[chan]
                .lines
                .last()
                .unwrap()
                .plain()
                .contains("[carol VERSION reply] Other 1.0")
        );
    }

    #[test]
    fn messages_window_and_query_limit() {
        let mut app = app();
        app.settings.query_limit = 2;
        for nick in ["a", "b", "c", "d"] {
            app.apply_event(Event::Chat {
                channel: false,
                target: nick.into(),
                from: nick.into(),
                text: "hi".into(),
                action: false,
            });
        }
        assert!(app.find(Kind::Query, "a").is_some());
        assert!(app.find(Kind::Query, "b").is_some());
        assert!(app.find(Kind::Query, "c").is_none(), "past the limit");
        let messages = app.find(Kind::List, MESSAGES).unwrap();
        assert_eq!(app.windows[messages].lines.len(), 3, "a note, then c and d");
        app.apply_event(Event::Chat {
            channel: false,
            target: "a".into(),
            from: "a".into(),
            text: "again".into(),
            action: false,
        });
        let a = app.find(Kind::Query, "a").unwrap();
        assert!(
            app.windows[a]
                .lines
                .last()
                .unwrap()
                .plain()
                .contains("again")
        );
        assert_eq!(app.last_private.get(&None).map(String::as_str), Some("d"));

        app.settings.message_window = true;
        app.query_opens.clear();
        app.apply_event(Event::Chat {
            channel: false,
            target: "e".into(),
            from: "e".into(),
            text: "one window".into(),
            action: false,
        });
        assert!(app.find(Kind::Query, "e").is_none());
        assert!(
            app.windows[messages]
                .lines
                .last()
                .unwrap()
                .plain()
                .contains("one window")
        );
    }

    #[test]
    fn ircv3_events_track_away_typing_and_server_time() {
        let mut app = app();
        app.sessions.push(test_session(7, false));
        app.routing = Some(7);
        let query = app.ensure(Kind::Query, "alice");
        app.apply_event(Event::UserAway {
            who: "alice".into(),
            reason: Some("lunch".into()),
        });
        assert_eq!(
            app.sessions[0].away_nicks.get("alice").map(String::as_str),
            Some("lunch")
        );
        assert!(
            app.windows[query]
                .lines
                .last()
                .unwrap()
                .plain()
                .contains("alice is away: lunch")
        );
        app.apply_event(Event::Typing {
            who: "alice".into(),
            target: "alice".into(),
            state: "active".into(),
        });
        app.routing = None;
        app.active = query;
        assert_eq!(app.typing_label().as_deref(), Some("alice is typing…"));
        app.routing = Some(7);
        let effect = Effect {
            lines: Vec::new(),
            events: vec![Event::Chat {
                channel: false,
                target: "alice".into(),
                from: "alice".into(),
                text: "back".into(),
                action: false,
            }],
            outgoing: Vec::new(),
            quit: false,
            sender: None,
            time: Some(0),
            renames: 0,
        };
        app.apply_effect(7, effect);
        let line = app.windows[query].lines.last().unwrap();
        assert_eq!(
            line.time,
            Some(crate::clock::from_unix(0)),
            "server time is used"
        );
        assert_eq!(app.typing_label(), None, "a message ends typing");
    }

    #[test]
    fn sts_policy_upgrades_plain_connections() {
        let mut app = app();
        app.settings.sts_policies.push((
            "irc.example".into(),
            6697,
            crate::rules::unix_now() + 3_600,
        ));
        assert_eq!(app.sts_upgrade("IRC.example", 6667, false), (6697, true));
        assert_eq!(app.sts_upgrade("other.example", 6667, false), (6667, false));
        app.settings.sts_policies[0].2 = 1;
        assert_eq!(app.sts_upgrade("irc.example", 6667, false), (6667, false));
        assert!(
            app.settings.sts_policies.is_empty(),
            "expired policies are dropped"
        );

        app.sessions.push(test_session(3, true));
        app.routing = Some(3);
        app.apply_event(Event::Sts {
            port: None,
            duration: Some(600),
        });
        assert_eq!(app.settings.sts_policies.len(), 1);
        assert_eq!(app.settings.sts_policies[0].1, 6697);
        app.apply_event(Event::Sts {
            port: None,
            duration: Some(0),
        });
        assert!(
            app.settings.sts_policies.is_empty(),
            "duration 0 removes it"
        );
    }

    #[test]
    fn logs_keep_codes_and_reload_when_a_window_opens() {
        let dir = std::env::temp_dir().join(format!("openirc-ui-logs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = app();
        app.settings.logging = true;
        app.settings.log_dir = dir.display().to_string();
        app.settings.log_keep_codes = true;
        app.settings.log_reload = 2;
        let index = app.ensure(Kind::Channel, "#logs");
        assert_eq!(app.windows[index].lines.len(), 0, "no log yet");
        for text in ["one", "\u{3}04two", "three"] {
            app.apply_event(Event::Chat {
                channel: true,
                target: "#logs".into(),
                from: "ann".into(),
                text: text.into(),
                action: false,
            });
        }
        let path = app.log_path_for(index);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("\u{3}04two\u{f}"), "{saved:?}");
        app.close_window(index);
        let reopened = app.ensure(Kind::Channel, "#logs");
        let lines = texts(&app, reopened);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[0].ends_with("two"));
        assert!(lines[1].ends_with("three"));
        assert!(app.windows[reopened].lines[0].time.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_history_empties_the_chosen_lists_only() {
        let mut app = app();
        app.history.push("/join #a");
        let urls = app.ensure(Kind::List, "URLs");
        app.push(urls, note("a: https://example.com"));
        app.find_text = "needle".into();
        app.clear_picked([true, true, false, false, false]);
        assert_eq!(app.history.len(), 0);
        assert!(app.windows[urls].lines.is_empty());
        assert_eq!(app.find_text, "needle", "not picked");
    }

    #[test]
    fn dcc_offers_are_refused_accepted_or_asked() {
        let dir = std::env::temp_dir().join(format!("openirc-ui-dcc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = app();
        app.settings.dcc_folder = dir.join("in").display().to_string();
        app.settings
            .dcc_folder_rules
            .push(("jpg png".into(), "Pictures".into()));
        app.settings.dcc_auto_accept.push("friend".into());
        let offer = |filename: &str, port: u16, size: u64| DccSendOffer {
            filename: filename.into(),
            host: "127.0.0.1".into(),
            port,
            size: Some(size),
            token: None,
        };

        app.incoming_file("someone".into(), offer("setup.exe", 1, 10));
        assert!(app.get_prompt.is_none() && app.send_offers.is_empty());
        assert!(texts(&app, 0).last().unwrap().contains("Refused setup.exe"));

        let source = dir.join("photo.jpg");
        std::fs::write(&source, vec![9u8; 50_000]).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let sender = std::thread::spawn(move || {
            crate::dcc_send::serve_file(listener, &source, &crate::dcc_send::Meter::default())
        });
        app.incoming_file("friend".into(), offer("photo.jpg", port, 50_000));
        assert_eq!(
            app.transfers.len(),
            1,
            "a trusted sender's file starts at once"
        );
        assert!(app.show_transfers);
        let started = std::time::Instant::now();
        while app.transfers[0].running() && started.elapsed() < Duration::from_secs(10) {
            app.poll_transfers();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!app.transfers[0].running());
        sender.join().unwrap().unwrap();
        let saved = dir.join("in").join("Pictures").join("photo.jpg");
        assert_eq!(std::fs::metadata(&saved).unwrap().len(), 50_000);

        app.incoming_file("stranger".into(), offer("notes.txt", 2, 5));
        assert!(app.get_prompt.is_some(), "others are asked");
        app.incoming_file("stranger".into(), offer("more.txt", 3, 5));
        assert_eq!(app.send_offers.len(), 1, "a second offer waits in the list");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_window_opened_in_the_background_keeps_the_active_one() {
        let mut app = app();
        let query = app.ensure(Kind::Query, "alice");
        app.activate(query);
        app.apply_event(Event::Topic {
            channel: "#elsewhere".into(),
            topic: Some("news".into()),
        });
        assert_eq!(app.windows[app.active].title, "alice");
    }

    #[test]
    fn windows_can_be_ordered_by_name() {
        let mut app = app();
        app.settings.window_order = "name".into();
        let zed = app.ensure(Kind::Query, "zed");
        app.activate(zed);
        app.ensure(Kind::Channel, "#zoo");
        app.ensure(Kind::Query, "amy");
        app.ensure(Kind::Channel, "#ant");
        let order: Vec<&str> = app
            .windows
            .iter()
            .filter(|window| matches!(window.kind, Kind::Channel | Kind::Query))
            .map(|window| window.title.as_str())
            .collect();
        assert_eq!(order, vec!["#ant", "#openirc", "#zoo", "amy", "zed"]);
        assert_eq!(app.windows[app.active].title, "zed");
    }

    #[test]
    fn server_command_opens_or_switches_connections() {
        let mut app = app();
        app.dispatch("/server 127.0.0.1 1".into(), Origin::Input);
        assert_eq!(
            app.sessions.len(),
            1,
            "the first /server opens a connection"
        );
        let first = app.sessions[0].id;
        app.routing = Some(first);
        let chan = app.ensure(Kind::Channel, "#old");
        app.routing = None;
        app.activate(chan);
        app.dispatch("/server -m 127.0.0.2 +2".into(), Origin::Input);
        assert_eq!(app.sessions.len(), 2, "-m adds a connection");
        assert!(app.sessions[1].tls && app.sessions[1].port == 2);
        let status = app
            .windows
            .iter()
            .position(|window| window.kind == Kind::Status && window.server == Some(first))
            .unwrap();
        app.activate(status);
        app.dispatch("/server -j #new 127.0.0.3 3".into(), Origin::Input);
        assert_eq!(
            app.sessions.len(),
            2,
            "without -m the current connection moves"
        );
        app.settings.servers.push(crate::servers::ServerEntry::new(
            "Local",
            "127.0.0.9",
            6697,
            true,
        ));
        app.dispatch("/server -m 127.0.0.9 6667".into(), Origin::Input);
        let plain = app.sessions.last().unwrap();
        assert!(
            !plain.tls,
            "port 6667 given: the saved entry's TLS stays with 6697"
        );
        assert_eq!(plain.port, 6667);
        app.dispatch("/server -m Local".into(), Origin::Input);
        let saved = app.sessions.last().unwrap();
        assert!(
            saved.tls && saved.port == 6697,
            "a network name finds the saved entry"
        );
        let moved = app
            .sessions
            .iter()
            .find(|session| session.id == first)
            .unwrap();
        assert_eq!(moved.host, "127.0.0.3");
        assert_eq!(moved.autojoin, vec!["#new".to_string()]);
        assert!(
            !app.windows.iter().any(|window| window.title == "#old"),
            "the old server's channel windows close"
        );
        for session in &mut app.sessions {
            if let Some(link) = session.link.take() {
                link.stop.store(true, Ordering::Relaxed);
            }
        }
    }

    #[test]
    fn window_commands_change_settings_and_windows() {
        let mut app = app();
        app.active = 0;
        app.dispatch("/notify ann".into(), Origin::Input);
        assert_eq!(app.settings.notify, vec!["ann".to_string()]);
        app.dispatch("/notify -r ANN".into(), Origin::Input);
        assert!(app.settings.notify.is_empty());
        app.dispatch("/alias hi /echo hello $1".into(), Origin::Input);
        app.dispatch("/hi there".into(), Origin::Input);
        assert_eq!(texts(&app, 0).last().unwrap(), "* hello there");
        app.dispatch("/alias hi".into(), Origin::Input);
        assert!(app.settings.aliases.is_empty());
        app.dispatch("/auser 10,friend *!*@home.example".into(), Origin::Input);
        assert_eq!(app.settings.users.len(), 1);
        app.dispatch("/ruser *!*@home.example".into(), Origin::Input);
        assert!(app.settings.users.is_empty());
        app.dispatch("/log on".into(), Origin::Input);
        assert!(app.settings.logging);
        app.settings.logging = false;
        app.dispatch("/timer 60 1 /echo later".into(), Origin::Input);
        app.dispatch("/timers off".into(), Origin::Input);
        assert!(app.timers.is_empty());
        let query = app.ensure(Kind::Query, "bob");
        app.dispatch("/echo bob for bob".into(), Origin::Input);
        assert_eq!(texts(&app, query).last().unwrap(), "* for bob");
        app.dispatch("/close -m".into(), Origin::Input);
        assert!(app.find(Kind::Query, "bob").is_none());
        app.dispatch("/editbox draft text".into(), Origin::Input);
        assert_eq!(app.input, "draft text");
        app.dispatch("/clearall".into(), Origin::Input);
        assert!(app.windows.iter().all(|window| window.lines.is_empty()));
    }

    #[test]
    fn short_server_labels() {
        assert_eq!(short_host("127.0.0.1"), "127.0.0.1");
        assert_eq!(short_host("irc.libera.chat"), "libera");
        assert_eq!(short_host("chat.freenode.net"), "freenode");
        assert_eq!(short_host("efnet.example"), "efnet");
        assert_eq!(short_host("irc.local"), "irc");
        assert_eq!(short_host("::1"), "::1");
    }

    #[test]
    fn join_names_are_summed_up_and_windows_can_use_fixed_width() {
        let mut app = app();
        app.apply_event(Event::Join {
            who: "me".into(),
            channel: "#big".into(),
            address: "me@host".into(),
        });
        let index = app.find(Kind::Channel, "#big").unwrap();
        app.apply_event(Event::Names {
            channel: "#big".into(),
            names: vec!["@op".into(), "+voice".into(), "plain".into()],
            listed: false,
        });
        let lines = texts(&app, index);
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Now talking in #big"))
        );
        assert_eq!(lines.last().unwrap(), "* #big: 3 users (1 op, 1 voiced)");
        assert_eq!(app.windows[index].nicks.len(), 3);
        app.apply_event(Event::Join {
            who: "ann".into(),
            channel: "#big".into(),
            address: "a@example".into(),
        });
        assert_eq!(
            texts(&app, index).last().unwrap(),
            "* ann (a@example) joins #big"
        );
        app.apply_event(Event::ChannelInfo {
            channel: "#big".into(),
            text: "#big has modes +nt".into(),
        });
        assert_eq!(texts(&app, index).last().unwrap(), "* #big has modes +nt");
        assert!(!app.window_mono(index));
        app.toggle_mono(index);
        assert!(app.window_mono(index));
        app.toggle_mono(index);
        assert!(!app.window_mono(index));
    }

    #[test]
    fn idle_time_sets_away_and_typing_ends_it() {
        let mut app = app();
        app.dispatch("/server 127.0.0.1 1".into(), Origin::Input);
        app.sessions[0].connected_at = Some(std::time::Instant::now());
        app.settings.auto_away_minutes = 5;
        app.last_input = std::time::Instant::now() - Duration::from_secs(4 * 60);
        app.check_auto_away();
        assert!(!app.auto_away_on, "four minutes is not enough");
        app.last_input = std::time::Instant::now() - Duration::from_secs(6 * 60);
        app.check_auto_away();
        assert!(app.auto_away_on);
        app.user_active();
        assert!(!app.auto_away_on);
        app.sessions[0].away = Some("lunch".into());
        app.last_input = std::time::Instant::now() - Duration::from_secs(6 * 60);
        app.check_auto_away();
        assert!(!app.auto_away_on, "a manual away is left alone");
        if let Some(link) = app.sessions[0].link.take() {
            link.stop.store(true, Ordering::Relaxed);
        }
    }

    #[test]
    fn the_last_sessions_windows_come_back() {
        let mut app = app();
        app.settings.last_windows.push((
            "127.0.0.1".into(),
            vec!["#kept".into()],
            vec!["ann".into()],
        ));
        app.form.channel = "#first".into();
        app.form.server = "127.0.0.1".into();
        app.form.port = "1".into();
        app.form.tls = false;
        app.form.nick = "me".into();
        app.connect();
        let session = &app.sessions[0];
        assert_eq!(
            session.autojoin,
            vec!["#first".to_string(), "#kept".to_string()]
        );
        let id = session.id;
        assert!(app.windows.iter().any(|window| window.kind == Kind::Query
            && window.title == "ann"
            && window.server == Some(id)));
        app.routing = Some(id);
        app.ensure(Kind::Channel, "#first");
        app.routing = None;
        app.remember_windows();
        let (_, channels, queries) = app.settings.last_windows[0].clone();
        assert_eq!(channels, vec!["#first".to_string()]);
        assert_eq!(queries, vec!["ann".to_string()]);
        if let Some(link) = app.sessions[0].link.take() {
            link.stop.store(true, Ordering::Relaxed);
        }
    }

    #[test]
    fn popups_go_to_their_own_menus() {
        let mut app = app();
        for (menu, label) in [("channel", "Rules"), ("query", "Whois"), ("line", "Quote")] {
            app.settings.popups.push(crate::settings::Popup {
                label: label.into(),
                body: "/echo $nick $chan".into(),
                menu: menu.into(),
            });
        }
        let chan = app.ensure(Kind::Channel, "#c");
        let query = app.ensure(Kind::Query, "ann");
        let labels = |items: Vec<(String, String, String, String)>| -> Vec<String> {
            items.into_iter().map(|item| item.0).collect()
        };
        assert_eq!(labels(app.window_popups(chan)), vec!["Rules"]);
        let on_query = app.window_popups(query);
        assert_eq!(on_query[0].0, "Whois");
        assert_eq!(on_query[0].2, "ann", "$nick is the query's nick");
        assert_eq!(app.popups_for("line").len(), 1);
    }

    #[test]
    fn the_channels_list_opens_on_connect_for_that_server() {
        let mut app = app();
        app.sessions.push(test_session(4, false));
        app.sessions.push(test_session(5, false));
        app.routing = Some(4);
        app.apply_event(Event::Registered {
            nick: "me".into(),
            text: "Welcome".into(),
        });
        app.routing = None;
        assert!(app.channel_list.open);
        assert_eq!(app.channel_list.server, Some(4));
        assert!(!app.channel_list.listing, "the list waits for Get list");
        app.routing = Some(5);
        app.numeric(322, "#other 50 :from the other server".into());
        app.routing = Some(4);
        app.numeric(322, "#mine 9 :ours".into());
        app.routing = None;
        let names: Vec<&str> = app
            .channel_list
            .entries()
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(names, vec!["#mine"]);
        app.settings.channels_on_connect = false;
        app.channel_list.open = false;
        app.routing = Some(5);
        app.apply_event(Event::Registered {
            nick: "me".into(),
            text: "Welcome".into(),
        });
        app.routing = None;
        assert!(!app.channel_list.open, "turned off");
    }

    #[test]
    fn auto_op_waits_for_op_status_and_protect_restores_it() {
        let mut app = app();
        let index = app.ensure(Kind::Channel, "#home");
        app.settings.auto_mode_delay = false;
        app.settings.auto_modes.push(crate::rules::AutoMode {
            mask: "friend".into(),
            channels: "#home".into(),
            network: String::new(),
            mode: 'o',
            protect: true,
        });
        app.windows[index].nicks = vec!["me".into()];
        app.apply_event(Event::Join {
            who: "friend".into(),
            channel: "#home".into(),
            address: "f@host".into(),
        });
        assert!(
            app.timers.is_empty(),
            "not an operator, so nothing is scheduled"
        );
        app.windows[index].nicks = vec!["@me".into()];
        app.apply_event(Event::Join {
            who: "friend".into(),
            channel: "#home".into(),
            address: "f@host".into(),
        });
        assert_eq!(app.timers.len(), 1);
        assert_eq!(app.timers[0].command, "/raw MODE #home +o friend");
    }

    #[test]
    fn did_fills_dialog_values() {
        let mut values = std::collections::HashMap::new();
        values.insert(("demo".to_string(), 2), "Ann".to_string());
        values.insert(("demo".to_string(), 3), "1".to_string());
        assert_eq!(
            fill_did("/echo $did(2) $did(3) $did(9) $did(", "demo", &values),
            "/echo Ann 1  $did("
        );
    }

    #[test]
    fn scrollback_is_capped() {
        let mut app = app();
        for n in 0..MAX_SCROLLBACK + 10 {
            app.push(0, note(format!("line {n}")));
        }
        assert_eq!(app.windows[0].lines.len(), MAX_SCROLLBACK);
        assert_eq!(app.windows[0].lines[0].plain(), "* line 10");
    }

    #[test]
    fn readable_colors_reach_aa_contrast() {
        let paper = egui::Color32::from_rgb(10, 10, 10);
        let ink = egui::Color32::WHITE;
        for (slot, &nick) in NICK_COLORS.iter().enumerate() {
            let color = readable(nick, paper, ink);
            assert!(contrast(color, paper) >= 4.5, "slot {slot}");
        }
        assert_eq!(
            text_on(egui::Color32::from_rgb(10, 36, 106), egui::Color32::BLACK),
            egui::Color32::WHITE
        );
    }

    /// Run one headless frame of the whole window with these input events.
    pub(super) fn frame(ctx: &egui::Context, app: &mut OpenIrcApp, events: Vec<egui::Event>) {
        use eframe::App;
        let mut frame = eframe::Frame::_new_kittest();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(980.0, 640.0),
            )),
            events,
            ..Default::default()
        };
        headless_frame(ctx, input, |ui| app.ui(ui, &mut frame));
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn focused_app() -> (egui::Context, OpenIrcApp) {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let mut app = app();
        app.active = 1;
        frame(&ctx, &mut app, Vec::new());
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("input-line")));
        frame(&ctx, &mut app, Vec::new());
        (ctx, app)
    }

    #[test]
    fn shortcuts_cycle_and_close_windows() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let mut app = app();
        let query = app.ensure(Kind::Query, "dave");
        app.active = 0;
        frame(&ctx, &mut app, Vec::new());
        frame(
            &ctx,
            &mut app,
            vec![key(egui::Key::Tab, egui::Modifiers::CTRL)],
        );
        assert_eq!(app.active, 1);
        app.activate(query);
        frame(
            &ctx,
            &mut app,
            vec![key(egui::Key::Z, egui::Modifiers::ALT)],
        );
        assert!(app.find(Kind::Query, "dave").is_none());
        frame(
            &ctx,
            &mut app,
            vec![key(egui::Key::Num1, egui::Modifiers::ALT)],
        );
        assert_eq!(app.active, 0);
    }

    #[test]
    fn input_history_completion_and_formatting_keys() {
        let (ctx, mut app) = focused_app();
        app.windows[1].nicks = vec!["@alice".into(), "bob".into()];
        app.history.push("/echo first");
        app.history.push("/echo second");
        frame(
            &ctx,
            &mut app,
            vec![key(egui::Key::ArrowUp, egui::Modifiers::NONE)],
        );
        assert_eq!(app.input, "/echo second");
        frame(
            &ctx,
            &mut app,
            vec![key(egui::Key::ArrowUp, egui::Modifiers::NONE)],
        );
        assert_eq!(app.input, "/echo first");
        frame(
            &ctx,
            &mut app,
            vec![key(egui::Key::ArrowDown, egui::Modifiers::NONE)],
        );
        frame(
            &ctx,
            &mut app,
            vec![key(egui::Key::ArrowDown, egui::Modifiers::NONE)],
        );
        assert_eq!(app.input, "");
        frame(&ctx, &mut app, vec![egui::Event::Text("al".into())]);
        frame(
            &ctx,
            &mut app,
            vec![key(egui::Key::Tab, egui::Modifiers::NONE)],
        );
        assert_eq!(app.input, "alice: ");
        frame(
            &ctx,
            &mut app,
            vec![key(egui::Key::B, egui::Modifiers::COMMAND)],
        );
        frame(&ctx, &mut app, vec![egui::Event::Text("hi".into())]);
        assert_eq!(app.input, "alice: \u{2}hi");
    }

    #[test]
    fn multiline_input_sends_on_enter_and_breaks_on_shift_enter() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let mut app = app();
        app.settings.multiline_input = true;
        app.active = 0;
        frame(&ctx, &mut app, Vec::new());
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("input-line")));
        frame(&ctx, &mut app, Vec::new());
        frame(&ctx, &mut app, vec![egui::Event::Text("/echo one".into())]);
        frame(
            &ctx,
            &mut app,
            vec![key(egui::Key::Enter, egui::Modifiers::SHIFT)],
        );
        frame(&ctx, &mut app, vec![egui::Event::Text("/echo two".into())]);
        assert_eq!(app.input, "/echo one\n/echo two");
        frame(
            &ctx,
            &mut app,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        );
        assert!(app.input.is_empty());
        let lines = texts(&app, 0);
        assert!(
            lines.ends_with(&["* one".to_string(), "* two".to_string()]),
            "{lines:?}"
        );
    }

    #[test]
    fn prefix_separator_double_click_and_drop_commands() {
        let mut app = app();
        app.active = 0;
        app.settings.command_prefix = ".".into();
        app.dispatch(".echo dotted".into(), Origin::Input);
        assert_eq!(texts(&app, 0).last().unwrap(), "* dotted");
        app.dispatch(". not a command".into(), Origin::Script);
        assert_ne!(texts(&app, 0).last().unwrap(), "* not a command");

        app.settings.separator = ";".into();
        app.settings.aliases.push(crate::settings::Alias {
            name: "both".into(),
            body: "/echo first ; /echo second".into(),
        });
        app.dispatch("/both".into(), Origin::Input);
        assert!(texts(&app, 0).ends_with(&["* first".to_string(), "* second".to_string()]));

        let query = app.ensure(Kind::Query, "alice");
        app.active = query;
        app.settings.dclick_query = "/echo who is $nick".into();
        app.run_line_action("", LineAction::WindowDouble);
        assert_eq!(texts(&app, query).last().unwrap(), "* who is alice");

        app.settings.drop_command = "/echo send $file to $nick".into();
        app.drop_files(vec![std::path::PathBuf::from("/tmp/a b.txt")]);
        assert_eq!(
            texts(&app, query).last().unwrap(),
            "* send /tmp/a b.txt to alice"
        );
        let chan = app.ensure(Kind::Channel, "#c");
        app.active = chan;
        app.selected_nick = None;
        app.drop_files(vec![std::path::PathBuf::from("/tmp/x")]);
        assert!(
            texts(&app, chan)
                .last()
                .unwrap()
                .contains("Drop the file on a nick")
        );
        app.selected_nick = Some("bob".into());
        app.drop_files(vec![std::path::PathBuf::from("/tmp/x")]);
        assert_eq!(texts(&app, chan).last().unwrap(), "* send /tmp/x to bob");
    }

    #[test]
    fn pasting_many_lines_asks_first_and_few_lines_send() {
        let (ctx, mut app) = focused_app();
        app.preview = false;
        app.active = 0;
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("input-line")));
        frame(&ctx, &mut app, Vec::new());
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::Paste("/echo a\n/echo b".into())],
        );
        let lines = texts(&app, 0);
        assert!(
            lines.ends_with(&["* a".to_string(), "* b".to_string()]),
            "{lines:?}"
        );
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::Paste("one\ntwo\nthree\nfour\nfive".into())],
        );
        assert_eq!(app.paste_pending.as_ref().map(Vec::len), Some(5));
    }

    #[test]
    fn a_frame_lays_out_the_nick_list_inside_the_window() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let mut app = app();
        app.active = 1;
        app.windows[1].nicks = vec!["@alice".into(), "zed".into()];
        let mut frame = eframe::Frame::_new_kittest();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(980.0, 640.0),
            )),
            ..Default::default()
        };
        use eframe::App;
        headless_frame(&ctx, input, |ui| app.ui(ui, &mut frame));
        let rect = egui::containers::panel::PanelState::load(&ctx, egui::Id::new("client-nicks"))
            .expect("the nick list panel was laid out")
            .outer_rect;
        assert!(rect.width() > 50.0, "nick list has no width: {rect:?}");
        assert!(
            rect.right() <= 980.0 + 0.5,
            "nick list ends off screen: {rect:?}"
        );
    }

    #[test]
    fn mouse_side_buttons_step_through_windows() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let mut app = app();
        app.ensure(Kind::Query, "dave");
        app.active = 0;
        frame(&ctx, &mut app, Vec::new());
        let pos = egui::pos2(500.0, 300.0);
        let press = |button| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        frame(&ctx, &mut app, press(egui::PointerButton::Extra2));
        assert_eq!(app.active, 1, "forward goes to the next window");
        frame(&ctx, &mut app, press(egui::PointerButton::Extra1));
        assert_eq!(app.active, 0, "back goes to the previous window");
    }

    #[test]
    fn chat_lines_show_the_rank_and_the_chosen_brackets() {
        let mut app = app();
        let index = app.ensure(Kind::Channel, "#c");
        app.windows[index].nicks = vec!["@ann".into(), "bob".into()];
        let say = |app: &mut OpenIrcApp, from: &str, action: bool| {
            app.apply_event(crate::pump::Event::Chat {
                channel: true,
                target: "#c".into(),
                from: from.into(),
                text: "hi".into(),
                action,
            });
            app.windows[index].lines.last().unwrap().plain()
        };
        assert_eq!(say(&mut app, "ann", false), "<ann> hi");
        app.settings.nick_rank = true;
        assert_eq!(say(&mut app, "ann", false), "<@ann> hi");
        assert_eq!(say(&mut app, "bob", false), "<bob> hi");
        assert_eq!(say(&mut app, "ann", true), "• @ann hi");
        app.settings.nick_open = "[".into();
        app.settings.nick_close = "]".into();
        assert_eq!(say(&mut app, "ann", false), "[@ann] hi");
        app.settings.nick_open.clear();
        app.settings.nick_close = ":".into();
        app.settings.nick_rank = false;
        assert_eq!(say(&mut app, "bob", false), "bob: hi");
    }

    #[test]
    fn web_search_addresses_are_encoded() {
        assert_eq!(
            search_address("https://x.example/?q=%s&ia=web", "a&b c"),
            "https://x.example/?q=a%26b+c&ia=web"
        );
        assert_eq!(
            search_address("https://x.example/search?q=", "café"),
            "https://x.example/search?q=caf%C3%A9"
        );
        assert!(search_address("", "rust").starts_with("https://duckduckgo.com/?q=rust"));
    }

    #[test]
    fn spell_check_leaves_the_channel_nicks_alone() {
        let mut app = app();
        let index = app.ensure(Kind::Channel, "#c");
        app.windows[index].nicks = vec!["@zorblax".into()];
        app.activate(index);
        app.dictionary = crate::spell::load("hello there");
        app.update_spell_skip();
        let text = "zorblax: hello thar";
        let misses = crate::spell::miss_ranges(text, &app.dictionary, &app.spell_skip, false);
        assert_eq!(misses, vec![(15, 19)], "only thar");
        // A rename keeps the count but changes who is skipped.
        app.windows[index].nicks = vec!["@quuxly".into()];
        app.update_spell_skip();
        assert!(app.spell_skip.contains("quuxly"));
        assert!(!app.spell_skip.contains("zorblax"));
    }

    #[test]
    fn a_missing_font_is_reported_once_and_found_fonts_are_ordered() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let mut app = app();
        let status = app.status_index().unwrap();
        let before = app.windows[status].lines.len();
        app.font_scripts.push(crate::fonts::Script::Thai);
        app.font_tx
            .send(FontLoad {
                script: crate::fonts::Script::Thai,
                loaded: None,
            })
            .unwrap();
        app.poll_fonts(&ctx);
        let note = app.windows[status].lines.last().unwrap().plain();
        assert_eq!(app.windows[status].lines.len(), before + 1);
        assert!(note.contains("No font for Thai"), "{note}");
        // Asked once per script: more Thai text asks for nothing new.
        app.want_fonts("สวัสดี");
        assert_eq!(app.font_scripts, vec![crate::fonts::Script::Thai]);
        // DejaVu's bytes stand in for two fonts that arrive out of order.
        for (script, name) in [
            (crate::fonts::Script::Kana, "kana.ttf"),
            (crate::fonts::Script::Han, "han.ttf"),
        ] {
            app.font_tx
                .send(FontLoad {
                    script,
                    loaded: Some((
                        name.into(),
                        0,
                        include_bytes!("../assets/DejaVuSans.ttf").to_vec(),
                    )),
                })
                .unwrap();
        }
        app.poll_fonts(&ctx);
        let names: Vec<&str> = app
            .extra_fonts
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(names, vec!["han.ttf", "kana.ttf"]);
    }

    #[test]
    fn faces_of_one_collection_are_kept_apart_in_a_fixed_order() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let bytes = include_bytes!("../assets/DejaVuSans.ttf").to_vec();
        let mut results = Vec::new();
        for order in [
            [
                (crate::fonts::Script::Han, 2),
                (crate::fonts::Script::Kana, 0),
            ],
            [
                (crate::fonts::Script::Kana, 0),
                (crate::fonts::Script::Han, 2),
            ],
        ] {
            let mut app = app();
            for (script, index) in order {
                app.font_tx
                    .send(FontLoad {
                        script,
                        loaded: Some(("cjk.ttc".into(), index, bytes.clone())),
                    })
                    .unwrap();
            }
            // The same file and face for a second script adds nothing.
            app.font_tx
                .send(FontLoad {
                    script: crate::fonts::Script::Hangul,
                    loaded: Some(("cjk.ttc".into(), 0, bytes.clone())),
                })
                .unwrap();
            app.poll_fonts(&ctx);
            let names: Vec<String> = app
                .extra_fonts
                .iter()
                .map(|(name, _)| name.clone())
                .collect();
            results.push(names);
        }
        assert_eq!(
            results[0],
            vec!["cjk.ttc#2".to_string(), "cjk.ttc".to_string()]
        );
        assert_eq!(results[0], results[1]);
    }

    /// A TrueType collection holding `faces`, in order: each face's tables
    /// copied in and its table offsets pointed at the copies.
    fn collection(faces: &[&[u8]]) -> Vec<u8> {
        let be32 =
            |bytes: &[u8], at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
        let be16 =
            |bytes: &[u8], at: usize| u16::from_be_bytes(bytes[at..at + 2].try_into().unwrap());
        let header = 12 + 4 * faces.len();
        let directories: usize = faces
            .iter()
            .map(|face| 12 + 16 * usize::from(be16(face, 4)))
            .sum();
        let mut out = Vec::new();
        out.extend_from_slice(b"ttcf");
        out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        out.extend_from_slice(&(faces.len() as u32).to_be_bytes());
        let mut directory_at = header;
        for face in faces {
            out.extend_from_slice(&(directory_at as u32).to_be_bytes());
            directory_at += 12 + 16 * usize::from(be16(face, 4));
        }
        let mut tables = Vec::new();
        let table_at = header + directories;
        for face in faces {
            let count = usize::from(be16(face, 4));
            out.extend_from_slice(&face[..12]);
            for record in 0..count {
                let at = 12 + 16 * record;
                let (offset, length) = (be32(face, at + 8) as usize, be32(face, at + 12) as usize);
                out.extend_from_slice(&face[at..at + 8]);
                out.extend_from_slice(&((table_at + tables.len()) as u32).to_be_bytes());
                out.extend_from_slice(&(length as u32).to_be_bytes());
                tables.extend_from_slice(&face[offset..offset + length]);
                while tables.len() % 4 != 0 {
                    tables.push(0);
                }
            }
        }
        out.extend_from_slice(&tables);
        out
    }

    #[test]
    fn the_chosen_face_of_a_real_collection_is_the_one_drawn() {
        let sans: &[u8] = include_bytes!("../assets/DejaVuSans.ttf");
        let mono: &[u8] = include_bytes!("../assets/DejaVuSansMono.ttf");
        let ttc = collection(&[sans, mono]);
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let mut app = app();
        app.font_tx
            .send(FontLoad {
                script: crate::fonts::Script::Han,
                loaded: Some(("cjk.ttc".into(), 1, ttc)),
            })
            .unwrap();
        app.poll_fonts(&ctx);
        assert_eq!(app.extra_fonts[0].0, "cjk.ttc#1");
        // Draw with that font alone: face 1, the fixed-width one, gives
        // `iiii` and `MMMM` the same width; face 0 would not.
        let mut only = egui::FontDefinitions::empty();
        only.font_data
            .insert("face".into(), app.extra_fonts[0].1.clone());
        only.families
            .insert(egui::FontFamily::Proportional, vec!["face".into()]);
        let test = egui::Context::default();
        test.set_fonts(only);
        let mut widths = Vec::new();
        headless_frame(&test, egui::RawInput::default(), |ui| {
            for text in ["iiii", "MMMM"] {
                let galley = ui.fonts_mut(|fonts| {
                    fonts.layout_no_wrap(
                        text.into(),
                        egui::FontId::proportional(14.0),
                        egui::Color32::WHITE,
                    )
                });
                widths.push(galley.size().x);
            }
        });
        assert_eq!(widths.len(), 2);
        assert!((widths[0] - widths[1]).abs() < 0.5, "{widths:?}");
    }

    #[test]
    fn a_face_two_scripts_share_takes_the_place_of_the_earlier_one() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let bytes = include_bytes!("../assets/DejaVuSans.ttf").to_vec();
        let load = |script, file: &str| FontLoad {
            script,
            loaded: Some((file.into(), 0, bytes.clone())),
        };
        use crate::fonts::Script::{Han, Hangul, Kana};
        let mut results = Vec::new();
        for order in [
            [
                (Hangul, "shared.ttf"),
                (Kana, "kana.ttf"),
                (Han, "shared.ttf"),
            ],
            [
                (Han, "shared.ttf"),
                (Kana, "kana.ttf"),
                (Hangul, "shared.ttf"),
            ],
        ] {
            let mut app = app();
            // One at a time, as loads finish.
            for (script, file) in order {
                app.font_tx.send(load(script, file)).unwrap();
                app.poll_fonts(&ctx);
            }
            let names: Vec<String> = app
                .extra_fonts
                .iter()
                .map(|(name, _)| name.clone())
                .collect();
            results.push(names);
        }
        // Han comes before Kana, so the face Han shares goes first, however
        // late Han's own load came.
        assert_eq!(
            results[0],
            vec!["shared.ttf".to_string(), "kana.ttf".to_string()]
        );
        assert_eq!(results[0], results[1]);
    }

    /// A connected session whose outgoing lines the test can read.
    pub(super) fn linked_session(id: u64) -> (Session, std::sync::mpsc::Receiver<String>) {
        let (outgoing, sent) = std::sync::mpsc::channel();
        let (_feed, incoming) = std::sync::mpsc::channel();
        let mut session = test_session(id, false);
        session.hangup = false;
        session.link = Some(Link {
            stop: Arc::new(AtomicBool::new(false)),
            debug: Arc::new(AtomicBool::new(false)),
            outgoing,
            incoming,
        });
        (session, sent)
    }

    /// What the window sent to a server: each command as typed, without
    /// the `/via <window>` it travels in, and without the quiet `/target`
    /// lines a window switch sends. `sent_raw` keeps them.
    pub(super) fn sent(rx: &std::sync::mpsc::Receiver<String>) -> Vec<String> {
        rx.try_iter()
            .filter(|line| !line.starts_with("/target -q "))
            .map(|line| match line.strip_prefix("/via ") {
                Some(rest) => {
                    let rest = match rest.strip_prefix('=') {
                        Some(run) => run.split_once(' ').map_or(run, |(_, rest)| rest),
                        None => rest,
                    };
                    rest.split_once(' ')
                        .map_or(rest, |(_, line)| line)
                        .to_string()
                }
                None => line,
            })
            .filter(|line| line != "/target -q")
            .collect()
    }

    /// What the window's lines say on the wire, through a real server side:
    /// each PRIVMSG, read back as the `/msg` or `/describe` it amounts to.
    pub(super) fn said(rx: &std::sync::mpsc::Receiver<String>) -> Vec<String> {
        let mut server = engine(&[]);
        rx.try_iter()
            .flat_map(|line| server.on_user_line(&line).outgoing)
            .filter_map(|line| {
                let (target, text) = line.strip_prefix("PRIVMSG ")?.split_once(" :")?;
                let action = text
                    .strip_prefix("\u{1}ACTION ")
                    .and_then(|text| text.strip_suffix('\u{1}'));
                Some(match action {
                    Some(action) => format!("/describe {target} {action}"),
                    None => format!("/msg {target} {text}"),
                })
            })
            .collect()
    }

    /// Two connected servers, 1 and 2, each with an `#ops` window where we
    /// are an operator. The window on server 2 is in front.
    pub(super) fn two_servers() -> (
        OpenIrcApp,
        std::sync::mpsc::Receiver<String>,
        std::sync::mpsc::Receiver<String>,
        usize,
        usize,
    ) {
        let mut app = app();
        let (one, rx1) = linked_session(1);
        let (two, rx2) = linked_session(2);
        app.sessions.push(one);
        app.sessions.push(two);
        app.routing = Some(1);
        let ops1 = app.ensure(Kind::Channel, "#ops");
        app.windows[ops1].nicks = vec!["@me".into()];
        app.routing = Some(2);
        let ops2 = app.ensure(Kind::Channel, "#ops");
        app.windows[ops2].nicks = vec!["@me".into()];
        app.routing = None;
        app.active = ops2;
        (app, rx1, rx2, ops1, ops2)
    }

    #[test]
    fn an_auto_op_timer_runs_on_the_server_it_was_set_on() {
        let (mut app, rx1, rx2, _, _) = two_servers();
        app.settings.auto_mode_delay = false;
        app.settings.auto_modes.push(crate::rules::AutoMode {
            mask: "friend".into(),
            channels: "#ops".into(),
            network: String::new(),
            mode: 'o',
            protect: false,
        });
        app.routing = Some(1);
        app.apply_event(Event::Join {
            who: "friend".into(),
            channel: "#ops".into(),
            address: "f@host".into(),
        });
        app.routing = None;
        app.tick_timers();
        assert_eq!(sent(&rx1), vec!["/raw MODE #ops +o friend".to_string()]);
        assert!(sent(&rx2).is_empty(), "the server in front gets nothing");

        // A timer whose server has been closed ends without running.
        app.routing = Some(1);
        app.dispatch("/timer 1 1 /raw PING x".into(), Origin::Input);
        app.routing = None;
        assert_eq!(app.timers.len(), 1, "the timer was set");
        app.timers[0].next = std::time::Instant::now();
        app.sessions.retain(|session| session.id != 1);
        app.tick_timers();
        assert!(app.timers.is_empty());
        assert!(sent(&rx2).is_empty());
    }

    fn hung_up(app: &OpenIrcApp, id: u64) -> bool {
        app.sessions
            .iter()
            .find(|session| session.id == id)
            .is_some_and(|session| session.hangup)
    }

    fn fire_timers_now(app: &mut OpenIrcApp) {
        let now = std::time::Instant::now();
        for timer in &mut app.timers {
            timer.next = now;
        }
        app.tick_timers();
    }

    #[test]
    fn every_command_a_timer_runs_stays_on_the_server_it_was_set_on() {
        let (mut app, rx1, rx2, _, ops2) = two_servers();
        // /debug changes windows and routing; the MODE after it must still
        // go to server 1.
        app.settings.aliases.push(crate::settings::Alias {
            name: "later".into(),
            body: "/debug on | /raw MODE #ops +o friend".into(),
        });
        app.routing = Some(1);
        app.dispatch("/timer 1 1 /later".into(), Origin::Input);
        app.routing = None;
        fire_timers_now(&mut app);
        assert_eq!(sent(&rx1), vec!["/raw MODE #ops +o friend".to_string()]);
        assert!(sent(&rx2).is_empty(), "server 2 gets nothing");
        let debugging = |app: &OpenIrcApp, id: u64| {
            app.sessions
                .iter()
                .find(|session| session.id == id)
                .and_then(|session| session.link.as_ref())
                .is_some_and(|link| link.debug.load(Ordering::Relaxed))
        };
        assert!(debugging(&app, 1), "/debug shows server 1's lines");
        assert!(!debugging(&app, 2));

        // /disconnect from a timer hangs up the timer's server.
        app.active = ops2;
        app.routing = Some(1);
        app.dispatch("/timer 1 1 /disconnect".into(), Origin::Input);
        app.routing = None;
        fire_timers_now(&mut app);
        assert_eq!(sent(&rx1), vec!["/quit".to_string()]);
        assert!(sent(&rx2).is_empty());
        assert!(hung_up(&app, 1));
        assert!(!hung_up(&app, 2), "the server in front still reconnects");
    }

    #[test]
    fn a_servers_own_notes_stay_in_its_windows() {
        let (mut app, _rx1, _rx2, _, _) = two_servers();
        for id in [1, 2] {
            app.routing = Some(id);
            app.ensure(Kind::Status, "Status");
        }
        app.routing = None;
        let ops2 = app
            .windows
            .iter()
            .position(|window| window.kind == Kind::Channel && window.server == Some(2))
            .unwrap();
        app.active = ops2;
        let effect = |text: &str| Effect {
            lines: vec![format!("* {text}")],
            events: Vec::new(),
            outgoing: Vec::new(),
            quit: false,
            time: None,
            sender: None,
            renames: 0,
        };
        app.apply_effect(1, effect("disconnected"));
        assert!(
            !texts(&app, ops2)
                .iter()
                .any(|line| line.contains("disconnected"))
        );
        let status1 = app
            .windows
            .iter()
            .position(|window| window.kind == Kind::Status && window.server == Some(1))
            .unwrap();
        assert!(
            texts(&app, status1)
                .iter()
                .any(|line| line.contains("disconnected"))
        );
        // The server in front shows its own notes where the user is.
        app.apply_effect(2, effect("pong"));
        assert!(texts(&app, ops2).iter().any(|line| line.contains("pong")));
    }

    #[test]
    fn a_quit_on_one_server_leaves_the_other_ready_to_reconnect() {
        let (mut app, rx1, rx2, ops1, ops2) = two_servers();
        app.active = ops1;
        app.open_prompt(prompts::PromptKind::Quit);
        let prompt = app.prompt.take().unwrap();
        // The user looks at server 2 before confirming.
        app.active = ops2;
        app.finish_prompt(&prompt, &["bye".to_string()]).unwrap();
        assert_eq!(sent(&rx1), vec!["/quit bye".to_string()]);
        assert!(sent(&rx2).is_empty());
        assert!(hung_up(&app, 1));
        assert!(!hung_up(&app, 2));

        // A timer's /quit hangs up its own server too.
        let (mut app, rx1, rx2, _, _) = two_servers();
        app.routing = Some(1);
        app.dispatch("/timer 1 1 /quit later".into(), Origin::Input);
        app.routing = None;
        fire_timers_now(&mut app);
        assert_eq!(sent(&rx1), vec!["/quit later".to_string()]);
        assert!(sent(&rx2).is_empty());
        assert!(hung_up(&app, 1));
        assert!(!hung_up(&app, 2));
    }

    #[test]
    fn names_compare_the_way_their_own_server_does() {
        let (mut app, _rx1, _rx2, _, ops2) = two_servers();
        // Server 1 says [ and { differ; server 2, in front, says they are
        // the same.
        app.sessions[0].casemap = crate::state::CaseMap::Ascii;
        app.routing = Some(1);
        let square = app.ensure(Kind::Query, "bob[a]");
        let curly = app.ensure(Kind::Query, "bob{a}");
        app.routing = None;
        assert_ne!(square, curly);
        app.active = ops2;
        set_casemap(crate::state::CaseMap::Rfc1459);
        app.dispatch("/scon -a /closemsg bob{a}".into(), Origin::Input);
        let queries: Vec<String> = app
            .windows
            .iter()
            .filter(|window| window.kind == Kind::Query)
            .map(|window| window.title.clone())
            .collect();
        assert_eq!(queries, vec!["bob[a]".to_string()]);
    }

    #[test]
    fn channel_central_reads_and_changes_its_own_channel() {
        let (mut app, rx1, rx2, _, ops2) = two_servers();
        app.sessions[0].casemap = crate::state::CaseMap::Ascii;
        app.routing = Some(1);
        let square = app.ensure(Kind::Channel, "#ops[1]");
        let curly = app.ensure(Kind::Channel, "#ops{1}");
        app.routing = None;
        app.active = curly;
        app.open_channel_central();
        let _ = sent(&rx1);
        for index in [square, curly] {
            app.windows[index].mode_lists.clear();
        }
        // Server 2's window is in front, and names compare its way.
        app.active = app
            .windows
            .iter()
            .position(|w| w.server == Some(2))
            .unwrap_or(ops2);
        set_casemap(crate::state::CaseMap::Rfc1459);
        let mut central = app.central.take().unwrap();
        app.central_actions(&mut central, curly, Some('e'), None, None, false);
        assert!(app.windows[curly].mode_lists.contains_key(&'e'));
        assert!(!app.windows[square].mode_lists.contains_key(&'e'));
        assert_eq!(sent(&rx1), vec!["/raw MODE #ops{1} +e".to_string()]);
        assert!(sent(&rx2).is_empty());

        // The form takes the modes of its own channel, not the other one.
        app.windows[curly].modes = "+s".into();
        app.central = Some(central);
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        frame(&ctx, &mut app, Vec::new());
        assert_eq!(app.central.as_ref().map(|c| c.loaded.as_str()), Some("+s"));
    }

    #[test]
    fn saved_history_leaves_out_passwords_however_they_are_typed() {
        let mut app = app();
        app.settings.keep_history = true;
        app.settings.command_prefix = "#".into();
        app.settings.aliases.push(crate::settings::Alias {
            name: "id".into(),
            body: "/msg NickServ IDENTIFY $1".into(),
        });
        app.settings.aliases.push(crate::settings::Alias {
            name: "go".into(),
            body: "/id $1".into(),
        });
        for (name, body) in [("a1", "/a2 $1"), ("a2", "/a3 $1"), ("a3", "/id $1")] {
            app.settings.aliases.push(crate::settings::Alias {
                name: name.into(),
                body: body.into(),
            });
        }
        // An alias that calls itself is read once, not forever.
        app.settings.aliases.push(crate::settings::Alias {
            name: "loop".into(),
            body: "/loop $1".into(),
        });
        let secret = [
            "#oper admin swordfish",
            "/key swordfish",
            "/mode #room -l+k swordfish",
            "/server irc.example:6697 swordfish",
            "/id swordfish",
            "#go swordfish",
            "#a1 swordfish",
        ];
        for line in secret
            .iter()
            .chain(&["hello there", "/join #rust", "/loop x"])
        {
            app.history.push(line);
        }
        app.save_history();
        assert_eq!(
            app.settings.history,
            vec![
                "hello there".to_string(),
                "/join #rust".to_string(),
                "/loop x".to_string()
            ]
        );
    }

    #[test]
    fn a_flood_waits_in_the_connection_instead_of_in_memory() {
        use std::io::Write as _;
        const LINES: usize = 40_000;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let finished = Arc::new(AtomicBool::new(false));
        let done = finished.clone();
        let server = thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut flood = Vec::new();
            for number in 0..LINES {
                writeln!(flood, ":irc 372 me :{number:06} {}\r", "x".repeat(200)).unwrap();
            }
            // 8 MB, far more than the socket buffers hold.
            let _ = sock.write_all(&flood);
            done.store(true, Ordering::Relaxed);
            thread::sleep(Duration::from_millis(500));
        });
        let config = Config::new("me", "me", "Me", Vec::new(), Vec::new());
        let link = spawn_link(
            "127.0.0.1".into(),
            port,
            false,
            config,
            None,
            connection::ConnectOptions::default(),
        );
        // Nobody takes messages for a while: the thread stops at the limit
        // and the rest stays with the server.
        thread::sleep(Duration::from_millis(1500));
        assert!(!finished.load(Ordering::Relaxed), "the server is held back");
        // Taking them lets everything through: each numbered line exactly
        // once, in order.
        let mut numbers = Vec::with_capacity(LINES);
        let started = std::time::Instant::now();
        while numbers.len() < LINES && started.elapsed() < Duration::from_secs(30) {
            match link.incoming.recv_timeout(Duration::from_secs(5)) {
                Ok(NetMsg::Effect(effect)) => {
                    // As the window reads it: the events, else the lines.
                    let texts: Vec<String> = if effect.events.is_empty() {
                        effect.lines.clone()
                    } else {
                        effect
                            .events
                            .iter()
                            .map(crate::display::format_event)
                            .collect()
                    };
                    for text in texts {
                        if let Some(number) = text.split_whitespace().find(|word| {
                            word.len() == 6 && word.bytes().all(|b| b.is_ascii_digit())
                        }) {
                            numbers.push(number.parse::<usize>().unwrap());
                        }
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        assert_eq!(numbers.len(), LINES, "every line arrived once");
        // Nothing numbered comes after the last line: read on until the
        // server hangs up.
        loop {
            match link.incoming.recv_timeout(Duration::from_secs(5)) {
                Ok(NetMsg::Effect(effect)) => {
                    let texts: Vec<String> = if effect.events.is_empty() {
                        effect.lines.clone()
                    } else {
                        effect
                            .events
                            .iter()
                            .map(crate::display::format_event)
                            .collect()
                    };
                    assert!(
                        texts.iter().all(|text| !text.contains(" xxxxxxxx")),
                        "a line after the end: {texts:?}"
                    );
                }
                // The server hung up, or its thread ended: the stream is
                // over. A wait with nothing at all fails instead.
                Ok(NetMsg::Failed(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    panic!("no end of stream within 5 seconds of the last line")
                }
                Ok(_) => {}
            }
        }
        let wrong = numbers
            .iter()
            .enumerate()
            .find(|(at, number)| at != *number);
        assert!(
            wrong.is_none(),
            "{wrong:?} near {:?}",
            wrong.map(|(at, _)| &numbers[at.saturating_sub(2)..(at + 3).min(numbers.len())])
        );
        assert!(finished.load(Ordering::Relaxed));
        link.stop.store(true, Ordering::Relaxed);
        drop(link);
        server.join().unwrap();
    }

    #[test]
    fn the_queue_to_the_window_holds_the_limit_and_no_more() {
        let (tx, rx) = ingress();
        for _ in 0..INGRESS_LIMIT {
            tx.try_send(NetMsg::Queued(0)).unwrap();
        }
        assert!(matches!(
            tx.try_send(NetMsg::Queued(0)),
            Err(mpsc::TrySendError::Full(_))
        ));
        // Taking one makes room for one.
        rx.try_recv().unwrap();
        tx.try_send(NetMsg::Queued(0)).unwrap();
        assert!(tx.try_send(NetMsg::Queued(0)).is_err());
    }

    #[test]
    fn a_hidden_window_comes_back_when_the_tray_goes_away() {
        // Embedded and minimized: hide.
        assert_eq!(tray_step(false, true, true, true), Some(TrayStep::Hide));
        // Hidden while the icon is there: stay hidden.
        assert_eq!(tray_step(true, true, true, true), None);
        // The tray or panel is gone: show the window again.
        assert_eq!(tray_step(true, true, true, false), Some(TrayStep::Show));
        // Without an icon in a tray it never hides.
        assert_eq!(tray_step(false, true, true, false), None);
        assert_eq!(tray_step(false, false, true, true), None);
    }

    #[test]
    fn added_words_go_beside_the_list_and_come_back_after_a_restart() {
        let dir = std::env::temp_dir().join(format!("openirc-ui-dict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let list = dir.join("words.txt");
        // A Windows-1252 list without a final line end.
        let original = b"caf\xe9\nhello".to_vec();
        std::fs::write(&list, &original).unwrap();
        let mut app = app();
        app.settings.dictionary = list.display().to_string();
        app.load_dictionary();
        assert!(app.dictionary.contains("café"));
        app.add_to_dictionary("Größe");
        app.add_to_dictionary("naïve");
        assert!(app.dictionary.contains("größe"));
        // The list itself is untouched, in its own encoding.
        assert_eq!(std::fs::read(&list).unwrap(), original);
        let added = crate::spell::added_path(&list);
        assert_eq!(std::fs::read_to_string(&added).unwrap(), "größe\nnaïve\n");
        // A new start reads both.
        let mut again = OpenIrcApp::with_settings(app.settings.clone(), &[], None);
        again.load_dictionary();
        for word in ["café", "hello", "größe", "naïve"] {
            assert!(again.dictionary.contains(word), "{word}");
        }
        // A list that cannot be read turns spell checking off and says so.
        let mut missing = super::tests::app();
        missing.settings.dictionary = dir.join("none.txt").display().to_string();
        missing.load_dictionary();
        assert!(missing.dictionary.is_empty());
        assert!(
            texts(&missing, 0)
                .iter()
                .any(|line| line.contains("Spell checking is off"))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_line_that_sent_a_password_through_a_script_is_never_saved() {
        let (mut app, rx1, _rx2, ops1, _) = two_servers();
        app.active = ops1;
        app.settings.keep_history = true;
        app.settings.script_text = "alias signin { msg NickServ IDENTIFY $1 }".into();
        for (name, body) in [
            ("outer", "/middle $1"),
            ("middle", "/inner $1"),
            ("inner", "/msg NickServ IDENTIFY $1"),
            ("later", "/timer 30 1 /msg NickServ IDENTIFY $1"),
        ] {
            app.settings.aliases.push(crate::settings::Alias {
                name: name.into(),
                body: body.into(),
            });
        }
        for line in [
            "hello there",
            "/signin swordfish",
            "/outer swordfish",
            "/later swordfish",
            "/join #rust",
        ] {
            app.input = line.into();
            app.enter_line();
        }
        // The passwords went to the server…
        let wire = sent(&rx1);
        assert_eq!(
            wire.iter()
                .filter(|line| line.contains("swordfish"))
                .count(),
            2,
            "{wire:?}"
        );
        // …and nowhere into the saved history; the rest is kept.
        app.save_history();
        assert_eq!(
            app.settings.history,
            vec!["hello there".to_string(), "/join #rust".to_string()]
        );
        // Changing the aliases later does not bring an old secret back.
        app.settings.script_text.clear();
        app.settings.aliases.clear();
        app.save_history();
        assert_eq!(
            app.settings.history,
            vec!["hello there".to_string(), "/join #rust".to_string()]
        );
    }

    #[test]
    fn a_timer_speaks_where_it_was_set_not_where_the_user_looks() {
        let (mut app, rx1, rx2, ops1, _) = two_servers();
        let (dcc_rx, _peer) = fake_dcc(&mut app, None, "bob");
        let chat = app
            .windows
            .iter()
            .position(|w| w.kind == Kind::Dcc)
            .unwrap();
        let dcc_window = |app: &OpenIrcApp| {
            app.windows
                .iter()
                .position(|w| w.kind == Kind::Dcc)
                .unwrap()
        };
        let _ = chat;

        // Set on server 1's channel; fires while bob's DCC chat is in front.
        app.active = ops1;
        app.dispatch("/timer 1 1 /me timer-A-only".into(), Origin::Input);
        app.dispatch("/timer 1 1 plain-A-only".into(), Origin::Input);
        app.active = dcc_window(&app);
        fire_timers_now(&mut app);
        assert!(dcc_rx.try_iter().next().is_none(), "nothing reaches bob");
        // To the channel it was set in, by name, not to the server's
        // currently picked one.
        let to_one = said(&rx1);
        assert!(
            to_one.contains(&"/describe #ops timer-A-only".to_string()),
            "{to_one:?}"
        );
        assert!(
            to_one.contains(&"/msg #ops plain-A-only".to_string()),
            "{to_one:?}"
        );

        // Fired while server 2's Messages window is in front: still server 1.
        app.settings.message_window = true;
        app.routing = Some(2);
        app.apply_event(Event::Chat {
            channel: false,
            target: "carol".into(),
            from: "carol".into(),
            text: "hi".into(),
            action: false,
        });
        app.active = app.find(Kind::List, MESSAGES).unwrap();
        app.routing = None;
        let ops1 = app
            .windows
            .iter()
            .position(|w| w.kind == Kind::Channel && w.server == Some(1))
            .unwrap();
        let messages = app.active;
        app.active = ops1;
        app.dispatch("/timer 1 1 to-the-channel".into(), Origin::Input);
        app.active = messages;
        fire_timers_now(&mut app);
        assert_eq!(said(&rx1), vec!["/msg #ops to-the-channel".to_string()]);
        assert!(sent(&rx2).is_empty(), "carol on server 2 gets nothing");

        // An event's command on server 1, with bob's chat in front, stays
        // on server 1 too.
        app.active = dcc_window(&app);
        app.on_server(Some(1), |app| {
            app.dispatch("/me from-an-event".into(), Origin::Script)
        });
        assert_eq!(sent(&rx1), vec!["/me from-an-event".to_string()]);
        assert!(sent(&rx2).is_empty());
        assert!(dcc_rx.try_iter().next().is_none());

        // Typed in the DCC chat: that chat, as before.
        app.active = dcc_window(&app);
        app.dispatch("/me waves".into(), Origin::Input);
        app.dispatch("hello bob".into(), Origin::Input);
        // A timer set in the chat speaks in the chat, whatever is in front.
        app.dispatch("/timer 1 1 /me later".into(), Origin::Input);
        app.active = ops1;
        fire_timers_now(&mut app);
        let to_bob: Vec<String> = dcc_rx.try_iter().collect();
        assert_eq!(
            to_bob,
            vec![
                "\u{1}ACTION waves\u{1}".to_string(),
                "hello bob".to_string(),
                "\u{1}ACTION later\u{1}".to_string()
            ]
        );
        assert!(sent(&rx1).is_empty());
        assert!(sent(&rx2).is_empty());
    }

    /// Type `line` and return the saved-history snapshot right after.
    fn enter(app: &mut OpenIrcApp, line: &str) -> Vec<String> {
        app.input = line.into();
        app.enter_line();
        app.settings.history.clone()
    }

    fn no_password(snapshot: &[String]) -> bool {
        snapshot.iter().all(|line| !line.contains("swordfish"))
    }

    #[test]
    fn history_is_never_saved_before_a_line_is_known() {
        let (mut app, rx1, _rx2, ops1, _) = two_servers();
        app.active = ops1;
        app.settings.keep_history = true;
        app.settings.script_text = "alias signin { msg NickServ IDENTIFY $1 }".into();
        // The tenth entry is the one saved right after it runs.
        for n in 0..9 {
            assert!(no_password(&enter(&mut app, &format!("/echo line {n}"))));
        }
        let snapshot = enter(&mut app, "/signin swordfish");
        assert_eq!(app.history.len(), 10);
        assert!(no_password(&snapshot), "{snapshot:?}");
        assert!(!snapshot.is_empty(), "the tenth Enter saved the history");
        // At the 200-entry cap every Enter saves; no snapshot ever holds it.
        for n in 0..200 {
            let line = if n % 37 == 0 {
                "/signin swordfish".to_string()
            } else {
                format!("/echo more {n}")
            };
            let snapshot = enter(&mut app, &line);
            assert!(no_password(&snapshot), "after {line}: {snapshot:?}");
        }
        assert_eq!(app.history.len(), 200);
        assert!(sent(&rx1).iter().any(|line| line.contains("swordfish")));
        // Not one snapshot written along the way held it, even for a moment.
        // Every tenth Enter, then every Enter once the history is full.
        assert!(app.history_saves.len() >= 29, "{}", app.history_saves.len());
        assert!(app.history_saves.iter().all(|saved| no_password(saved)));

        // A line already saved, found later to have sent a password (by
        // work it started), is taken out of the saved history at once.
        let entry = app.history.push("/echo innocent-looking");
        app.save_history();
        assert!(
            app.settings
                .history
                .contains(&"/echo innocent-looking".to_string())
        );
        app.provenance = entry;
        app.dispatch("/msg NickServ IDENTIFY swordfish".into(), Origin::Script);
        app.provenance = None;
        assert!(
            !app.settings
                .history
                .contains(&"/echo innocent-looking".to_string())
        );
    }

    #[test]
    fn a_timer_line_waits_and_takes_the_blame_for_what_it_sends_later() {
        let (mut app, rx1, _rx2, ops1, _) = two_servers();
        app.active = ops1;
        app.settings.keep_history = true;
        app.settings.script_text = "alias signin { msg NickServ IDENTIFY $1 }".into();
        app.settings.aliases.push(crate::settings::Alias {
            name: "si".into(),
            body: "/msg NickServ IDENTIFY $1".into(),
        });
        app.settings.aliases.push(crate::settings::Alias {
            name: "go".into(),
            body: "/echo nothing yet".into(),
        });
        enter(&mut app, "/timer 1 1 /signin swordfish");
        enter(&mut app, "/timer 1 1 /si swordfish");
        enter(&mut app, "/timer 1 1 /timer 1 1 /signin swordfish");
        enter(&mut app, "/timer 1 1 /go swordfish");
        // Unrelated typing while they wait; none of them is saved yet.
        enter(&mut app, "/echo unrelated");
        app.save_history();
        assert_eq!(app.settings.history, vec!["/echo unrelated".to_string()]);
        // The alias changes before its timer fires.
        app.settings.aliases[1].body = "/msg NickServ IDENTIFY $1".into();
        fire_timers_now(&mut app);
        // The nested one has set its own timer, still waiting.
        assert_eq!(app.timers.len(), 1);
        app.save_history();
        assert!(
            no_password(&app.settings.history),
            "{:?}",
            app.settings.history
        );
        fire_timers_now(&mut app);
        app.save_history();
        assert_eq!(app.settings.history, vec!["/echo unrelated".to_string()]);
        let wire = sent(&rx1);
        assert_eq!(
            wire.iter()
                .filter(|line| line.contains("swordfish"))
                .count(),
            4,
            "{wire:?}"
        );
        // Not one snapshot written while they ran held a password: the due
        // ones stay protected until each has run.
        assert!(app.history_saves.iter().all(|saved| no_password(saved)));

        // Two at once, with a nested one, all due in one tick.
        app.history_saves.clear();
        enter(&mut app, "/timer 1 1 /signin swordfish-a");
        enter(&mut app, "/timer 1 1 /signin swordfish-b");
        enter(&mut app, "/timer 1 1 /timer 1 1 /si swordfish-c");
        fire_timers_now(&mut app);
        fire_timers_now(&mut app);
        assert!(app.history_saves.len() >= 2, "the marks saved the history");
        assert!(
            app.history_saves.iter().all(|saved| no_password(saved)),
            "{:?}",
            app.history_saves
        );
    }

    #[test]
    fn an_alias_that_logs_in_to_a_server_is_never_saved() {
        use std::io::BufRead as _;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut lines = Vec::new();
            for line in std::io::BufReader::new(sock).lines().take(4) {
                lines.push(line.unwrap_or_default());
            }
            lines
        });
        let mut app = app();
        app.settings.keep_history = true;
        app.settings.script_text =
            format!("alias connectme {{ server -m -p $1 127.0.0.1 {port} }}");
        enter(&mut app, "/connectme swordfish");
        enter(&mut app, "/echo after");
        let registration = server.join().unwrap();
        assert!(
            registration
                .iter()
                .any(|line| line == "PASS :swordfish" || line == "PASS swordfish"),
            "{registration:?}"
        );
        app.save_history();
        assert_eq!(app.settings.history, vec!["/echo after".to_string()]);
    }

    /// Two servers, `#ops` on each, a `#public` on server 1, a query with
    /// alice on server 1, and a DCC chat with bob (its queue returned).
    fn destinations() -> (
        OpenIrcApp,
        std::sync::mpsc::Receiver<String>,
        std::sync::mpsc::Receiver<String>,
        std::sync::mpsc::Receiver<String>,
        Sender<String>,
    ) {
        let (mut app, rx1, rx2, _, _) = two_servers();
        app.routing = Some(1);
        app.ensure(Kind::Channel, "#public");
        app.ensure(Kind::Query, "alice");
        app.routing = None;
        let (dcc_rx, peer) = fake_dcc(&mut app, Some(1), "bob");
        (app, rx1, rx2, dcc_rx, peer)
    }

    /// A DCC chat with `nick` attached on `server` the way a real one is:
    /// gives what we send to the peer, and the peer's side for lines to us
    /// (drop it to hang up).
    fn fake_dcc(
        app: &mut OpenIrcApp,
        server: Option<u64>,
        nick: &str,
    ) -> (std::sync::mpsc::Receiver<String>, Sender<String>) {
        let (outgoing, to_peer) = std::sync::mpsc::sync_channel(dcc::OUTGOING_LIMIT);
        let (from_peer, incoming) = std::sync::mpsc::channel();
        let hangup = dcc::Hangup::none();
        app.attach_dcc(
            nick.into(),
            server,
            DccChat {
                outgoing,
                incoming,
                hangup,
            },
        );
        (to_peer, from_peer)
    }

    fn window(app: &OpenIrcApp, kind: Kind, title: &str, server: Option<u64>) -> usize {
        app.windows
            .iter()
            .position(|w| w.kind == kind && w.title == title && w.server == server)
            .unwrap()
    }

    #[test]
    fn a_timer_whose_window_closed_sends_nothing() {
        let (mut app, rx1, rx2, dcc_rx, _peer) = destinations();
        let public = window(&app, Kind::Channel, "#public", Some(1));
        for (kind, title) in [(Kind::Dcc, "DCC bob"), (Kind::Query, "alice")] {
            app.active = window(&app, kind, title, Some(1));
            app.dispatch("/timer 30 1 /me private-follow-up".into(), Origin::Input);
            app.dispatch("/timer 30 1 private words".into(), Origin::Input);
            let closing = app.active;
            app.close_window(closing);
            // A window of the same name opened again is not the same one.
            app.routing = Some(1);
            app.ensure(kind, title);
            app.routing = None;
            app.active = window(&app, Kind::Channel, "#public", Some(1));
            fire_timers_now(&mut app);
            let to_one = sent(&rx1);
            assert!(
                to_one.iter().all(|line| !line.contains("private")),
                "{title}: {to_one:?}"
            );
            assert!(dcc_rx.try_iter().next().is_none());
            assert!(sent(&rx2).is_empty());
        }
        let _ = public;
        assert!((0..app.windows.len()).any(|index| {
            texts(&app, index)
                .iter()
                .any(|line| !line.contains("private words") && line.contains("a line was not sent"))
        }));
    }

    #[test]
    fn a_dcc_chat_that_ends_sends_nothing_and_says_so() {
        let (mut app, rx1, _rx2, _, _) = two_servers();
        let public = app.active;
        // The peer hangs up, and the window is polled before the timer fires.
        let (to_peer, from_peer) = fake_dcc(&mut app, Some(1), "bob");
        let chat = window(&app, Kind::Dcc, "DCC bob", Some(1));
        app.active = chat;
        app.dispatch("/timer 30 1 after-the-end".into(), Origin::Input);
        drop(from_peer);
        drop(to_peer);
        app.poll_dcc();
        assert!(
            texts(&app, chat)
                .iter()
                .any(|line| line.contains("The DCC chat ended"))
        );
        app.active = public;
        fire_timers_now(&mut app);
        // Its thread is gone before the window looks: the send itself fails.
        let (to_peer, from_peer) = fake_dcc(&mut app, Some(1), "carol");
        let other = window(&app, Kind::Dcc, "DCC carol", Some(1));
        app.active = other;
        app.dispatch("/timer 30 1 lost-in-transit".into(), Origin::Input);
        drop(to_peer);
        app.active = public;
        fire_timers_now(&mut app);
        drop(from_peer);
        for (index, payload) in [(chat, "after-the-end"), (other, "lost-in-transit")] {
            assert!(
                !texts(&app, index)
                    .iter()
                    .any(|line| line.contains(&format!("> {payload}"))),
                "no line shown as sent: {:?}",
                texts(&app, index)
            );
            assert!(
                (0..app.windows.len()).any(|at| texts(&app, at)
                    .iter()
                    .any(|line| !line.contains(payload) && line.contains("a line was not sent"))),
                "a note, without {payload}"
            );
        }
        assert!(
            sent(&rx1)
                .iter()
                .all(|line| !line.contains("after-the-end") && !line.contains("lost-in-transit"))
        );
    }

    #[test]
    fn two_chats_with_the_same_name_keep_their_own_connections() {
        let (mut app, _rx1, _rx2, _, _) = two_servers();
        let (to_a, from_a) = fake_dcc(&mut app, Some(1), "bob");
        let (to_b, from_b) = fake_dcc(&mut app, Some(2), "bob");
        let a = window(&app, Kind::Dcc, "DCC bob", Some(1));
        let b = window(&app, Kind::Dcc, "DCC bob", Some(2));
        assert_ne!(a, b);
        // Typed, and by a timer, in B's window: only B's peer gets it.
        app.active = b;
        app.dispatch("/say for-bob-on-B".into(), Origin::Input);
        app.dispatch("/timer 30 1 timed-for-B".into(), Origin::Input);
        app.active = a;
        fire_timers_now(&mut app);
        assert_eq!(
            to_b.try_iter().collect::<Vec<_>>(),
            vec!["for-bob-on-B", "timed-for-B"]
        );
        assert!(to_a.try_iter().next().is_none());
        // Lines from each peer land in its own window, whichever is in front.
        app.active = b;
        from_a.send("hello from A".into()).unwrap();
        from_b.send("hello from B".into()).unwrap();
        app.poll_dcc();
        assert!(
            texts(&app, a)
                .iter()
                .any(|line| line.contains("hello from A"))
        );
        assert!(
            texts(&app, b)
                .iter()
                .any(|line| line.contains("hello from B"))
        );
        assert!(
            !texts(&app, a)
                .iter()
                .any(|line| line.contains("hello from B"))
        );
        // Closing B leaves A connected.
        app.close_window(b);
        let a = window(&app, Kind::Dcc, "DCC bob", Some(1));
        app.active = a;
        app.dispatch("/say still-here".into(), Origin::Input);
        assert_eq!(to_a.try_iter().collect::<Vec<_>>(), vec!["still-here"]);
        assert!(from_b.send("x".into()).is_err(), "B's chat was let go");
    }

    /// Run `poll_dcc` until `done` holds, for up to five seconds.
    fn poll_until(app: &mut OpenIrcApp, done: impl Fn(&OpenIrcApp) -> bool) {
        let started = std::time::Instant::now();
        while !done(app) && started.elapsed() < Duration::from_secs(5) {
            app.poll_dcc();
            thread::sleep(Duration::from_millis(20));
        }
        assert!(done(app), "timed out waiting for the DCC chat");
    }

    fn has_window(app: &OpenIrcApp, title: &str, server: Option<u64>) -> bool {
        app.windows
            .iter()
            .any(|w| w.kind == Kind::Dcc && w.title == title && w.server == server)
    }

    fn read_line(stream: &mut std::net::TcpStream) -> String {
        use std::io::BufRead as _;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut line = String::new();
        std::io::BufReader::new(stream)
            .read_line(&mut line)
            .unwrap();
        line.trim_end().to_string()
    }

    #[test]
    fn a_dcc_chat_belongs_to_the_server_it_was_offered_on() {
        let (mut app, rx1, _rx2, ops1, ops2) = two_servers();
        // A different bob already chats with us on server 2.
        let (to_b, _from_b) = fake_dcc(&mut app, Some(2), "bob");
        // bob on server 1 offers a chat; we accept with server 2 in front.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        app.routing = Some(1);
        app.apply_event(Event::DccChat {
            from: "bob".into(),
            host: "127.0.0.1".into(),
            port,
        });
        app.routing = None;
        app.active = ops2;
        app.accept_chat_offer(0);
        let mut peer_a = crate::dcc::accept_within(&listener, Duration::from_secs(5)).unwrap();
        poll_until(&mut app, |app| has_window(app, "DCC bob", Some(1)));
        // Both chats live, each in its own window and on its own server.
        let a = window(&app, Kind::Dcc, "DCC bob", Some(1));
        let b = window(&app, Kind::Dcc, "DCC bob", Some(2));
        app.active = a;
        app.dispatch("/say to-bob-on-A".into(), Origin::Input);
        assert_eq!(read_line(&mut peer_a), "to-bob-on-A");
        app.active = b;
        app.dispatch("/say to-bob-on-B".into(), Origin::Input);
        assert_eq!(to_b.try_iter().collect::<Vec<_>>(), vec!["to-bob-on-B"]);

        // Our own offer from server 1, answered with server 2 in front.
        app.active = ops1;
        app.dcc_nick = "carol".into();
        app.offer_chat();
        let offer = sent(&rx1).pop().unwrap();
        let port: u16 = offer
            .trim_end_matches('\u{1}')
            .rsplit(' ')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        app.active = ops2;
        let _peer_c = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        poll_until(&mut app, |app| has_window(app, "DCC carol", Some(1)));
        assert!(!has_window(&app, "DCC carol", Some(2)));
    }

    #[test]
    fn a_chat_that_connects_after_its_server_closed_is_hung_up() {
        let (mut app, _rx1, _rx2, _, ops2) = two_servers();
        app.routing = Some(1);
        let status1 = app.ensure(Kind::Status, "Status");
        app.routing = None;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        app.routing = Some(1);
        app.apply_event(Event::DccChat {
            from: "dave".into(),
            host: "127.0.0.1".into(),
            port,
        });
        app.routing = None;
        app.accept_chat_offer(0);
        let mut peer = crate::dcc::accept_within(&listener, Duration::from_secs(5)).unwrap();
        app.close_window(status1);
        app.active = app
            .windows
            .iter()
            .position(|w| w.server == Some(2))
            .unwrap_or(ops2);
        let started = std::time::Instant::now();
        while !app.dcc_connecting.is_empty() && started.elapsed() < Duration::from_secs(5) {
            app.poll_dcc();
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            app.windows.iter().all(|w| w.kind != Kind::Dcc),
            "no window for it"
        );
        assert!(app.dcc_links.is_empty());
        // The peer sees the hang-up.
        peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut rest = Vec::new();
        assert!(std::io::Read::read_to_end(&mut peer, &mut rest).is_ok());
    }

    #[test]
    fn an_old_dcc_timer_does_not_follow_a_replacement_connection() {
        let (mut app, _rx1, _rx2, _, _) = two_servers();
        // The chat ends, and later another bob connects into the window.
        let (to_old, from_old) = fake_dcc(&mut app, Some(1), "bob");
        app.dispatch("/timer 30 1 private-old-chat".into(), Origin::Input);
        drop(to_old);
        drop(from_old);
        app.poll_dcc();
        let (to_new, from_new) = fake_dcc(&mut app, Some(1), "bob");
        fire_timers_now(&mut app);
        assert!(to_new.try_iter().next().is_none());
        // A live chat replaced before the timer fires.
        app.dispatch("/timer 30 1 for-the-first".into(), Origin::Input);
        let (to_newer, _from_newer) = fake_dcc(&mut app, Some(1), "bob");
        fire_timers_now(&mut app);
        assert!(to_newer.try_iter().next().is_none());
        assert!(to_new.try_iter().next().is_none());
        for payload in ["private-old-chat", "for-the-first"] {
            assert!(
                (0..app.windows.len()).any(|at| texts(&app, at).iter().any(|line| {
                    !line.contains(payload)
                        && line.contains("DCC chat this was meant for has ended")
                })),
                "a note says the chat ended, without {payload}"
            );
        }
        // Hung up, the replaced chat's thread ends; the newest chat begins,
        // and a timer set in it now does reach it.
        drop(from_new);
        app.poll_dcc();
        app.dispatch("/timer 30 1 for-the-newest".into(), Origin::Input);
        fire_timers_now(&mut app);
        assert_eq!(
            to_newer.try_iter().collect::<Vec<_>>(),
            vec!["for-the-newest"]
        );
    }

    #[test]
    fn a_failed_dcc_send_keeps_the_peers_buffered_final_line() {
        let (mut app, _rx1, _rx2, _, _) = two_servers();
        let (to_peer, from_peer) = fake_dcc(&mut app, Some(1), "bob");
        let chat = window(&app, Kind::Dcc, "DCC bob", Some(1));
        app.dispatch("/timer 30 1 after-close".into(), Origin::Input);
        from_peer.send("last-line-from-peer".into()).unwrap();
        drop(from_peer);
        drop(to_peer);
        fire_timers_now(&mut app);
        app.poll_dcc();
        app.poll_dcc();
        let lines = texts(&app, chat);
        let count = |text: &str| lines.iter().filter(|line| line.contains(text)).count();
        assert_eq!(count("last-line-from-peer"), 1, "{lines:?}");
        assert_eq!(count("The DCC chat ended"), 1, "{lines:?}");
        assert_eq!(count("> after-close"), 0);
        // The peer's line comes before the end.
        let last = lines
            .iter()
            .position(|line| line.contains("last-line-from-peer"));
        let ended = lines
            .iter()
            .position(|line| line.contains("The DCC chat ended"));
        assert!(last < ended);
    }

    #[test]
    fn a_busy_dcc_chat_does_not_hold_back_the_others() {
        let (mut app, _rx1, _rx2, _, _) = two_servers();
        let (_to_a, from_a) = fake_dcc(&mut app, Some(1), "amy");
        let (_to_b, from_b) = fake_dcc(&mut app, Some(1), "ben");
        let (_to_c, from_c) = fake_dcc(&mut app, Some(1), "cat");
        let b = window(&app, Kind::Dcc, "DCC ben", Some(1));
        let c = window(&app, Kind::Dcc, "DCC cat", Some(1));
        from_b.send("ben-says-hi".into()).unwrap();
        // cat hangs up while amy floods.
        drop(from_c);
        for _ in 0..2 {
            for n in 0..FRAME_MESSAGES {
                from_a.send(format!("amy {n}")).unwrap();
            }
            app.poll_dcc();
        }
        assert!(
            texts(&app, b)
                .iter()
                .any(|line| line.contains("ben-says-hi"))
        );
        assert!(
            texts(&app, c)
                .iter()
                .any(|line| line.contains("The DCC chat ended"))
        );
    }

    #[test]
    fn closing_a_server_hangs_up_its_dcc_chats() {
        let (mut app, _rx1, _rx2, _, _) = two_servers();
        app.routing = Some(1);
        let status1 = app.ensure(Kind::Status, "Status");
        app.routing = None;
        let (to_a, from_a) = fake_dcc(&mut app, Some(1), "amy");
        let (to_b, from_b) = fake_dcc(&mut app, Some(2), "ben");
        app.close_window(status1);
        // amy's chat is let go: its thread's way out is closed.
        assert!(matches!(
            to_a.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Disconnected)
        ));
        assert!(from_a.send("x".into()).is_err());
        // ben's chat on server 2 still works both ways.
        let b = window(&app, Kind::Dcc, "DCC ben", Some(2));
        app.active = b;
        app.dispatch("/say still-here".into(), Origin::Input);
        assert_eq!(to_b.try_iter().collect::<Vec<_>>(), vec!["still-here"]);
        from_b.send("ben-replies".into()).unwrap();
        app.poll_dcc();
        let b = window(&app, Kind::Dcc, "DCC ben", Some(2));
        assert!(
            texts(&app, b)
                .iter()
                .any(|line| line.contains("ben-replies"))
        );
    }

    #[test]
    fn replacement_keeps_the_old_peers_queued_final_line() {
        let (mut app, _rx1, _rx2, _, _) = two_servers();
        let (to_old, from_old) = fake_dcc(&mut app, Some(1), "bob");
        from_old.send("old-final-line".into()).unwrap();
        drop(from_old);
        drop(to_old);
        // A completed replacement attaches before the old queue is polled.
        let (_to_new, from_new) = fake_dcc(&mut app, Some(1), "bob");
        from_new.send("new-first-line".into()).unwrap();
        // The old chat finishes in the first poll, the new one is read in
        // the next.
        app.poll_dcc();
        app.poll_dcc();
        let chat = window(&app, Kind::Dcc, "DCC bob", Some(1));
        let lines = texts(&app, chat);
        let at = |text: &str| {
            let found: Vec<usize> = lines
                .iter()
                .enumerate()
                .filter(|(_, line)| line.contains(text))
                .map(|(at, _)| at)
                .collect();
            assert_eq!(found.len(), 1, "{text}: {lines:?}");
            found[0]
        };
        // The old chat's line, its end, the new chat, then its line.
        assert!(at("old-final-line") < at("The DCC chat ended"));
        let boundaries: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.contains("DCC chat with bob"))
            .map(|(at, _)| at)
            .collect();
        assert_eq!(boundaries.len(), 2);
        assert!(at("The DCC chat ended") < boundaries[1]);
        assert!(boundaries[1] < at("new-first-line"));

        // A chat with more waiting than one frame's budget is replaced: its
        // lines are read within the budget, every one once and in order,
        // then its end, then the new chat.
        let (_to_live, from_live) = fake_dcc(&mut app, Some(1), "amy");
        let (_others, _peers): (Vec<_>, Vec<_>) = (0..3)
            .map(|n| fake_dcc(&mut app, Some(1), &format!("x{n}")))
            .unzip();
        for n in 0..700 {
            from_live.send(format!("backlog {n:03}")).unwrap();
        }
        let (_to_next, from_next) = fake_dcc(&mut app, Some(1), "amy");
        from_next.send("next-first-line".into()).unwrap();
        // Hanging up ends the old chat's thread, which lets go of its side.
        drop(from_live);
        let amy = window(&app, Kind::Dcc, "DCC amy", Some(1));
        let shown = |app: &OpenIrcApp| texts(app, amy).len();
        for _ in 0..4 {
            let before = shown(&app);
            app.poll_dcc();
            assert!(
                shown(&app) - before <= FRAME_MESSAGES + 2,
                "within the budget"
            );
        }
        let lines = texts(&app, amy);
        let backlog: Vec<&String> = lines
            .iter()
            .filter(|line| line.contains("backlog "))
            .collect();
        assert_eq!(backlog.len(), 700);
        assert!(
            backlog
                .iter()
                .enumerate()
                .all(|(n, line)| line.contains(&format!("backlog {n:03}")))
        );
        let place = |text: &str| lines.iter().rposition(|line| line.contains(text)).unwrap();
        assert!(place("backlog 699") < place("The DCC chat ended"));
        assert!(place("The DCC chat ended") < place("DCC chat with amy"));
        assert!(place("DCC chat with amy") < place("next-first-line"));
    }

    #[test]
    fn two_chats_set_up_at_once_both_open() {
        let (mut app, rx1, rx2, ops1, ops2) = two_servers();
        // Two of our own offers, from two servers, both waiting.
        let mut ports = Vec::new();
        for (front, nick, rx) in [(ops1, "amy", &rx1), (ops2, "ben", &rx2)] {
            app.active = front;
            app.dcc_nick = nick.into();
            app.offer_chat();
            let offer = sent(rx).pop().unwrap();
            ports.push(
                offer
                    .trim_end_matches('\u{1}')
                    .rsplit(' ')
                    .next()
                    .unwrap()
                    .parse::<u16>()
                    .unwrap(),
            );
        }
        assert_eq!(app.outgoing_dcc.len(), 2);
        // The second asked answers first.
        let _ben = std::net::TcpStream::connect(("127.0.0.1", ports[1])).unwrap();
        let _amy = std::net::TcpStream::connect(("127.0.0.1", ports[0])).unwrap();
        poll_until(&mut app, |app| {
            has_window(app, "DCC amy", Some(1)) && has_window(app, "DCC ben", Some(2))
        });
        // Two offers we accepted, both still connecting at once.
        let first = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let second = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        for (server, nick, listener) in [(1, "cat", &first), (2, "dan", &second)] {
            app.routing = Some(server);
            app.apply_event(Event::DccChat {
                from: nick.into(),
                host: "127.0.0.1".into(),
                port: listener.local_addr().unwrap().port(),
            });
            app.routing = None;
        }
        app.accept_chat_offer(0);
        app.accept_chat_offer(0);
        assert_eq!(app.dcc_connecting.len(), 2);
        let _cat = crate::dcc::accept_within(&first, Duration::from_secs(5)).unwrap();
        let _dan = crate::dcc::accept_within(&second, Duration::from_secs(5)).unwrap();
        poll_until(&mut app, |app| {
            has_window(app, "DCC cat", Some(1)) && has_window(app, "DCC dan", Some(2))
        });
    }

    #[test]
    fn the_dcc_budget_holds_however_many_chats_there_are() {
        let (mut app, _rx1, _rx2, _, _) = two_servers();
        app.poll_dcc();
        let shown = |app: &OpenIrcApp| {
            (0..app.windows.len())
                .filter(|&at| app.windows[at].kind == Kind::Dcc)
                .map(|at| {
                    texts(app, at)
                        .iter()
                        .filter(|line| line.contains("line from"))
                        .count()
                })
                .sum::<usize>()
        };
        let mut peers = Vec::new();
        for count in [1, 500, 501] {
            while peers.len() < count {
                let n = peers.len();
                peers.push(fake_dcc(&mut app, Some(1), &format!("p{n}")));
            }
            for (_, from_peer) in &peers {
                from_peer.send("line from peer".into()).unwrap();
            }
            let before = shown(&app);
            app.poll_dcc();
            let read = shown(&app) - before;
            assert_eq!(read, count.min(FRAME_MESSAGES), "{count} chats");
            // The next poll reaches everyone the first did not.
            app.poll_dcc();
            assert_eq!(shown(&app) - before, count, "{count} chats, two polls");
        }
        // A thousand chats that all keep sending: two polls reach every
        // one, since the second starts where the first stopped.
        while peers.len() < 1_000 {
            let n = peers.len();
            peers.push(fake_dcc(&mut app, Some(1), &format!("p{n}")));
        }
        let heard = |app: &OpenIrcApp| {
            (0..app.windows.len())
                .filter(|&at| app.windows[at].kind == Kind::Dcc)
                .map(|at| {
                    texts(app, at)
                        .iter()
                        .filter(|line| line.contains("again"))
                        .count()
                })
                .collect::<Vec<usize>>()
        };
        for _ in 0..2 {
            for (_, from_peer) in &peers {
                from_peer.send("again".into()).unwrap();
            }
            app.poll_dcc();
        }
        let counts = heard(&app);
        assert_eq!(counts.iter().sum::<usize>(), 2 * FRAME_MESSAGES);
        assert!(
            counts.iter().all(|count| *count >= 1),
            "every chat was reached"
        );
    }
    #[test]
    fn replacing_a_chat_whose_peer_keeps_sending_stays_within_the_budget() {
        let (mut app, _rx1, _rx2, _, _) = two_servers();
        // The old peer never stops sending, until it is hung up on.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        // How the peer's writing ended.
        #[derive(Debug, PartialEq)]
        enum Ended {
            HungUp,
            TimedOut,
            Limit,
        }
        let flood = thread::spawn(move || {
            let mut socket = crate::dcc::accept_within(&listener, Duration::from_secs(5)).unwrap();
            // A write that cannot go on gives up rather than blocking.
            socket
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut sent = 0u64;
            let started = std::time::Instant::now();
            // Bounded, in time and in lines, whatever happens on our side.
            while started.elapsed() < Duration::from_secs(10) && sent < 2_000_000 {
                let line = format!("flood {sent}\r\n");
                match std::io::Write::write_all(&mut socket, line.as_bytes()) {
                    Ok(()) => sent += 1,
                    Err(err)
                        if matches!(
                            err.kind(),
                            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                        ) =>
                    {
                        return (Ended::TimedOut, sent);
                    }
                    Err(_) => return (Ended::HungUp, sent),
                }
            }
            (Ended::Limit, sent)
        });
        let old = crate::dcc::connect_chat("127.0.0.1", port).unwrap();
        app.attach_dcc("bob".into(), Some(1), old);
        let (_to_other, from_other) = fake_dcc(&mut app, Some(1), "amy");
        // Let the old chat's queue fill, then a new bob chat arrives. The
        // arrival itself shows none of the old chat's lines: those are read
        // in the polls, within their budget.
        thread::sleep(Duration::from_millis(300));
        let bob = window(&app, Kind::Dcc, "DCC bob", Some(1));
        let before_attach = texts(&app, bob).len();
        let (_to_new, from_new) = fake_dcc(&mut app, Some(1), "bob");
        assert_eq!(
            texts(&app, bob).len(),
            before_attach,
            "nothing shown on arrival"
        );
        from_new.send("new-bob-line".into()).unwrap();
        from_other.send("amy-line".into()).unwrap();
        let bob = window(&app, Kind::Dcc, "DCC bob", Some(1));
        let amy = window(&app, Kind::Dcc, "DCC amy", Some(1));
        let total = |app: &OpenIrcApp| texts(app, bob).len() + texts(app, amy).len();
        let started = std::time::Instant::now();
        let mut polls = 0;
        while !texts(&app, bob)
            .iter()
            .any(|line| line.contains("new-bob-line"))
        {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "the new chat got going"
            );
            let before = total(&app);
            app.poll_dcc();
            polls += 1;
            assert!(
                total(&app) - before <= FRAME_MESSAGES + 2,
                "poll {polls} within the budget"
            );
            if polls == 1 {
                // amy is heard in the very first poll, whatever bob's old
                // chat has waiting.
                assert!(
                    texts(&app, amy)
                        .iter()
                        .any(|line| line.contains("amy-line"))
                );
            }
            thread::sleep(Duration::from_millis(10));
        }
        let lines = texts(&app, bob);
        let place = |text: &str| lines.iter().rposition(|line| line.contains(text)).unwrap();
        assert!(place("The DCC chat ended") < place("new-bob-line"));
        // Every flood line shown is shown once, in order.
        let numbers: Vec<u64> = lines
            .iter()
            .filter_map(|line| line.split("flood ").nth(1)?.trim().parse().ok())
            .collect();
        assert!(!numbers.is_empty(), "the old chat's lines were shown");
        assert_eq!(numbers[0], 0, "from its first line");
        assert!(numbers.windows(2).all(|pair| pair[1] == pair[0] + 1));
        // The peer stopped because it was hung up on: not by its own limit,
        // and not because its writes stalled.
        let (ended, sent) = flood.join().expect("the peer's thread");
        assert_eq!(ended, Ended::HungUp, "after {sent} lines");
    }

    #[test]
    fn closing_a_server_frees_the_port_of_its_unanswered_chat_offer() {
        let (mut app, rx1, _rx2, ops1, _) = two_servers();
        app.routing = Some(1);
        let status1 = app.ensure(Kind::Status, "Status");
        app.routing = None;
        let (to_b, _from_b) = fake_dcc(&mut app, Some(2), "ben");
        app.active = ops1;
        app.dcc_nick = "amy".into();
        app.offer_chat();
        let offer = sent(&rx1).pop().unwrap();
        let port: u16 = offer
            .trim_end_matches('\u{1}')
            .rsplit(' ')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        app.close_window(status1);
        assert!(app.outgoing_dcc.is_empty());
        // The port is free again at once, not when the offer runs out.
        let started = std::time::Instant::now();
        while std::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, port)).is_err() {
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "port {port} still held"
            );
            thread::sleep(Duration::from_millis(20));
        }
        // ben's chat on server 2 carries on.
        app.active = window(&app, Kind::Dcc, "DCC ben", Some(2));
        app.dispatch("/say still-here".into(), Origin::Input);
        assert_eq!(to_b.try_iter().collect::<Vec<_>>(), vec!["still-here"]);
    }

    #[test]
    fn a_peer_that_is_not_keeping_up_gets_no_more_until_it_does() {
        let (mut app, _rx1, _rx2, _, _) = two_servers();
        let (to_peer, _from_peer) = fake_dcc(&mut app, Some(1), "bob");
        let chat = window(&app, Kind::Dcc, "DCC bob", Some(1));
        app.active = chat;
        for n in 0..crate::dcc::OUTGOING_LIMIT {
            app.dispatch(format!("/say line {n}"), Origin::Input);
        }
        app.dispatch("/say one-too-many".into(), Origin::Input);
        assert!(
            !texts(&app, chat)
                .iter()
                .any(|line| line.contains("> one-too-many"))
        );
        assert!((0..app.windows.len()).any(|at| {
            texts(&app, at)
                .iter()
                .any(|line| !line.contains("one-too-many") && line.contains("not keeping up"))
        }));
        // The chat is still there: once the peer reads, it is used again.
        assert_eq!(to_peer.try_iter().count(), crate::dcc::OUTGOING_LIMIT);
        app.dispatch("/me is back".into(), Origin::Input);
        assert_eq!(
            to_peer.try_iter().collect::<Vec<_>>(),
            vec!["\u{1}ACTION is back\u{1}"]
        );
    }

    #[test]
    fn a_failed_dcc_chat_is_noted_on_its_own_server_by_name() {
        let (mut app, _rx1, _rx2, _, ops2) = two_servers();
        for id in [1, 2] {
            app.routing = Some(id);
            app.ensure(Kind::Status, "Status");
        }
        app.routing = None;
        // A port nobody listens on.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        app.routing = Some(1);
        app.apply_event(Event::DccChat {
            from: "eve".into(),
            host: "127.0.0.1".into(),
            port,
        });
        app.routing = None;
        app.active = ops2;
        app.accept_chat_offer(0);
        let started = std::time::Instant::now();
        while !app.dcc_connecting.is_empty() && started.elapsed() < Duration::from_secs(10) {
            app.poll_dcc();
            thread::sleep(Duration::from_millis(20));
        }
        let status1 = window(&app, Kind::Status, "Status", Some(1));
        let status2 = window(&app, Kind::Status, "Status", Some(2));
        assert!(
            texts(&app, status1)
                .iter()
                .any(|line| line.contains("The DCC chat with eve failed")),
            "{:?}",
            texts(&app, status1)
        );
        assert!(!texts(&app, status2).iter().any(|line| line.contains("eve")));
    }

    #[test]
    fn a_new_chat_takes_nothing_before_it_begins() {
        let (mut app, _rx1, _rx2, _, _) = two_servers();
        let (_to_old, from_old) = fake_dcc(&mut app, Some(1), "amy");
        let (_others, _peers): (Vec<_>, Vec<_>) = (0..3)
            .map(|n| fake_dcc(&mut app, Some(1), &format!("x{n}")))
            .unzip();
        for n in 0..700 {
            from_old.send(format!("backlog {n:03}")).unwrap();
        }
        // A new amy chat arrives; the old one's lines are still to show.
        let (to_new, _from_new) = fake_dcc(&mut app, Some(1), "amy");
        let amy = window(&app, Kind::Dcc, "DCC amy", Some(1));
        app.poll_dcc();
        let begun = |app: &OpenIrcApp| {
            texts(app, amy)
                .iter()
                .filter(|line| line.contains("DCC chat with amy"))
                .count()
                == 2
        };
        assert!(!begun(&app), "the old chat is still being shown");
        // Replies typed now, and a timer set now, reach neither peer.
        app.active = amy;
        // Plain text through Enter, as the window sends it.
        type_draft(&mut app, "plain-during-drain");
        app.enter_line();
        app.dispatch("/say reply-during-drain".into(), Origin::Input);
        app.dispatch("/me acts-during-drain".into(), Origin::Input);
        app.dispatch("/timer 30 1 timed-during-drain".into(), Origin::Input);
        assert!(to_new.try_iter().next().is_none());
        let me = app.me().to_string();
        for (text, echo) in [
            ("plain-during-drain", "> plain-during-drain".to_string()),
            ("reply-during-drain", "> reply-during-drain".to_string()),
            ("acts-during-drain", format!("{me} acts-during-drain")),
        ] {
            let lines = texts(&app, amy);
            assert!(
                lines
                    .iter()
                    .any(|line| !line.contains(text) && line.contains("a line was not sent")),
                "{text}"
            );
            // Not shown as sent, in the form a sent line would take.
            assert!(
                !lines.iter().any(|line| line.contains(&echo)),
                "{echo}: {lines:?}"
            );
        }
        // The old chat ends and the new one begins.
        drop(from_old);
        for _ in 0..5 {
            app.poll_dcc();
        }
        assert!(begun(&app));
        fire_timers_now(&mut app);
        assert!(
            to_new.try_iter().next().is_none(),
            "the timer was set before it began"
        );
        // Now replies reach the new chat.
        app.dispatch("/say reply-after-start".into(), Origin::Input);
        assert_eq!(
            to_new.try_iter().collect::<Vec<_>>(),
            vec!["reply-after-start"]
        );
    }

    /// Type `text` into the input line, as the window does.
    fn type_draft(app: &mut OpenIrcApp, text: &str) {
        app.input = text.into();
        app.note_draft();
    }

    #[test]
    fn a_chat_that_connects_never_takes_a_draft_meant_for_another_window() {
        let (mut app, rx1, _rx2, ops1, _) = two_servers();
        app.active = ops1;
        type_draft(&mut app, "draft for ops");
        // A chat connects, and our own join completes, while the user types.
        let (to_bob, _from_bob) = fake_dcc(&mut app, Some(1), "bob");
        app.routing = Some(1);
        app.apply_event(Event::Join {
            who: "me".into(),
            channel: "#elsewhere".into(),
            address: String::new(),
        });
        app.routing = None;
        assert_eq!(app.active, ops1, "the window being typed in stays in front");
        let bob = window(&app, Kind::Dcc, "DCC bob", Some(1));
        assert_ne!(
            app.windows[bob].activity,
            Activity::None,
            "the chat is marked"
        );
        app.enter_line();
        assert!(to_bob.try_iter().next().is_none());
        assert_eq!(sent(&rx1), vec!["draft for ops".to_string()]);
        // With nothing typed, a chat that connects comes forward as before.
        let (_to_amy, _from_amy) = fake_dcc(&mut app, Some(1), "amy");
        assert_eq!(app.active, window(&app, Kind::Dcc, "DCC amy", Some(1)));
    }

    #[test]
    fn a_draft_typed_for_one_chat_is_not_sent_to_the_next() {
        let (mut app, _rx1, _rx2, _, _) = two_servers();
        // The chat ends while its window stays; the user starts a reply.
        let (to_old, from_old) = fake_dcc(&mut app, Some(1), "bob");
        let bob = window(&app, Kind::Dcc, "DCC bob", Some(1));
        drop(from_old);
        drop(to_old);
        app.poll_dcc();
        app.active = bob;
        type_draft(&mut app, "meant for the old bob");
        // A new bob connects into the same window before Enter.
        let (to_new, from_new) = fake_dcc(&mut app, Some(1), "bob");
        app.enter_line();
        assert!(
            to_new.try_iter().next().is_none(),
            "the first Enter sends nothing"
        );
        assert_eq!(app.input, "meant for the old bob", "the text stays");
        assert!(
            texts(&app, bob)
                .iter()
                .any(|line| line.contains("press Enter again"))
        );
        // A second Enter sends it to the new chat, on purpose.
        app.enter_line();
        assert_eq!(
            to_new.try_iter().collect::<Vec<_>>(),
            vec!["meant for the old bob"]
        );

        // The chat after that begins; a reply is typed for it, and then a
        // newer chat replaces it and begins before Enter.
        let (_to_next, from_next) = fake_dcc(&mut app, Some(1), "bob");
        drop(from_new);
        app.poll_dcc();
        type_draft(&mut app, "for the bob before");
        let (to_last, _from_last) = fake_dcc(&mut app, Some(1), "bob");
        drop(from_next);
        app.poll_dcc();
        app.enter_line();
        assert!(to_last.try_iter().next().is_none());
        assert_eq!(app.input, "for the bob before");
    }

    #[test]
    fn a_waiting_paste_goes_where_it_was_pasted_or_nowhere() {
        let (mut app, rx1, _rx2, ops1, _) = two_servers();
        app.settings.paste_confirm = 3;
        // Pasted into #ops; a chat connects while the confirmation waits.
        app.active = ops1;
        type_draft(&mut app, "one\ntwo\nthree");
        app.enter_line();
        assert!(app.paste_pending.is_some());
        let (to_bob, from_bob) = fake_dcc(&mut app, Some(1), "bob");
        assert_eq!(app.active, ops1, "the paste's window stays in front");
        app.send_paste();
        assert!(to_bob.try_iter().next().is_none());
        assert_eq!(
            said(&rx1),
            vec!["/msg #ops one", "/msg #ops two", "/msg #ops three"]
        );
        // Pasted into a chat that is replaced before the confirmation.
        let bob = window(&app, Kind::Dcc, "DCC bob", Some(1));
        app.active = bob;
        type_draft(&mut app, "four\nfive\nsix");
        app.enter_line();
        let (to_new, _from_new) = fake_dcc(&mut app, Some(1), "bob");
        // The old chat ends, so the new one really has begun and could take
        // lines, before the paste is confirmed.
        drop(from_bob);
        drop(to_bob);
        app.poll_dcc();
        let opened = texts(&app, bob)
            .iter()
            .filter(|line| line.contains("DCC chat with bob"))
            .count();
        assert_eq!(opened, 2, "the new chat has begun");
        assert!(app.active_chat().is_some(), "and is usable");
        app.send_paste();
        assert!(
            to_new.try_iter().next().is_none(),
            "nothing for the new bob"
        );
        assert!((0..app.windows.len()).any(|at| {
            texts(&app, at)
                .iter()
                .any(|line| !line.contains("four") && line.contains("a line was not sent"))
        }));
    }

    #[test]
    fn a_draft_reaches_the_channel_it_was_typed_in_after_a_background_join() {
        let (mut app, rx1, _rx2, ops1, _) = two_servers();
        // The server side, as the network thread runs it, in #ops.
        let mut server = crate::pump::Pump::new(crate::pump::Config::new(
            "me",
            "me",
            "Me",
            Vec::new(),
            Vec::new(),
        ));
        server.on_server_line(":me!u@h JOIN :#ops");
        app.active = ops1;
        type_draft(&mut app, "draft for ops");
        // Our own join completes in the background: the server side now
        // points at the new channel, while #ops stays in front.
        server.on_server_line(":me!u@h JOIN :#elsewhere");
        app.routing = Some(1);
        app.apply_event(Event::Join {
            who: "me".into(),
            channel: "#elsewhere".into(),
            address: String::new(),
        });
        app.routing = None;
        app.enter_line();
        // What the window sent reaches the server side line by line, and
        // before every one of them another of our own joins arrives and
        // moves the server side's target: the worst order there can be.
        let mut wire: Vec<String> = Vec::new();
        let mut joins = 0;
        let mut deliver = |server: &mut crate::pump::Pump, wire: &mut Vec<String>| {
            for line in rx1.try_iter() {
                joins += 1;
                server.on_server_line(&format!(":me!u@h JOIN :#moved{joins}"));
                wire.extend(
                    server
                        .on_user_line(&line)
                        .outgoing
                        .into_iter()
                        .filter(|line| line.starts_with("PRIVMSG") || line.starts_with("TOPIC")),
                );
            }
        };
        deliver(&mut server, &mut wire);
        // A timer's command with no channel of its own goes to its channel
        // too, and so does a typed one.
        app.dispatch("/timer 1 1 /topic set by a timer".into(), Origin::Input);
        deliver(&mut server, &mut wire);
        fire_timers_now(&mut app);
        deliver(&mut server, &mut wire);
        type_draft(&mut app, "/topic typed topic");
        app.enter_line();
        deliver(&mut server, &mut wire);
        assert_eq!(
            wire,
            vec![
                "PRIVMSG #ops :draft for ops".to_string(),
                "TOPIC #ops :set by a timer".to_string(),
                "TOPIC #ops :typed topic".to_string()
            ]
        );
    }

    #[test]
    fn an_editbox_draft_is_kept_for_the_chat_it_was_put_in_for() {
        let (mut app, _rx1, _rx2, _, _) = two_servers();
        let (to_old, from_old) = fake_dcc(&mut app, Some(1), "bob");
        let bob = window(&app, Kind::Dcc, "DCC bob", Some(1));
        app.active = bob;
        app.dispatch("/editbox meant for the old bob".into(), Origin::Script);
        // The chat is replaced, and the new one begins, before Enter.
        let (to_new, _from_new) = fake_dcc(&mut app, Some(1), "bob");
        drop(from_old);
        drop(to_old);
        app.poll_dcc();
        assert!(app.active_chat().is_some(), "the new chat has begun");
        app.enter_line();
        assert!(to_new.try_iter().next().is_none());
        assert_eq!(app.input, "meant for the old bob");
        app.enter_line();
        assert_eq!(
            to_new.try_iter().collect::<Vec<_>>(),
            vec!["meant for the old bob"]
        );
    }

    #[test]
    fn the_messages_window_answers_whom_it_answered_when_the_words_were_written() {
        let (mut app, rx1, _rx2, _, _) = two_servers();
        app.settings.message_window = true;
        app.settings.paste_confirm = 3;
        let message = |app: &mut OpenIrcApp, nick: &str| {
            app.routing = Some(1);
            app.apply_event(Event::Chat {
                channel: false,
                target: nick.into(),
                from: nick.into(),
                text: "hi".into(),
                action: false,
            });
            app.routing = None;
        };
        message(&mut app, "carol");
        app.routing = Some(1);
        let messages = app.find(Kind::List, MESSAGES).unwrap();
        app.routing = None;
        app.active = messages;
        // A paste for carol waits; dave writes before it is confirmed.
        type_draft(&mut app, "one\ntwo\nthree");
        app.enter_line();
        message(&mut app, "dave");
        app.send_paste();
        assert_eq!(
            said(&rx1),
            vec!["/msg carol one", "/msg carol two", "/msg carol three"]
        );
        // A reply typed to dave; erin writes before Enter.
        type_draft(&mut app, "for dave");
        message(&mut app, "erin");
        app.enter_line();
        assert!(sent(&rx1).is_empty(), "the first Enter sends nothing");
        assert_eq!(app.input, "for dave");
        assert!(
            texts(&app, messages)
                .iter()
                .any(|line| line.contains("press Enter again"))
        );
        app.enter_line();
        assert_eq!(said(&rx1), vec!["/msg erin for dave".to_string()]);
    }

    #[test]
    fn replacing_a_chat_stuck_writing_to_a_peer_that_never_reads() {
        let (mut app, _rx1, _rx2, _, _) = two_servers();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let peer = thread::spawn(move || {
            // Connected, and never reads, until the test is over (or fails,
            // which drops `done_tx`): the test does not race this side.
            let socket = crate::dcc::accept_within(&listener, Duration::from_secs(5)).unwrap();
            let _ = done_rx.recv();
            drop(socket);
        });
        let old = crate::dcc::connect_chat("127.0.0.1", port).unwrap();
        app.attach_dcc("bob".into(), Some(1), old);
        let bob = window(&app, Kind::Dcc, "DCC bob", Some(1));
        app.active = bob;
        // Send until the socket is full, the chat's thread is stuck in a
        // write, and the queue behind it is full too. How many lines that
        // takes depends on the machine's socket buffers; each round costs
        // the same however many went before, since only the newest status
        // line is read: that is where a line not sent is told.
        let line = "x".repeat(64 * 1024);
        let refusal = |app: &OpenIrcApp| {
            app.windows
                .iter()
                .filter(|window| window.kind == Kind::Status)
                .filter_map(|window| window.lines.last())
                .map(RichLine::plain)
                .find(|text| text.contains("was not sent"))
        };
        let started = std::time::Instant::now();
        let note = loop {
            app.speak(&line, Speech::Say);
            if let Some(note) = refusal(&app) {
                break note;
            }
            // Only against a hang: the peer waits as long as it takes.
            assert!(
                started.elapsed() < Duration::from_secs(60),
                "the queue never filled"
            );
        };
        assert!(note.contains("not keeping up"), "{note}");
        // A new bob arrives: the stuck chat is hung up, ends, and the new
        // one begins, without waiting for the write to give up (30 s).
        let replaced = std::time::Instant::now();
        let (to_new, _from_new) = fake_dcc(&mut app, Some(1), "bob");
        while app.active_chat().is_none() {
            assert!(
                replaced.elapsed() < Duration::from_secs(10),
                "the new chat began"
            );
            app.poll_dcc();
            thread::sleep(Duration::from_millis(20));
        }
        app.dispatch("/say hello new bob".into(), Origin::Input);
        assert_eq!(to_new.try_iter().collect::<Vec<_>>(), vec!["hello new bob"]);
        let _ = done_tx.send(());
        peer.join().unwrap();
    }

    /// A private message from `nick` on server 1, which the Messages window
    /// shows and answers.
    fn private_from(app: &mut OpenIrcApp, nick: &str) {
        app.routing = Some(1);
        app.apply_event(Event::Chat {
            channel: false,
            target: nick.into(),
            from: nick.into(),
            text: "hi".into(),
            action: false,
        });
        app.routing = None;
    }

    fn messages_window(app: &mut OpenIrcApp) -> usize {
        app.routing = Some(1);
        let index = app.ensure(Kind::List, MESSAGES);
        app.routing = None;
        index
    }

    #[test]
    fn a_timer_set_in_the_messages_window_answers_whom_it_answered_then() {
        let (mut app, rx1, _rx2, _, _) = two_servers();
        app.settings.message_window = true;
        app.settings.paste_confirm = 3;
        private_from(&mut app, "carol");
        app.active = messages_window(&mut app);
        // Typed, nested, and set by a confirmed paste: all while carol is
        // the one being answered.
        app.dispatch("/timer 1 1 to carol".into(), Origin::Input);
        app.dispatch(
            "/timer 1 1 /timer 1 1 /me waves at carol".into(),
            Origin::Input,
        );
        type_draft(
            &mut app,
            "/timer 1 1 pasted to carol\n/echo two\n/echo three",
        );
        app.enter_line();
        app.send_paste();
        // dave writes before any of them fires.
        private_from(&mut app, "dave");
        fire_timers_now(&mut app);
        fire_timers_now(&mut app);
        assert_eq!(
            said(&rx1),
            vec![
                "/msg carol to carol",
                "/msg carol pasted to carol",
                "/describe carol waves at carol"
            ]
        );
    }

    #[test]
    fn words_meant_for_nobody_stay_unsent_when_someone_writes() {
        let (mut app, rx1, _rx2, _, _) = two_servers();
        app.settings.message_window = true;
        app.settings.paste_confirm = 3;
        // Our own /msg opens the Messages window, with nobody to answer.
        app.routing = Some(1);
        app.apply_event(Event::Sent {
            target: "zed".into(),
            from: "me".into(),
            text: "hello".into(),
            action: false,
        });
        app.routing = None;
        let messages = messages_window(&mut app);
        app.active = messages;
        type_draft(&mut app, "one\ntwo\nthree");
        app.enter_line();
        app.dispatch("/timer 1 1 timed for nobody".into(), Origin::Input);
        // dave writes; the paste and the timer still answer nobody.
        private_from(&mut app, "dave");
        app.send_paste();
        fire_timers_now(&mut app);
        assert!(sent(&rx1).is_empty(), "dave gets nothing");
        assert!(
            texts(&app, messages)
                .iter()
                .any(|line| line.contains("Nobody to answer"))
        );
    }

    #[test]
    fn saved_recipients_follow_a_nick_change_on_their_own_server() {
        let (mut app, rx1, rx2, _, _) = two_servers();
        app.settings.message_window = true;
        app.settings.paste_confirm = 3;
        private_from(&mut app, "carol");
        let messages = messages_window(&mut app);
        app.active = messages;
        app.dispatch("/timer 1 1 timed for carol".into(), Origin::Input);
        type_draft(&mut app, "one\ntwo\nthree");
        app.enter_line();
        // A carol on server 2 renaming changes nothing here.
        app.routing = Some(2);
        app.apply_event(Event::Nick {
            old: "carol".into(),
            new: "someone".into(),
        });
        // Server 1's carol becomes caroline, and a new carol arrives.
        app.routing = Some(1);
        app.apply_event(Event::Nick {
            old: "carol".into(),
            new: "caroline".into(),
        });
        app.routing = None;
        app.send_paste();
        fire_timers_now(&mut app);
        assert_eq!(
            said(&rx1),
            vec![
                "/msg caroline one",
                "/msg caroline two",
                "/msg caroline three",
                "/msg caroline timed for carol"
            ]
        );
        assert!(sent(&rx2).is_empty());
        // A draft typed for carol follows her too, and a plain reply goes to
        // caroline.
        type_draft(&mut app, "draft for carol");
        app.routing = Some(1);
        app.apply_event(Event::Nick {
            old: "caroline".into(),
            new: "carol2".into(),
        });
        app.routing = None;
        app.enter_line();
        assert_eq!(said(&rx1), vec!["/msg carol2 draft for carol".to_string()]);
    }

    /// The real protocol engine for server 1, in the given channels, fed
    /// what the window sends; gives the final PRIVMSG and QUIT lines.
    fn final_lines(
        server: &mut crate::pump::Pump,
        rx: &std::sync::mpsc::Receiver<String>,
    ) -> Vec<String> {
        rx.try_iter()
            .flat_map(|line| server.on_user_line(&line).outgoing)
            .filter(|line| line.starts_with("PRIVMSG") || line.starts_with("QUIT"))
            .collect()
    }

    fn engine(channels: &[&str]) -> crate::pump::Pump {
        let mut server = crate::pump::Pump::new(crate::pump::Config::new(
            "me",
            "me",
            "Me",
            Vec::new(),
            Vec::new(),
        ));
        for channel in channels {
            server.on_server_line(&format!(":me!u@h JOIN :{channel}"));
        }
        server
    }

    #[test]
    fn a_channel_name_with_an_unusual_space_stays_whole() {
        let (mut app, rx1, _rx2, _, _) = two_servers();
        for (channel, text) in [
            ("#ops\u{a0}private", "confidential"),
            ("#ops\u{a0}/quit", "hello"),
        ] {
            let mut server = engine(&[channel]);
            app.routing = Some(1);
            let index = app.ensure(Kind::Channel, channel);
            app.routing = None;
            app.active = index;
            type_draft(&mut app, text);
            app.enter_line();
            assert_eq!(
                final_lines(&mut server, &rx1),
                vec![format!("PRIVMSG {channel} :{text}")]
            );
        }
    }

    #[test]
    fn an_alias_that_picks_a_target_is_followed_for_the_rest_of_its_run() {
        let (mut app, rx1, _rx2, ops1, _) = two_servers();
        let mut server = engine(&["#ops"]);
        app.settings.aliases.push(crate::settings::Alias {
            name: "privately".into(),
            body: "/query alice | /say confidential".into(),
        });
        app.active = ops1;
        type_draft(&mut app, "/privately");
        app.enter_line();
        // The same from a timer, and the run after it is back in #ops.
        app.dispatch("/timer 1 1 /privately".into(), Origin::Input);
        fire_timers_now(&mut app);
        type_draft(&mut app, "back in ops");
        app.enter_line();
        assert_eq!(
            final_lines(&mut server, &rx1),
            vec![
                "PRIVMSG alice :confidential",
                "PRIVMSG alice :confidential",
                "PRIVMSG #ops :back in ops"
            ]
        );
    }

    #[test]
    fn a_typed_via_line_with_a_password_is_never_saved() {
        // However the wrapper is spelled, as long as it runs.
        for (prefix, line) in [
            ("/", "/via #ops /join #locked swordfish"),
            ("/", "/via #ops /via #ops /oper admin swordfish"),
            ("/", "/VIA #ops /oper admin swordfish"),
            ("/", "/via\t#ops /oper admin swordfish"),
            ("/", "/via #ops /Via\t#ops /oper admin swordfish"),
            ("#", "#VIA #ops /join #locked swordfish"),
            (".", ".via #ops /oper admin swordfish"),
        ] {
            let (mut app, rx1, _rx2, ops1, _) = two_servers();
            let mut server = engine(&["#ops"]);
            app.settings.keep_history = true;
            app.settings.command_prefix = prefix.into();
            app.active = ops1;
            type_draft(&mut app, line);
            app.enter_line();
            type_draft(&mut app, "hello");
            app.enter_line();
            let joined: Vec<String> = rx1
                .try_iter()
                .flat_map(|line| server.on_user_line(&line).outgoing)
                .collect();
            assert!(
                joined.iter().any(|line| line.contains("swordfish")),
                "{line}: {joined:?}"
            );
            app.save_history();
            assert_eq!(app.settings.history, vec!["hello".to_string()], "{line}");
        }
    }

    /// Runs `/peer` through Enter in `window`, with `body` as its alias.
    fn run_alias(app: &mut OpenIrcApp, window: usize, body: &str) {
        app.settings.aliases.retain(|alias| alias.name != "peer");
        app.settings.aliases.push(crate::settings::Alias {
            name: "peer".into(),
            body: body.into(),
        });
        app.active = window;
        type_draft(app, "/peer");
        app.enter_line();
    }

    #[test]
    fn a_target_chosen_any_way_the_server_side_reads_it_leads_the_run() {
        for (body, channels, expected) in [
            (
                "/query\talice | /say confidential",
                &["#ops"][..],
                "PRIVMSG alice :confidential",
            ),
            (
                "/QUERY  alice | /say confidential",
                &["#ops"],
                "PRIVMSG alice :confidential",
            ),
            (
                "/j ,#private key | /say confidential",
                &["#ops", "#private"],
                "PRIVMSG #private :confidential",
            ),
            (
                "/j &private key | /say confidential",
                &["#ops", "#&private"],
                "PRIVMSG #&private :confidential",
            ),
            // A choice inside a `/via` of the alias's own counts too.
            (
                "/via #ops /query alice | /say confidential",
                &["#ops"],
                "PRIVMSG alice :confidential",
            ),
            // A `/via` of a line's own is its own choice, whatever was chosen
            // before it in the run.
            (
                "/query alice | /via #ops /say to ops",
                &["#ops"],
                "PRIVMSG #ops :to ops",
            ),
        ] {
            let (mut app, rx1, _rx2, ops1, _) = two_servers();
            let mut server = engine(channels);
            // Only `#` makes a channel here, so `&private` is `#&private`.
            server.on_server_line(":irc 005 me CHANTYPES=# :are supported");
            run_alias(&mut app, ops1, body);
            assert_eq!(final_lines(&mut server, &rx1), vec![expected], "{body}");
        }
    }

    #[test]
    fn a_target_chosen_in_the_messages_window_leads_the_run() {
        let (mut app, rx1, _rx2, _, _) = two_servers();
        let mut server = engine(&["#ops"]);
        app.settings.message_window = true;
        private_from(&mut app, "carol");
        let messages = messages_window(&mut app);
        run_alias(&mut app, messages, "/query alice | /say confidential");
        // The next line on its own answers carol again.
        type_draft(&mut app, "for carol");
        app.enter_line();
        assert_eq!(
            final_lines(&mut server, &rx1),
            vec!["PRIVMSG alice :confidential", "PRIVMSG carol :for carol"]
        );
    }

    #[test]
    fn a_menu_choice_or_a_drop_is_one_run() {
        let (mut app, rx1, _rx2, ops1, _) = two_servers();
        let mut server = engine(&["#ops"]);
        app.active = ops1;
        // The tab and tree menus run their popups this way.
        app.run_with_fields("/query alice | /say from the menu", "", "#ops", "");
        app.settings.drop_command = "/query alice | /say dropped $file".into();
        app.drop_files(vec!["notes.txt".into()]);
        type_draft(&mut app, "back in ops");
        app.enter_line();
        let lines = final_lines(&mut server, &rx1);
        assert_eq!(lines[0], "PRIVMSG alice :from the menu");
        assert!(lines[1].starts_with("PRIVMSG alice :dropped "), "{lines:?}");
        assert!(lines[1].ends_with("notes.txt"), "{lines:?}");
        assert_eq!(lines[2..], ["PRIVMSG #ops :back in ops"]);
    }

    #[test]
    fn a_run_number_written_by_hand_is_refused() {
        for spelling in ["/via", "/VIA\t"] {
            let (mut app, rx1, _rx2, ops1, _) = two_servers();
            let mut server = engine(&["#ops"]);
            app.routing = Some(1);
            let status = app.ensure(Kind::Status, "Status");
            app.routing = None;
            // The status window sends its lines unwrapped, so a number
            // written there could claim the run after it.
            let claimed = app.next_run + 2;
            app.active = status;
            type_draft(
                &mut app,
                &format!("{spelling} ={claimed} #ops /query alice"),
            );
            app.enter_line();
            app.active = ops1;
            for _ in 0..2 {
                type_draft(&mut app, "confidential");
                app.enter_line();
            }
            assert_eq!(
                final_lines(&mut server, &rx1),
                vec!["PRIVMSG #ops :confidential"; 2],
                "{spelling}"
            );
            assert!(
                texts(&app, status)
                    .iter()
                    .any(|line| line.contains("cannot start with"))
            );
        }
    }

    #[test]
    fn a_password_to_a_service_never_reaches_saved_history() {
        let alias = "/query NickServ | /say ID swordfish";
        // Where the line is typed: `#ops`, a query, or the Messages window
        // answering someone.
        for (prefix, line, place) in [
            ("/", "/via NickServ ID swordfish", "#ops"),
            ("/", "/via NickServ /say ID swordfish", "#ops"),
            ("/", "/via #ops /via nickserv ID swordfish", "#ops"),
            ("/", "/VIA\tNickServ RELEASE carol swordfish", "#ops"),
            ("#", "#VIA NickServ ID swordfish", "#ops"),
            ("/", "/msg NickServ ID swordfish", "#ops"),
            ("/", "/describe NickServ ID swordfish", "#ops"),
            ("/", "/ctcp NickServ ID swordfish", "#ops"),
            ("/", "/ctcpreply NickServ ID swordfish", "#ops"),
            ("/", "/peer", "#ops"),
            ("/", "/peer2", "#ops"),
            // Typed in a query with the service, with no command at all.
            ("/", "ID swordfish", "NickServ"),
            // Bots not named like services: their login words count.
            ("/", "/msg Q AUTH me swordfish", "#ops"),
            (
                "/",
                "/msg X@channels.undernet.org LOGIN me swordfish",
                "#ops",
            ),
            ("/", "AUTH me swordfish", "Q"),
            ("/", "LOGIN me swordfish", "Messages"),
        ] {
            let (mut app, rx1, _rx2, ops1, _) = two_servers();
            let mut server = engine(&["#ops"]);
            app.settings.keep_history = true;
            app.settings.command_prefix = prefix.into();
            app.settings.aliases.push(crate::settings::Alias {
                name: "peer".into(),
                body: alias.into(),
            });
            app.settings.aliases.push(crate::settings::Alias {
                name: "peer2".into(),
                body: "/target -q NickServ | /say ID swordfish".into(),
            });
            app.active = ops1;
            match place {
                "#ops" => {}
                "Messages" => {
                    app.settings.message_window = true;
                    private_from(&mut app, "X");
                    app.active = messages_window(&mut app);
                }
                nick => {
                    app.routing = Some(1);
                    app.active = app.ensure(Kind::Query, nick);
                    app.routing = None;
                }
            }
            type_draft(&mut app, line);
            app.enter_line();
            app.active = ops1;
            type_draft(&mut app, "hello");
            app.enter_line();
            let wire: Vec<String> = rx1
                .try_iter()
                .flat_map(|line| server.on_user_line(&line).outgoing)
                .collect();
            assert!(
                wire.iter().any(|sent| sent.contains("swordfish")),
                "{line}: {wire:?}"
            );
            app.save_history();
            assert_eq!(app.settings.history, vec!["hello".to_string()], "{line}");
            assert!(
                app.history_saves
                    .iter()
                    .flatten()
                    .all(|saved| !saved.contains("swordfish")),
                "{line}"
            );
        }
    }

    #[test]
    fn a_login_to_a_bot_is_masked_in_the_transcript_the_log_and_debug() {
        let dir = std::env::temp_dir().join(format!("openirc-ui-login-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (mut app, rx1, _rx2, ops1, _) = two_servers();
        app.settings.logging = true;
        app.settings.log_dir = dir.display().to_string();
        // What the connection reports reaches the window as it does from a
        // real network thread.
        let mut feeds = Vec::new();
        for session in &mut app.sessions {
            let (feed, incoming) = std::sync::mpsc::channel();
            session.link.as_mut().unwrap().incoming = incoming;
            feeds.push((session.id, feed));
        }
        let feed = &feeds.iter().find(|(id, _)| *id == 1).unwrap().1;
        // A real connection, with raw lines kept for the Debug window, to a
        // server that reads all it is sent.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let reader = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut seen = Vec::new();
            let _ = std::io::Read::read_to_end(&mut socket, &mut seen);
            String::from_utf8_lossy(&seen).into_owned()
        });
        let mut connection = Connection::new(
            crate::tls::Socket::plain(std::net::TcpStream::connect(address).unwrap()),
            Config::new("me", "me", "Me", Vec::new(), Vec::new()),
        );
        connection.set_debug(Arc::new(AtomicBool::new(true)));
        app.active = ops1;
        for line in [
            "/msg Q AUTH me swordfish",
            "/msg X@channels.undernet.org LOGIN me swordfish",
        ] {
            type_draft(&mut app, line);
            app.enter_line();
        }
        for line in rx1.try_iter() {
            let effect = connection.handle_user(&line).unwrap();
            feed.send(NetMsg::Wire(connection.take_wire())).unwrap();
            feed.send(NetMsg::Effect(effect)).unwrap();
        }
        app.poll();
        drop(connection);
        // The password went out.
        assert!(reader.join().unwrap().contains("AUTH me swordfish"));
        let debug = window(&app, Kind::List, DEBUG, Some(1));
        assert_eq!(
            texts(&app, debug),
            vec![
                "* -> PRIVMSG Q :AUTH ****",
                "* -> PRIVMSG X@channels.undernet.org :LOGIN ****"
            ]
        );
        let q = window(&app, Kind::Query, "Q", Some(1));
        let x = window(&app, Kind::Query, "X@channels.undernet.org", Some(1));
        assert!(
            texts(&app, q)
                .iter()
                .any(|line| line.ends_with("AUTH ****"))
        );
        assert!(
            texts(&app, x)
                .iter()
                .any(|line| line.ends_with("LOGIN ****"))
        );
        let shown: Vec<String> = (0..app.windows.len())
            .flat_map(|at| texts(&app, at))
            .collect();
        assert!(
            shown.iter().all(|line| !line.contains("swordfish")),
            "{shown:?}"
        );
        // And the logs, as written.
        fn read_all(path: &std::path::Path, into: &mut String) {
            if path.is_dir() {
                for entry in std::fs::read_dir(path).unwrap() {
                    read_all(&entry.unwrap().path(), into);
                }
            } else {
                into.push_str(&std::fs::read_to_string(path).unwrap());
            }
        }
        let mut logged = String::new();
        read_all(&dir, &mut logged);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(logged.contains("AUTH ****"), "{logged}");
        assert!(logged.contains("LOGIN ****"), "{logged}");
        assert!(!logged.contains("swordfish"), "{logged}");
    }

    #[test]
    fn words_that_only_look_like_a_login_are_shown_and_kept() {
        let (mut app, rx1, _rx2, ops1, _) = two_servers();
        let mut server = engine(&["#ops"]);
        app.settings.keep_history = true;
        app.active = ops1;
        for line in ["/msg bob set the table", "Login page is broken"] {
            type_draft(&mut app, line);
            app.enter_line();
        }
        for line in rx1.try_iter() {
            let effect = server.on_user_line(&line);
            app.apply_effect(1, effect);
        }
        let bob = window(&app, Kind::Query, "bob", Some(1));
        assert!(
            texts(&app, bob)
                .iter()
                .any(|line| line.ends_with("set the table"))
        );
        // A channel is no bot, and everyone in it reads what is said there.
        assert!(
            texts(&app, ops1)
                .iter()
                .any(|line| line.ends_with("Login page is broken"))
        );
        app.save_history();
        // Kept. The channel line is not: the typed line alone does not say
        // where it went, so a login's first word keeps it out wherever.
        assert_eq!(
            app.settings.history,
            vec!["/msg bob set the table".to_string()]
        );
    }

    #[test]
    fn a_queued_line_never_reaches_whoever_takes_the_nick_of_someone_who_left() {
        let (mut app, rx1, _rx2, _, _) = two_servers();
        let mut server = engine(&["#ops"]);
        app.settings.message_window = true;
        app.settings.paste_confirm = 3;
        private_from(&mut app, "carol");
        app.active = messages_window(&mut app);
        type_draft(&mut app, "first\nconfidential\nthird");
        app.enter_line();
        app.send_paste();
        // And a line typed in carol's query, and a timer firing there.
        app.routing = Some(1);
        app.active = app.ensure(Kind::Query, "carol");
        app.routing = None;
        type_draft(&mut app, "typed for carol");
        app.enter_line();
        app.dispatch("/timer 1 1 timed for carol".into(), Origin::Input);
        fire_timers_now(&mut app);
        // All of it is queued when the server side reads that carol quit,
        // and that mallory took her nick.
        server.on_server_line(":carol!c@h QUIT :bye");
        server.on_server_line(":mallory!m@h NICK :carol");
        let effects: Vec<Effect> = rx1
            .try_iter()
            .map(|line| server.on_user_line(&line))
            .collect();
        let wire: Vec<&String> = effects.iter().flat_map(|effect| &effect.outgoing).collect();
        assert!(wire.is_empty(), "{wire:?}");
        let notes: Vec<&String> = effects.iter().flat_map(|effect| &effect.lines).collect();
        assert_eq!(notes.len(), 5, "{notes:?}");
        assert!(
            notes
                .iter()
                .all(|note| *note == "* a line for carol was not sent: carol left meanwhile"),
            "{notes:?}"
        );
    }

    #[test]
    fn what_was_written_for_someone_who_left_never_reaches_whoever_takes_the_nick() {
        let (mut app, rx1, _rx2, _, _) = two_servers();
        let mut server = engine(&["#ops"]);
        app.settings.message_window = true;
        app.settings.paste_confirm = 3;
        private_from(&mut app, "carol");
        let messages = messages_window(&mut app);
        app.active = messages;
        // All answering carol: a paste waiting to be confirmed, a timer, a
        // draft begun, and a typing notice in flight.
        type_draft(&mut app, "first\nconfidential\nthird");
        app.enter_line();
        app.dispatch("/timer 1 1 timed for carol".into(), Origin::Input);
        type_draft(&mut app, "drafted for carol");
        app.typing_sent = Some((1, "carol".into(), std::time::Instant::now()));
        // The window sees carol quit and mallory take her nick, so what it
        // sends from now on carries a count past her quit.
        for line in [":carol!c@h QUIT :bye", ":mallory!m@h NICK :carol"] {
            let effect = server.on_server_line(line);
            app.apply_effect(1, effect);
        }
        assert_eq!(app.sessions[0].renames, 2);
        assert!(!app.last_private.contains_key(&Some(1)));
        assert!(app.typing_sent.is_none());
        // Then the paste is confirmed, the timer fires, and the draft is
        // entered.
        app.send_paste();
        fire_timers_now(&mut app);
        app.enter_line();
        let wire = final_lines(&mut server, &rx1);
        assert!(wire.is_empty(), "{wire:?}");
        let notes: Vec<String> = texts(&app, messages)
            .into_iter()
            .filter(|line| line.contains("left"))
            .collect();
        assert_eq!(
            notes,
            vec![
                "* carol left, so a line was not sent.",
                "* carol left, so a line was not sent.",
                "* carol left, so a line was not sent.",
                "* carol left, so a line was not sent.",
                "* carol left while you typed; press Enter again to send this to whoever this window answers now.",
            ]
        );
        // Entered again, it answers whoever the window answers now: nobody.
        app.enter_line();
        assert!(final_lines(&mut server, &rx1).is_empty());
        let shown: Vec<String> = (0..app.windows.len())
            .flat_map(|at| texts(&app, at))
            .collect();
        for words in ["first", "confidential", "third", "timed for", "drafted for"] {
            assert!(
                shown.iter().all(|line| !line.contains(words)),
                "{words}: {shown:?}"
            );
        }
        // A new message from whoever has the nick now starts afresh.
        private_from(&mut app, "carol");
        type_draft(&mut app, "");
        type_draft(&mut app, "welcome");
        app.enter_line();
        assert_eq!(
            final_lines(&mut server, &rx1),
            vec!["PRIVMSG carol :welcome"]
        );
    }

    #[test]
    fn someone_who_left_is_not_followed_through_the_next_holders_rename() {
        let (mut app, rx1, _rx2, _, _) = two_servers();
        let mut server = engine(&["#ops"]);
        app.settings.message_window = true;
        private_from(&mut app, "carol");
        let messages = messages_window(&mut app);
        app.active = messages;
        app.dispatch("/timer 1 1 timed for carol".into(), Origin::Input);
        // Carol quits, mallory takes her nick and then moves on to another.
        for line in [
            ":carol!c@h QUIT :bye",
            ":mallory!m@h NICK :carol",
            ":carol!m@h NICK :mallory2",
        ] {
            let effect = server.on_server_line(line);
            app.apply_effect(1, effect);
        }
        fire_timers_now(&mut app);
        let wire = final_lines(&mut server, &rx1);
        assert!(wire.is_empty(), "{wire:?}");
        let notes: Vec<String> = texts(&app, messages)
            .into_iter()
            .filter(|line| line.contains("left"))
            .collect();
        assert_eq!(notes, vec!["* carol left, so a line was not sent."]);
    }

    #[test]
    fn a_queued_line_follows_a_nick_change_the_server_side_saw_first() {
        let (mut app, rx1, _rx2, _, _) = two_servers();
        let mut server = engine(&["#ops"]);
        app.settings.message_window = true;
        app.settings.paste_confirm = 3;
        private_from(&mut app, "carol");
        app.active = messages_window(&mut app);
        type_draft(&mut app, "first\nconfidential\nthird");
        app.enter_line();
        app.send_paste();
        // And a line typed in carol's query, and a timer firing there.
        app.routing = Some(1);
        app.active = app.ensure(Kind::Query, "carol");
        app.routing = None;
        type_draft(&mut app, "typed for carol");
        app.enter_line();
        app.dispatch("/timer 1 1 timed for carol".into(), Origin::Input);
        fire_timers_now(&mut app);
        // All of it is queued when the server side reads carol's rename,
        // and someone else taking her old nick, before sending any of it.
        let news = [
            server.on_server_line(":carol!c@h NICK :caroline"),
            server.on_server_line(":mallory!m@h NICK :carol"),
        ];
        assert_eq!(
            final_lines(&mut server, &rx1),
            vec![
                "PRIVMSG caroline :first",
                "PRIVMSG caroline :confidential",
                "PRIVMSG caroline :third",
                "PRIVMSG caroline :typed for carol",
                "PRIVMSG caroline :timed for carol",
            ]
        );
        // Once the window has seen the renames too, the new carol is the one
        // meant when someone writes to carol.
        for effect in news {
            app.apply_effect(1, effect);
        }
        type_draft(&mut app, "/msg carol hello newcomer");
        app.enter_line();
        type_draft(&mut app, "/query carol");
        app.enter_line();
        type_draft(&mut app, "welcome");
        app.enter_line();
        assert_eq!(
            final_lines(&mut server, &rx1),
            vec!["PRIVMSG carol :hello newcomer", "PRIVMSG carol :welcome"]
        );
    }

    #[test]
    fn a_window_switch_queued_before_a_rename_follows_it() {
        let (mut app, rx1, _rx2, _, _) = two_servers();
        let mut server = engine(&["#ops"]);
        app.routing = Some(1);
        app.ensure(Kind::Query, "carol");
        let status = app.ensure(Kind::Status, "Status");
        app.routing = None;
        let carol = window(&app, Kind::Query, "carol", Some(1));
        // Carol's query, then the status window, whose words go to the
        // server side's own target: carol, as the switch left it.
        app.activate(carol);
        app.activate(status);
        type_draft(&mut app, "confidential for carol");
        app.enter_line();
        // All queued when the server side reads her rename, and someone
        // else taking her old nick.
        server.on_server_line(":carol!c@h NICK :caroline");
        server.on_server_line(":mallory!m@h NICK :carol");
        assert_eq!(
            final_lines(&mut server, &rx1),
            vec!["PRIVMSG caroline :confidential for carol"]
        );
    }

    #[test]
    fn a_window_switch_inside_a_run_keeps_what_the_run_chose() {
        let (mut app, rx1, _rx2, ops1, _) = two_servers();
        let mut server = engine(&["#ops"]);
        run_alias(
            &mut app,
            ops1,
            "/query alice | /channel #ops | /say confidential for alice",
        );
        assert_eq!(
            final_lines(&mut server, &rx1),
            vec!["PRIVMSG alice :confidential for alice"]
        );
    }

    #[test]
    fn a_typing_notice_follows_a_rename_the_window_saw() {
        let (mut app, rx1, _rx2, _, _) = two_servers();
        let mut server = engine(&[]);
        server.on_server_line(":irc CAP me ACK :message-tags");
        app.settings.send_typing = true;
        app.routing = Some(1);
        app.ensure(Kind::Query, "carol");
        let status = app.ensure(Kind::Status, "Status");
        app.routing = None;
        app.activate(window(&app, Kind::Query, "carol", Some(1)));
        app.input = "typing for carol".into();
        app.send_typing();
        let typing = |server: &mut crate::pump::Pump| {
            rx1.try_iter()
                .flat_map(|line| server.on_user_line(&line).outgoing)
                .filter(|line| line.contains("TAGMSG"))
                .collect::<Vec<_>>()
        };
        assert_eq!(typing(&mut server), vec!["@+typing=active TAGMSG carol"]);
        // Carol renames, someone takes her old nick, and the window sees it.
        for line in [":carol!c@h NICK :caroline", ":mallory!m@h NICK :carol"] {
            let effect = server.on_server_line(line);
            app.apply_effect(1, effect);
        }
        app.activate(status);
        assert_eq!(typing(&mut server), vec!["@+typing=done TAGMSG caroline"]);
    }

    #[test]
    fn words_never_sent_are_never_shown() {
        let (mut app, rx1, _rx2, _, _) = two_servers();
        let mut server = engine(&["#ops"]);
        app.routing = Some(1);
        app.ensure(Kind::Query, "NickServ");
        app.routing = None;
        let query = window(&app, Kind::Query, "NickServ", Some(1));
        app.active = query;
        // An action to the service is masked where it is shown.
        type_draft(&mut app, "/me ID swordfish");
        app.enter_line();
        for line in rx1.try_iter() {
            let effect = server.on_user_line(&line);
            app.apply_effect(1, effect);
        }
        // A timer set there, firing once the query is closed.
        app.active = query;
        app.dispatch("/timer 1 1 /say ID swordfish".into(), Origin::Input);
        app.close_window(query);
        fire_timers_now(&mut app);
        assert!(final_lines(&mut server, &rx1).is_empty());
        let shown: Vec<String> = (0..app.windows.len())
            .flat_map(|at| texts(&app, at))
            .collect();
        assert!(
            shown
                .iter()
                .any(|line| line.contains("a line was not sent")),
            "{shown:?}"
        );
        assert!(
            shown.iter().all(|line| !line.contains("swordfish")),
            "{shown:?}"
        );
    }

    #[test]
    fn words_for_the_server_sides_own_target_stay_out_of_saved_history() {
        let (mut app, rx1, _rx2, ops1, _) = two_servers();
        let mut server = engine(&["#ops"]);
        app.settings.keep_history = true;
        app.routing = Some(1);
        let status = app.ensure(Kind::Status, "Status");
        app.routing = None;
        app.active = status;
        for line in [
            "/target NickServ",
            "/say ID swordfish",
            "ID swordfish again",
        ] {
            type_draft(&mut app, line);
            app.enter_line();
        }
        app.active = ops1;
        type_draft(&mut app, "hello");
        app.enter_line();
        assert_eq!(
            final_lines(&mut server, &rx1),
            vec![
                "PRIVMSG NickServ :ID swordfish",
                "PRIVMSG NickServ :ID swordfish again",
                "PRIVMSG #ops :hello",
            ]
        );
        app.save_history();
        assert_eq!(app.settings.history, vec!["hello".to_string()]);
        assert!(
            app.history_saves
                .iter()
                .flatten()
                .all(|saved| !saved.contains("swordfish"))
        );
    }

    #[test]
    fn a_dcc_run_that_chooses_someone_on_irc_never_tells_the_chat() {
        let (mut app, rx1, _rx2, _, _) = two_servers();
        let mut server = engine(&["#ops"]);
        let (to_bob, _peer) = fake_dcc(&mut app, Some(1), "bob");
        let chat = window(&app, Kind::Dcc, "DCC bob", Some(1));
        run_alias(
            &mut app,
            chat,
            "/query alice | /say confidential | /me waves",
        );
        app.run_line_action(
            "",
            LineAction::Popup {
                body: "/query alice | plain words".into(),
                nick: String::new(),
                text: String::new(),
            },
        );
        // From a timer set in the chat, too.
        app.dispatch("/timer 1 1 /peer".into(), Origin::Input);
        fire_timers_now(&mut app);
        // A choice that did not take sends nothing at all.
        run_alias(&mut app, chat, "/query | /say nobody chosen");
        let lines = final_lines(&mut server, &rx1);
        assert_eq!(
            lines,
            vec![
                "PRIVMSG alice :confidential",
                "PRIVMSG alice :\u{1}ACTION waves\u{1}",
                "PRIVMSG alice :plain words",
                "PRIVMSG alice :confidential",
                "PRIVMSG alice :\u{1}ACTION waves\u{1}",
            ]
        );
        // Bob got only the line that chose nothing.
        assert_eq!(to_bob.try_iter().collect::<Vec<_>>(), vec!["nobody chosen"]);
        // And the next run in the chat is the chat's again.
        type_draft(&mut app, "hello bob");
        app.enter_line();
        assert_eq!(to_bob.try_iter().collect::<Vec<_>>(), vec!["hello bob"]);
        assert!(final_lines(&mut server, &rx1).is_empty());
    }

    #[test]
    fn a_run_that_chooses_someone_in_an_empty_messages_window_reaches_them() {
        let (mut app, rx1, _rx2, _, _) = two_servers();
        let mut server = engine(&["#ops"]);
        app.settings.message_window = true;
        let messages = messages_window(&mut app);
        run_alias(&mut app, messages, "/query alice | /say confidential");
        assert_eq!(
            final_lines(&mut server, &rx1),
            vec!["PRIVMSG alice :confidential"]
        );
    }

    #[test]
    fn opening_a_query_puts_it_in_front_for_what_is_typed_next() {
        let (mut app, rx1, rx2, ops1, ops2) = two_servers();
        let mut one = engine(&["#ops"]);
        let mut two = engine(&["#ops"]);
        // The Open query prompt, on each server, with no message.
        for (ops, server) in [(ops1, Some(1)), (ops2, Some(2))] {
            app.active = ops;
            app.open_prompt(prompts::PromptKind::Query);
            let prompt = app.prompt.take().unwrap();
            app.finish_prompt(&prompt, &["alice".to_string(), String::new()])
                .unwrap();
            assert_eq!(app.active, window(&app, Kind::Query, "alice", server));
            type_draft(&mut app, "for alice");
            app.enter_line();
        }
        // A typed /query on its own.
        app.active = ops1;
        type_draft(&mut app, "/query bob");
        app.enter_line();
        assert_eq!(app.active, window(&app, Kind::Query, "bob", Some(1)));
        type_draft(&mut app, "for bob");
        app.enter_line();
        assert_eq!(
            final_lines(&mut one, &rx1),
            vec!["PRIVMSG alice :for alice", "PRIVMSG bob :for bob"]
        );
        assert_eq!(
            final_lines(&mut two, &rx2),
            vec!["PRIVMSG alice :for alice"]
        );
    }

    #[test]
    fn a_popup_choosing_a_target_leads_only_its_own_lines() {
        let (mut app, rx1, rx2, ops1, ops2) = two_servers();
        let (to_bob, _peer) = fake_dcc(&mut app, Some(1), "bob");
        let chat = window(&app, Kind::Dcc, "DCC bob", Some(1));
        let mut one = engine(&["#ops", "#public"]);
        let mut two = engine(&["#ops"]);
        let popup = |app: &mut OpenIrcApp, body: &str| {
            app.run_line_action(
                "#ops",
                LineAction::Popup {
                    body: body.into(),
                    nick: String::new(),
                    text: String::new(),
                },
            );
        };
        app.active = ops1;
        popup(&mut app, "/join #public | /say hello public");
        // Then a popup in bob's chat, and one on the other server.
        app.active = chat;
        popup(&mut app, "/say confidential");
        app.active = ops1;
        popup(&mut app, "/query alice");
        app.active = ops2;
        popup(&mut app, "/say for ops on two");
        assert_eq!(
            final_lines(&mut one, &rx1),
            vec!["PRIVMSG #public :hello public"]
        );
        assert_eq!(to_bob.try_iter().collect::<Vec<_>>(), vec!["confidential"]);
        assert_eq!(
            final_lines(&mut two, &rx2),
            vec!["PRIVMSG #ops :for ops on two"]
        );
    }

    #[test]
    fn scon_in_a_timer_speaks_on_every_server() {
        let (mut app, rx1, rx2, ops1, _) = two_servers();
        app.settings.script_text = "alias tell { say told $chan }".into();
        let (to_bob, _peer) = fake_dcc(&mut app, Some(1), "bob");
        let chat = window(&app, Kind::Dcc, "DCC bob", Some(1));
        for (origin, command, each) in [
            (ops1, "/scon -a /say scoped-note", "/say scoped-note"),
            (ops1, "/scon -a /me acts", "/me acts"),
            (ops1, "/scon -a /tell", "/say told"),
            (chat, "/scon -a /me from-dcc", "/me from-dcc"),
        ] {
            app.active = origin;
            app.dispatch(format!("/timer 1 1 {command}"), Origin::Input);
            fire_timers_now(&mut app);
            assert_eq!(sent(&rx1), vec![each.to_string()], "{command}");
            assert_eq!(sent(&rx2), vec![each.to_string()], "{command}");
            assert!(
                to_bob.try_iter().next().is_none(),
                "{command}: nothing to bob"
            );
        }
        // A timer set under /scon in a timer: on each server, once.
        app.active = ops1;
        app.dispatch(
            "/timer 1 1 /scon -a /timer 1 1 /say nested".into(),
            Origin::Input,
        );
        fire_timers_now(&mut app);
        fire_timers_now(&mut app);
        assert_eq!(sent(&rx1), vec!["/say nested".to_string()]);
        assert_eq!(sent(&rx2), vec!["/say nested".to_string()]);
    }

    #[test]
    fn plain_words_say_and_me_go_where_the_timer_was_set() {
        let (mut app, rx1, rx2, dcc_rx, _peer) = destinations();
        app.settings.message_window = true;
        app.routing = Some(1);
        app.apply_event(Event::Chat {
            channel: false,
            target: "carol".into(),
            from: "carol".into(),
            text: "hi".into(),
            action: false,
        });
        let messages = app.find(Kind::List, MESSAGES).unwrap();
        app.routing = None;
        let ops1 = window(&app, Kind::Channel, "#ops", Some(1));
        let query = window(&app, Kind::Query, "alice", Some(1));
        let dcc = window(&app, Kind::Dcc, "DCC bob", Some(1));
        let ops2 = window(&app, Kind::Channel, "#ops", Some(2));
        let _ = sent(&rx1);
        for (origin, expected) in [
            (ops1, vec!["/msg #ops a", "/msg #ops b", "/describe #ops c"]),
            (
                query,
                vec!["/msg alice a", "/msg alice b", "/describe alice c"],
            ),
            (
                messages,
                vec!["/msg carol a", "/msg carol b", "/describe carol c"],
            ),
        ] {
            // Fresh timers for each window in front when they fire: another
            // channel of the same server, then a channel of another network.
            let public = window(&app, Kind::Channel, "#public", Some(1));
            for front in [public, ops2] {
                app.active = origin;
                for command in ["a", "/say b", "/me c"] {
                    app.dispatch(format!("/timer 30 1 {command}"), Origin::Input);
                }
                app.active = front;
                fire_timers_now(&mut app);
                assert_eq!(
                    said(&rx1),
                    expected,
                    "origin {}, front {}",
                    app.windows[origin].title,
                    app.windows[front].title
                );
                assert!(sent(&rx2).is_empty());
                assert!(dcc_rx.try_iter().next().is_none());
            }
        }
        // From the DCC chat: the chat, with the other windows in front.
        app.active = dcc;
        for command in ["a", "/say b", "/me c"] {
            app.dispatch(format!("/timer 30 1 {command}"), Origin::Input);
        }
        app.active = ops2;
        fire_timers_now(&mut app);
        let to_bob: Vec<String> = dcc_rx.try_iter().collect();
        assert_eq!(to_bob, vec!["a", "b", "\u{1}ACTION c\u{1}"]);
        assert!(sent(&rx1).is_empty() && sent(&rx2).is_empty());
        // Typed /say in a DCC chat stays in the chat too.
        app.active = dcc;
        app.dispatch("/say typed".into(), Origin::Input);
        assert_eq!(dcc_rx.try_iter().collect::<Vec<_>>(), vec!["typed"]);
    }

    #[test]
    fn a_timer_runs_a_script_alias_for_the_window_it_was_set_in() {
        let (mut app, rx1, rx2, dcc_rx, _peer) = destinations();
        app.settings.script_text = "alias announce { msg $chan $1- }".into();
        let ops1 = window(&app, Kind::Channel, "#ops", Some(1));
        let public = window(&app, Kind::Channel, "#public", Some(1));
        let ops2 = window(&app, Kind::Channel, "#ops", Some(2));
        let dcc = window(&app, Kind::Dcc, "DCC bob", Some(1));
        for front in [public, ops2, dcc] {
            app.active = ops1;
            app.dispatch("/timer 30 1 /announce private-note".into(), Origin::Input);
            app.active = front;
            fire_timers_now(&mut app);
            assert_eq!(sent(&rx1), vec!["/msg #ops private-note".to_string()]);
            assert!(sent(&rx2).is_empty());
            assert!(dcc_rx.try_iter().next().is_none());
        }
    }

    #[test]
    fn a_timer_too_far_ahead_is_refused_not_a_crash() {
        let mut app = app();
        app.active = 0;
        app.dispatch(
            "/timer 18446744073709551615 1 /echo done".into(),
            Origin::Input,
        );
        app.dispatch(
            format!("/timer {} 1 /echo done", TIMER_LIMIT.as_secs() + 1),
            Origin::Input,
        );
        assert!(app.timers.is_empty());
        assert!(
            texts(&app, 0)
                .iter()
                .any(|line| line.contains("at most a year"))
        );
        app.dispatch(
            format!("/timer {} 2 /echo done", TIMER_LIMIT.as_secs()),
            Origin::Input,
        );
        assert_eq!(app.timers.len(), 1, "a year is still allowed");
        // Rescheduling past what the clock holds ends the timer instead.
        app.timers[0].next = std::time::Instant::now();
        app.timers[0].every = Duration::from_secs(u64::MAX);
        app.tick_timers();
        assert!(app.timers.is_empty());
        let (mut app, _rx1, _rx2, ops1, _) = two_servers();
        app.active = ops1;
        app.dispatch("/ban -u18446744073709551615 bob".into(), Origin::Input);
        assert!(app.timers.is_empty());
    }

    #[test]
    fn channel_central_changes_go_to_its_own_server() {
        let (mut app, rx1, rx2, ops1, _) = two_servers();
        app.active = ops1;
        app.open_channel_central();
        let mut central = app.central.take().unwrap();
        let _ = sent(&rx1);
        // Now another server's window is in front.
        app.active = app.windows.len() - 1;
        app.central_actions(
            &mut central,
            ops1,
            None,
            Some(('b', "x!*@*".into())),
            Some(('b', "y!*@*".into())),
            false,
        );
        assert_eq!(
            sent(&rx1),
            vec![
                "/raw MODE #ops -b x!*@*".to_string(),
                "/raw MODE #ops +b y!*@*".to_string()
            ]
        );
        assert!(sent(&rx2).is_empty());
    }

    #[test]
    fn the_messages_window_answers_the_person_on_its_own_server() {
        let (mut app, rx1, rx2, _, _) = two_servers();
        app.settings.message_window = true;
        for (id, nick) in [(1, "alice"), (2, "bob")] {
            app.routing = Some(id);
            app.apply_event(Event::Chat {
                channel: false,
                target: nick.into(),
                from: nick.into(),
                text: "hi".into(),
                action: false,
            });
        }
        app.routing = Some(1);
        app.active = app.find(Kind::List, MESSAGES).unwrap();
        app.routing = None;
        app.say("hello".into());
        assert_eq!(said(&rx1), vec!["/msg alice hello".to_string()]);
        assert!(sent(&rx2).is_empty());
    }

    #[test]
    fn a_new_owner_of_a_nick_does_not_inherit_the_old_address() {
        let (mut app, _rx1, _rx2, _, _) = two_servers();
        app.settings.users = vec![("trusted".into(), "friend!*@trusted.example".into())];
        app.settings.default_level = "1".into();
        let effect = |sender: (&str, &str), events: Vec<Event>| Effect {
            lines: Vec::new(),
            events,
            outgoing: Vec::new(),
            quit: false,
            time: None,
            sender: Some((sender.0.to_string(), sender.1.to_string())),
            renames: 0,
        };
        let levels = |app: &mut OpenIrcApp| {
            app.routing = Some(1);
            let levels = app.user_levels("friend");
            app.routing = None;
            levels
        };
        app.apply_effect(
            1,
            effect(
                ("friend", "f@trusted.example"),
                vec![Event::Join {
                    who: "friend".into(),
                    channel: "#ops".into(),
                    address: "f@trusted.example".into(),
                }],
            ),
        );
        assert_eq!(levels(&mut app), vec!["trusted".to_string()]);
        // Someone else takes the nick and writes privately.
        app.apply_effect(1, effect(("friend", "x@elsewhere"), Vec::new()));
        assert_eq!(levels(&mut app), vec!["1".to_string()]);
        // An address left out by the server is not trusted either.
        app.apply_effect(1, effect(("friend", "f@trusted.example"), Vec::new()));
        app.apply_effect(1, effect(("friend", ""), Vec::new()));
        assert_eq!(levels(&mut app), vec!["1".to_string()]);
        // Quitting forgets the address; a nick change moves it.
        app.apply_effect(1, effect(("friend", "f@trusted.example"), Vec::new()));
        app.routing = Some(1);
        app.apply_event(Event::Quit {
            who: "friend".into(),
            reason: None,
        });
        app.routing = None;
        assert_eq!(levels(&mut app), vec!["1".to_string()]);
        app.apply_effect(1, effect(("pal", "f@trusted.example"), Vec::new()));
        app.routing = Some(1);
        app.apply_event(Event::Nick {
            old: "pal".into(),
            new: "friend".into(),
        });
        app.routing = None;
        assert_eq!(levels(&mut app), vec!["trusted".to_string()]);
    }

    #[test]
    fn a_separator_in_remote_text_never_runs_a_second_command_through_an_alias() {
        let mut app = app();
        app.active = 0;
        app.settings.aliases.push(crate::settings::Alias {
            name: "relay".into(),
            body: "/echo $1-".into(),
        });
        app.settings.remotes.push(crate::script::Remote {
            event: "text".into(),
            mask: "*".into(),
            body: "/relay $text".into(),
        });
        let before = app.windows[0].lines.len();
        app.fire_scripts("text", "bob", "#c", "hello | /echo second-command");
        let shown: Vec<String> = texts(&app, 0)[before..].to_vec();
        assert_eq!(shown.len(), 1, "{shown:?}");
        assert!(
            shown[0].contains("hello | /echo second-command"),
            "{shown:?}"
        );
    }

    #[test]
    fn aliases_that_fan_out_stop_at_the_command_budget() {
        let mut app = app();
        app.active = 0;
        let body = ["/boom"; 10].join(" | ");
        app.settings.aliases.push(crate::settings::Alias {
            name: "boom".into(),
            body,
        });
        let started = std::time::Instant::now();
        app.dispatch("/boom".into(), Origin::Input);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(
            texts(&app, 0)
                .iter()
                .any(|line| line.contains("too many commands at once"))
        );
        // The next typed line has its own budget again.
        app.dispatch("/echo fine".into(), Origin::Input);
        assert!(texts(&app, 0).last().unwrap().contains("fine"));
    }

    #[test]
    fn each_server_keeps_its_own_nick() {
        let (mut app, _rx1, _rx2, ops1, ops2) = two_servers();
        app.sessions[0].nick = "alice".into();
        app.sessions[1].nick = "bob".into();
        app.active = ops1;
        assert_eq!(app.me(), "alice");
        app.active = ops2;
        assert_eq!(app.me(), "bob");
        // Someone called bob on server 1 is not us.
        app.windows[ops1].nicks = vec!["@alice".into(), "bob".into()];
        app.routing = Some(1);
        app.apply_event(Event::Kick {
            who: "bob".into(),
            channel: "#ops".into(),
            by: "alice".into(),
            reason: None,
        });
        assert!(app.windows[ops1].nicks.iter().any(|nick| nick == "@alice"));
        // Our own nick change updates that server only.
        app.apply_event(Event::Nick {
            old: "alice".into(),
            new: "carol".into(),
        });
        app.routing = None;
        assert_eq!(app.sessions[0].nick, "carol");
        assert_eq!(app.sessions[1].nick, "bob");
    }

    #[test]
    fn windows_follow_the_servers_case_mapping() {
        let (mut app, _rx1, _rx2, _, _) = two_servers();
        app.sessions[1].casemap = crate::state::CaseMap::Ascii;
        let chat = |target: &str| Event::Chat {
            channel: true,
            target: target.into(),
            from: "bob".into(),
            text: "hi".into(),
            action: false,
        };
        let count = |app: &OpenIrcApp, server: u64| {
            app.windows
                .iter()
                .filter(|window| window.server == Some(server) && window.kind == Kind::Channel)
                .count()
        };
        for server in [1, 2] {
            let effect = Effect {
                lines: Vec::new(),
                events: vec![chat("#room[1]"), chat("#ROOM{1}")],
                outgoing: Vec::new(),
                quit: false,
                time: None,
                sender: None,
                renames: 0,
            };
            app.apply_effect(server, effect);
        }
        // #ops plus one room on the RFC 1459 server, two rooms on the ASCII one.
        assert_eq!(count(&app, 1), 2);
        assert_eq!(count(&app, 2), 3);
    }
}
