//! The window chrome: menus, toolbar, switchbar, tree, status bar, and the
//! keyboard shortcuts. Every menu item, toolbar button, and shortcut runs a
//! [`Command`], so the three never drift apart.

use crate::i18n::{tr, tr_key, trf};
use eframe::egui;

use super::icons::{self, Icon};
use super::prompts::PromptKind;
use super::{Activity, Kind, OpenIrcApp, Origin, color_face, color_hilite, color_paper, text_on};

/// Something the user asked the window to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Command {
    Connect,
    Disconnect,
    Options(usize),
    Favorites,
    Certificate,
    Quit,
    ToggleTree,
    ToggleSwitchbar,
    NickList(NickSide),
    ChannelList,
    AddressBook,
    Urls,
    FullScreen,
    OnTop,
    Profiler,
    Join,
    Part,
    Names,
    Whois,
    Who,
    Dcc,
    Transfers,
    Away,
    Back,
    Invite,
    Query,
    Notice,
    /// A CTCP request to someone; `None` asks which.
    Ctcp(Option<&'static str>),
    Ban,
    Kick,
    Ignore,
    Unignore,
    ChangeNick,
    /// Leave the server with a reason.
    QuitIrc,
    Hop,
    Clear,
    ChannelCentral,
    Scripts,
    Dialogs,
    Log,
    ClearHistory,
    MonitorPanel,
    SearchAll,
    ToggleLogging,
    NextWindow,
    PreviousWindow,
    CloseWindow,
    Find,
    Help,
    About,
}

/// Toolbar buttons that can be hidden, for the Options window.
pub(super) const TOOLBAR_CHOICES: [(Icon, &str); 17] = [
    (Icon::Connect, tr_key("Connect")),
    (Icon::Disconnect, tr_key("Disconnect")),
    (Icon::Options, tr_key("Options")),
    (Icon::Favorites, tr_key("Favorites")),
    (Icon::Address, tr_key("Address book")),
    (Icon::Notify, tr_key("Notify list")),
    (Icon::Join, tr_key("Join a channel")),
    (Icon::Part, tr_key("Leave the channel")),
    (Icon::ChannelList, tr_key("Channels list")),
    (Icon::Channel, tr_key("Channel central")),
    (Icon::Scripts, tr_key("Scripts editor")),
    (Icon::Logs, tr_key("Log files")),
    (Icon::Urls, tr_key("URL list")),
    (Icon::Dcc, tr_key("Send a file")),
    (Icon::Transfers, tr_key("Transfers")),
    (Icon::Colors, tr_key("Colors")),
    (Icon::Help, tr_key("About")),
];

/// A toolbar button: icon, caption, tooltip, command, and whether it is
/// enabled.
type ToolButton<'a> = (Icon, &'a str, &'a str, Command, bool);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NickSide {
    Left,
    Right,
    Hidden,
}

impl NickSide {
    pub(super) fn from_setting(value: &str) -> Self {
        match value {
            "left" => Self::Left,
            "hidden" => Self::Hidden,
            _ => Self::Right,
        }
    }

    fn setting(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Hidden => "hidden",
        }
    }
}

/// Keyboard shortcuts and the command each one runs.
pub(super) const SHORTCUTS: &[(egui::KeyboardShortcut, Command)] = {
    use egui::{Key, KeyboardShortcut as K, Modifiers as M};
    &[
        (K::new(M::ALT, Key::E), Command::Connect),
        (K::new(M::ALT, Key::O), Command::Options(0)),
        (K::new(M::ALT, Key::J), Command::Favorites),
        (K::new(M::ALT, Key::L), Command::ChannelList),
        (K::new(M::ALT, Key::R), Command::Scripts),
        (K::new(M::ALT, Key::B), Command::AddressBook),
        (K::new(M::ALT, Key::U), Command::Urls),
        (K::new(M::ALT, Key::K), Command::Options(5)),
        (K::new(M::ALT, Key::S), Command::Dcc),
        (K::new(M::ALT, Key::Z), Command::CloseWindow),
        (K::new(M::CTRL, Key::W), Command::CloseWindow),
        (
            K::new(M::CTRL.plus(M::SHIFT), Key::Tab),
            Command::PreviousWindow,
        ),
        (K::new(M::CTRL, Key::Tab), Command::NextWindow),
        (K::new(M::CTRL, Key::PageDown), Command::NextWindow),
        (K::new(M::CTRL, Key::PageUp), Command::PreviousWindow),
        (K::new(M::CTRL.plus(M::SHIFT), Key::F), Command::SearchAll),
        (K::new(M::CTRL, Key::F), Command::Find),
        (K::new(M::NONE, Key::F11), Command::FullScreen),
        (K::new(M::NONE, Key::F1), Command::Help),
        (K::new(M::NONE, Key::F2), Command::MonitorPanel),
    ]
};

fn shortcut_text(ctx: &egui::Context, command: Command) -> Option<String> {
    SHORTCUTS
        .iter()
        .find(|(_, bound)| *bound == command)
        .map(|(shortcut, _)| ctx.format_shortcut(shortcut))
}

impl OpenIrcApp {
    /// Run keyboard shortcuts pressed this frame.
    pub(super) fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let mut pressed = Vec::new();
        let mut pick = None;
        ctx.input_mut(|input| {
            for (shortcut, command) in SHORTCUTS {
                if input.consume_shortcut(shortcut) {
                    pressed.push(*command);
                }
            }
            // Alt+1 .. Alt+9 pick a window by its switchbar position.
            let digits = [
                egui::Key::Num1,
                egui::Key::Num2,
                egui::Key::Num3,
                egui::Key::Num4,
                egui::Key::Num5,
                egui::Key::Num6,
                egui::Key::Num7,
                egui::Key::Num8,
                egui::Key::Num9,
            ];
            for (slot, key) in digits.into_iter().enumerate() {
                if input.consume_key(egui::Modifiers::ALT, key) {
                    pick = Some(slot);
                }
            }
            // The mouse's back and forward side buttons step through the
            // windows like Ctrl+Shift+Tab and Ctrl+Tab.
            if input.pointer.button_pressed(egui::PointerButton::Extra1) {
                pressed.push(Command::PreviousWindow);
            }
            if input.pointer.button_pressed(egui::PointerButton::Extra2) {
                pressed.push(Command::NextWindow);
            }
        });
        if let Some(slot) = pick
            && slot < self.windows.len()
        {
            self.activate(slot);
        }
        for command in pressed {
            self.run(ctx, command);
        }
    }

    pub(super) fn run(&mut self, ctx: &egui::Context, command: Command) {
        match command {
            Command::Connect => self.open_server_list(),
            Command::Disconnect => self.disconnect(),
            Command::Options(page) => self.open_options(page),
            Command::Favorites => self.show_favorites = true,
            Command::Certificate => self.show_cert = true,
            Command::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Command::ToggleTree => self.show_tree = !self.show_tree,
            Command::ToggleSwitchbar => self.show_switchbar = !self.show_switchbar,
            Command::NickList(side) => {
                self.settings.nicklist = side.setting().into();
                self.persist();
            }
            Command::ChannelList => self.open_channel_list(),
            Command::AddressBook => self.show_address = true,
            Command::Urls => {
                let index = self.ensure(Kind::List, "URLs");
                self.activate(index);
            }
            Command::FullScreen => {
                self.full_screen = !self.full_screen;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.full_screen));
            }
            Command::OnTop => {
                self.on_top = !self.on_top;
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if self.on_top {
                    egui::WindowLevel::AlwaysOnTop
                } else {
                    egui::WindowLevel::Normal
                }));
            }
            Command::Profiler => self.show_profiler = true,
            Command::Join => self.open_prompt(PromptKind::Join),
            Command::Part => self.part_active(),
            Command::Names => self.send_user("/names".into()),
            Command::Whois => self.open_prompt(PromptKind::Whois),
            Command::Who => self.send_user("/who".into()),
            Command::Dcc => self.show_dcc = true,
            Command::Transfers => self.show_transfers = true,
            Command::Away => self.open_prompt(PromptKind::Away),
            Command::Back => self.send_user("/back".into()),
            Command::Invite => self.open_prompt(PromptKind::Invite),
            Command::Query => self.open_prompt(PromptKind::Query),
            Command::Notice => self.open_prompt(PromptKind::Notice),
            Command::Ctcp(request) => self.open_prompt(PromptKind::Ctcp(request)),
            Command::Ban => self.open_prompt(PromptKind::Ban),
            Command::Kick => self.open_prompt(PromptKind::Kick),
            Command::Ignore => self.open_prompt(PromptKind::Ignore),
            Command::Unignore => self.open_prompt(PromptKind::Unignore),
            Command::ChangeNick => self.open_prompt(PromptKind::Nick),
            Command::QuitIrc => self.open_prompt(PromptKind::Quit),
            Command::Hop => self.send_user("/hop".into()),
            Command::Clear => self.dispatch("/clear".into(), Origin::Script),
            Command::ChannelCentral => self.open_channel_central(),
            Command::Scripts => self.open_scripts(),
            Command::Dialogs => self.show_dialog_editor = true,
            Command::Log => self.open_logs(),
            Command::SearchAll => self.search.open = true,
            Command::MonitorPanel => {
                self.settings.monitor_panel = !self.settings.monitor_panel;
                self.persist();
            }
            Command::ClearHistory => self.clear_history = Some([true, true, true, true, false]),
            Command::ToggleLogging => {
                self.settings.logging = !self.settings.logging;
                self.persist();
                self.note_status(if self.settings.logging {
                    trf("Logging to {0}", &[&self.settings.log_dir])
                } else {
                    tr("Logging off.").into()
                });
            }
            Command::NextWindow => self.cycle_window(1),
            Command::PreviousWindow => self.cycle_window(-1),
            Command::CloseWindow => self.close_window(self.active),
            Command::Find => {
                ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("find-field")));
            }
            Command::Help => self.dispatch("/help".into(), Origin::Script),
            Command::About => self.show_about = true,
        }
    }

    fn cycle_window(&mut self, step: isize) {
        let count = self.windows.len() as isize;
        if count == 0 {
            return;
        }
        let next = (self.active as isize + step).rem_euclid(count) as usize;
        self.activate(next);
    }

    fn menu_item(&mut self, ui: &mut egui::Ui, label: &str, command: Command, enabled: bool) {
        let shortcut = shortcut_text(ui.ctx(), command);
        if menu_row(ui, label, false, shortcut, enabled).clicked() {
            self.run(ui.ctx(), command);
            ui.close();
        }
    }

    fn check_item(&mut self, ui: &mut egui::Ui, label: &str, on: bool, command: Command) {
        let shortcut = shortcut_text(ui.ctx(), command);
        if menu_row(ui, label, on, shortcut, true).clicked() {
            self.run(ui.ctx(), command);
            ui.close();
        }
    }

    pub(super) fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let connected = self.active_connected();
        let on_channel = self
            .windows
            .get(self.active)
            .is_some_and(|window| window.kind == Kind::Channel);
        egui::Panel::top("menu").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button(tr("File"), |ui| {
                    self.menu_item(ui, tr("Servers / connect..."), Command::Connect, true);
                    self.menu_item(ui, tr("Disconnect"), Command::Disconnect, connected);
                    ui.separator();
                    self.menu_item(ui, tr("Favorites..."), Command::Favorites, true);
                    self.menu_item(ui, tr("Certificates..."), Command::Certificate, true);
                    ui.separator();
                    self.menu_item(ui, tr("Options..."), Command::Options(0), true);
                    ui.separator();
                    self.menu_item(ui, tr("Exit"), Command::Quit, true);
                });
                ui.menu_button(tr("View"), |ui| {
                    self.check_item(ui, tr("Treebar"), self.show_tree, Command::ToggleTree);
                    self.check_item(
                        ui,
                        tr("Switchbar"),
                        self.show_switchbar,
                        Command::ToggleSwitchbar,
                    );
                    ui.menu_button((menu_gutter(), tr("Nickname list")), |ui| {
                        let side = NickSide::from_setting(&self.settings.nicklist);
                        for (label, choice) in [
                            (tr("Right"), NickSide::Right),
                            (tr("Left"), NickSide::Left),
                            (tr("Hidden"), NickSide::Hidden),
                        ] {
                            self.check_item(ui, label, side == choice, Command::NickList(choice));
                        }
                    });
                    ui.separator();
                    self.menu_item(ui, tr("Channels list..."), Command::ChannelList, connected);
                    self.menu_item(ui, tr("URLs list"), Command::Urls, true);
                    self.menu_item(ui, tr("Address book..."), Command::AddressBook, true);
                    ui.separator();
                    self.menu_item(ui, tr("Colors..."), Command::Options(5), true);
                    self.check_item(
                        ui,
                        tr("Monitor panel"),
                        self.settings.monitor_panel,
                        Command::MonitorPanel,
                    );
                    self.check_item(ui, tr("Full screen"), self.full_screen, Command::FullScreen);
                    self.check_item(ui, tr("Always on top"), self.on_top, Command::OnTop);
                    if self.profile.enabled() {
                        self.menu_item(ui, tr("Profiler"), Command::Profiler, true);
                    }
                });
                ui.menu_button(tr("Commands"), |ui| {
                    self.menu_item(ui, tr("Join channel..."), Command::Join, connected);
                    self.menu_item(
                        ui,
                        tr("Part channel"),
                        Command::Part,
                        connected && on_channel,
                    );
                    self.menu_item(
                        ui,
                        tr("Channel central..."),
                        Command::ChannelCentral,
                        connected && on_channel,
                    );
                    ui.separator();
                    self.menu_item(ui, tr("Query user..."), Command::Query, connected);
                    self.menu_item(ui, tr("Send notice..."), Command::Notice, connected);
                    self.menu_item(ui, tr("Whois user..."), Command::Whois, connected);
                    ui.add_enabled_ui(connected, |ui| {
                        ui.menu_button((menu_gutter(), tr("Send CTCP")), |ui| {
                            for (label, request) in [
                                (tr("Version"), "VERSION"),
                                (tr("Ping"), "PING"),
                                (tr("Time"), "TIME"),
                                (tr("Finger"), "FINGER"),
                                (tr("Client info"), "CLIENTINFO"),
                            ] {
                                self.menu_item(ui, label, Command::Ctcp(Some(request)), true);
                            }
                            ui.separator();
                            self.menu_item(ui, tr("Other..."), Command::Ctcp(None), true);
                        });
                        ui.menu_button((menu_gutter(), tr("Set away")), |ui| {
                            self.menu_item(ui, tr("Away..."), Command::Away, true);
                            self.menu_item(ui, tr("Back"), Command::Back, true);
                        });
                    });
                    ui.separator();
                    self.menu_item(ui, tr("Invite user..."), Command::Invite, connected);
                    self.menu_item(ui, tr("Ban user..."), Command::Ban, connected);
                    self.menu_item(ui, tr("Kick user..."), Command::Kick, connected);
                    self.menu_item(ui, tr("Ignore user..."), Command::Ignore, true);
                    self.menu_item(ui, tr("Unignore user..."), Command::Unignore, true);
                    ui.separator();
                    self.menu_item(ui, tr("Names"), Command::Names, connected && on_channel);
                    self.menu_item(ui, tr("Who"), Command::Who, connected && on_channel);
                    self.menu_item(ui, tr("Hop"), Command::Hop, connected && on_channel);
                    self.menu_item(ui, tr("Clear window"), Command::Clear, true);
                    ui.separator();
                    self.menu_item(ui, tr("Change nick..."), Command::ChangeNick, connected);
                    self.menu_item(ui, tr("Quit IRC..."), Command::QuitIrc, connected);
                });
                ui.menu_button(tr("Tools"), |ui| {
                    self.menu_item(ui, tr("Scripts editor..."), Command::Scripts, true);
                    self.menu_item(ui, tr("Dialogs..."), Command::Dialogs, true);
                    ui.separator();
                    self.menu_item(ui, tr("Send file / chat..."), Command::Dcc, true);
                    self.menu_item(ui, tr("Transfers..."), Command::Transfers, true);
                    ui.separator();
                    self.menu_item(ui, tr("Search all windows..."), Command::SearchAll, true);
                    self.menu_item(ui, tr("Log files..."), Command::Log, true);
                    self.check_item(
                        ui,
                        tr("Logging"),
                        self.settings.logging,
                        Command::ToggleLogging,
                    );
                    self.menu_item(ui, tr("Clear history..."), Command::ClearHistory, true);
                    ui.separator();
                    self.menu_item(ui, tr("Sounds..."), Command::Options(6), true);
                    self.menu_item(ui, tr("Options..."), Command::Options(0), true);
                });
                ui.menu_button(tr("Window"), |ui| {
                    self.menu_item(ui, tr("Next window"), Command::NextWindow, true);
                    self.menu_item(ui, tr("Previous window"), Command::PreviousWindow, true);
                    self.menu_item(ui, tr("Close window"), Command::CloseWindow, true);
                    ui.separator();
                    let titles: Vec<(usize, String, Kind)> = self
                        .windows
                        .iter()
                        .enumerate()
                        .map(|(index, window)| (index, self.switch_label(index), window.kind))
                        .collect();
                    for (index, title, _) in titles {
                        // Alt+1 to Alt+9 switch to the first nine windows.
                        let shortcut = (index < 9).then(|| format!("Alt+{}", index + 1));
                        if menu_row(ui, &title, index == self.active, shortcut, true).clicked() {
                            self.activate(index);
                            ui.close();
                        }
                    }
                });
                ui.menu_button(tr("Help"), |ui| {
                    self.menu_item(ui, tr("Command list"), Command::Help, true);
                    ui.separator();
                    self.menu_item(ui, tr("About OpenIRC"), Command::About, true);
                });
            });
        });
    }

    pub(super) fn toolbar(&mut self, ui: &mut egui::Ui) {
        let connected = self.active_connected();
        let on_channel = self
            .windows
            .get(self.active)
            .is_some_and(|window| window.kind == Kind::Channel);
        let frame = egui::Frame::new()
            .fill(color_face())
            .inner_margin(egui::Margin::symmetric(4, 2));
        let band = egui::Panel::top("toolbar").frame(frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                let mut run = None;
                let face = color_face();
                icons::gripper(ui, face);
                // Groups: connection; setup; channels; tools; files; looks; help.
                let groups: [&[ToolButton]; 7] = [
                    &[
                        (
                            Icon::Connect,
                            tr("Connect"),
                            tr("Connect to a server (Alt+E)"),
                            Command::Connect,
                            true,
                        ),
                        (
                            Icon::Disconnect,
                            "",
                            tr("Disconnect from this server"),
                            Command::Disconnect,
                            connected,
                        ),
                    ],
                    &[
                        (
                            Icon::Options,
                            "",
                            tr("Options (Alt+O)"),
                            Command::Options(0),
                            true,
                        ),
                        (
                            Icon::Favorites,
                            "",
                            tr("Favorites (Alt+J)"),
                            Command::Favorites,
                            true,
                        ),
                        (
                            Icon::Address,
                            "",
                            tr("Address book (Alt+B)"),
                            Command::AddressBook,
                            true,
                        ),
                        (
                            Icon::Notify,
                            "",
                            tr("Notify list"),
                            Command::Options(3),
                            true,
                        ),
                    ],
                    &[
                        (
                            Icon::Join,
                            "",
                            tr("Join a channel"),
                            Command::Join,
                            connected,
                        ),
                        (
                            Icon::Part,
                            "",
                            tr("Leave this channel"),
                            Command::Part,
                            connected && on_channel,
                        ),
                        (
                            Icon::ChannelList,
                            "",
                            tr("Channels list (Alt+L)"),
                            Command::ChannelList,
                            connected,
                        ),
                        (
                            Icon::Channel,
                            "",
                            tr("Channel central: topic, modes, bans"),
                            Command::ChannelCentral,
                            connected && on_channel,
                        ),
                    ],
                    &[
                        (
                            Icon::Scripts,
                            "",
                            tr("Scripts editor (Alt+R)"),
                            Command::Scripts,
                            true,
                        ),
                        (Icon::Logs, "", tr("Log files"), Command::Log, true),
                        (Icon::Urls, "", tr("URL list (Alt+U)"), Command::Urls, true),
                    ],
                    &[
                        (
                            Icon::Dcc,
                            "",
                            tr("Send a file or start a DCC chat (Alt+S)"),
                            Command::Dcc,
                            true,
                        ),
                        (
                            Icon::Transfers,
                            "",
                            tr("Transfers"),
                            Command::Transfers,
                            true,
                        ),
                    ],
                    &[(
                        Icon::Colors,
                        "",
                        tr("Colors (Alt+K)"),
                        Command::Options(5),
                        true,
                    )],
                    &[(Icon::Help, "", tr("About OpenIRC"), Command::About, true)],
                ];
                let hidden = &self.settings.toolbar_hidden;
                let shown = |icon: &Icon| !hidden.iter().any(|name| *name == format!("{icon:?}"));
                let mut first = true;
                for group in groups.iter() {
                    // A group whose buttons are all hidden leaves no separator.
                    if !group.iter().any(|button| shown(&button.0)) {
                        continue;
                    }
                    if !first {
                        icons::etched_separator(ui, face);
                    }
                    first = false;
                    for (icon, caption, tip, command, enabled) in group.iter() {
                        if !shown(icon) {
                            continue;
                        }
                        if icons::tool_button(ui, *icon, caption, tip, *enabled).clicked() {
                            run = Some(*command);
                        }
                    }
                }
                if let Some(command) = run {
                    self.run(ui.ctx(), command);
                }
            });
        });
        // The band is raised: a light line along its top, a dark one below.
        let rect = band.response.rect;
        let painter = ui.painter_at(rect);
        let face = color_face();
        painter.hline(
            rect.x_range(),
            rect.top() + 0.5,
            egui::Stroke::new(1.0, super::mix(face, egui::Color32::WHITE, 0.7)),
        );
        painter.hline(
            rect.x_range(),
            rect.bottom() - 0.5,
            egui::Stroke::new(1.0, super::mix(face, egui::Color32::BLACK, 0.35)),
        );
    }

    pub(super) fn switchbar(&mut self, ui: &mut egui::Ui) {
        let frame = egui::Frame::new()
            .fill(color_face())
            .inner_margin(egui::Margin::symmetric(4, 3));
        let mut close = None;
        let mut toggle = None;
        let mut popup_run: Option<(String, String, String)> = None;
        egui::Panel::top("switchbar").frame(frame).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(3.0, 3.0);
                for index in 0..self.windows.len() {
                    let response = self.window_tab(ui, index);
                    if response.clicked() {
                        self.activate(index);
                    }
                    if response.middle_clicked() {
                        close = Some(index);
                    }
                    let mono = self.window_mono(index);
                    let popups = self.window_popups(index);
                    response.context_menu(|ui| {
                        for (label, body, nick, chan) in &popups {
                            if menu_row(ui, label, false, None, true).clicked() {
                                popup_run = Some((body.clone(), nick.clone(), chan.clone()));
                                ui.close();
                            }
                        }
                        if !popups.is_empty() {
                            ui.separator();
                        }
                        if menu_row(ui, tr("Fixed-width font"), mono, None, true).clicked() {
                            toggle = Some(index);
                            ui.close();
                        }
                        if menu_row(ui, tr("Close"), false, None, true).clicked() {
                            close = Some(index);
                            ui.close();
                        }
                    });
                }
            });
        });
        if let Some(index) = toggle {
            self.toggle_mono(index);
        }
        if let Some((body, nick, chan)) = popup_run {
            self.run_with_fields(&body, &nick, &chan, "");
        }
        if let Some(index) = close {
            self.close_window(index);
        }
    }

    /// One switchbar tab: kind icon, label colored by activity.
    fn window_tab(&self, ui: &mut egui::Ui, index: usize) -> egui::Response {
        let window = &self.windows[index];
        let selected = index == self.active;
        let label = self.switch_label(index);
        let font = egui::TextStyle::Button.resolve(ui.style());
        let color = if selected {
            text_on(color_paper(), super::color_ink())
        } else {
            activity_color(window.activity)
        };
        let galley = ui.painter().layout_no_wrap(label, font, color);
        let size = egui::vec2(galley.size().x + 32.0, galley.size().y + 8.0);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        let visuals = ui.visuals();
        let (fill, stroke) = if selected {
            (color_paper(), egui::Stroke::new(1.0, color_hilite()))
        } else if response.hovered() {
            (
                visuals.widgets.hovered.weak_bg_fill,
                visuals.widgets.inactive.bg_stroke,
            )
        } else {
            (
                visuals.widgets.inactive.weak_bg_fill,
                visuals.widgets.inactive.bg_stroke,
            )
        };
        ui.painter().rect(
            rect,
            egui::CornerRadius::same(3),
            fill,
            stroke,
            egui::StrokeKind::Inside,
        );
        if selected {
            let bar = egui::Rect::from_min_max(
                egui::pos2(rect.left() + 1.0, rect.bottom() - 3.0),
                egui::pos2(rect.right() - 1.0, rect.bottom() - 1.0),
            );
            ui.painter().rect_filled(bar, 0.0, color_hilite());
        }
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(rect.left() + 13.0, rect.center().y),
            egui::Vec2::splat(14.0),
        );
        icons::paint(ui.painter(), icon_rect, kind_icon(window.kind), color);
        ui.painter().galley(
            egui::pos2(rect.left() + 24.0, rect.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
        response.on_hover_text(match window.kind {
            Kind::Channel => format!(
                "{} — {} users. Middle-click or right-click to close.",
                window.title,
                window.nicks.len()
            ),
            _ => trf(
                "{0} — middle-click or right-click to close.",
                &[&window.title],
            ),
        })
    }

    pub(super) fn tree(&mut self, ui: &mut egui::Ui) {
        let mut close = None;
        let mut toggle = None;
        let mut popup_run: Option<(String, String, String)> = None;
        egui::Panel::left("tree")
            .resizable(true)
            .default_size(170.0)
            .min_size(120.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("tree-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        let mut groups: Vec<(Option<u64>, String)> = Vec::new();
                        if self.sessions.is_empty() {
                            groups.push((None, tr("Not connected").into()));
                        }
                        for session in &self.sessions {
                            let name = session
                                .network
                                .clone()
                                .unwrap_or_else(|| session.host.clone());
                            groups.push((Some(session.id), name));
                        }
                        for (server, name) in groups {
                            let connected = server.is_some_and(|id| self.session_connected(id));
                            ui.horizontal(|ui| {
                                let dot = if connected {
                                    egui::Color32::from_rgb(40, 170, 80)
                                } else {
                                    egui::Color32::from_gray(150)
                                };
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(10.0, 14.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().circle_filled(rect.center(), 3.5, dot);
                                ui.label(egui::RichText::new(name).strong());
                            });
                            let items: Vec<usize> = (0..self.windows.len())
                                .filter(|&index| self.windows[index].server == server)
                                .collect();
                            for index in items {
                                let response = self.tree_row(ui, index);
                                if response.clicked() {
                                    self.activate(index);
                                }
                                if response.middle_clicked() {
                                    close = Some(index);
                                }
                                let mono = self.window_mono(index);
                                let popups = self.window_popups(index);
                                response.context_menu(|ui| {
                                    for (label, body, nick, chan) in &popups {
                                        if menu_row(ui, label, false, None, true).clicked() {
                                            popup_run =
                                                Some((body.clone(), nick.clone(), chan.clone()));
                                            ui.close();
                                        }
                                    }
                                    if !popups.is_empty() {
                                        ui.separator();
                                    }
                                    if menu_row(ui, tr("Fixed-width font"), mono, None, true)
                                        .clicked()
                                    {
                                        toggle = Some(index);
                                        ui.close();
                                    }
                                    if menu_row(ui, tr("Close"), false, None, true).clicked() {
                                        close = Some(index);
                                        ui.close();
                                    }
                                });
                            }
                            ui.add_space(6.0);
                        }
                    });
            });
        if let Some(index) = toggle {
            self.toggle_mono(index);
        }
        if let Some((body, nick, chan)) = popup_run {
            self.run_with_fields(&body, &nick, &chan, "");
        }
        if let Some(index) = close {
            self.close_window(index);
        }
    }

    fn tree_row(&self, ui: &mut egui::Ui, index: usize) -> egui::Response {
        let window = &self.windows[index];
        let selected = index == self.active;
        let indent = if window.kind == Kind::Status {
            6.0
        } else {
            18.0
        };
        let width = ui.available_width();
        let font = egui::TextStyle::Body.resolve(ui.style());
        let color = if selected {
            text_on(color_hilite(), egui::Color32::WHITE)
        } else {
            activity_color(window.activity)
        };
        // The user count takes the right end; the name gets the rest and
        // ends in `…` when it does not fit.
        let count_text = (window.kind == Kind::Channel && !window.nicks.is_empty())
            .then(|| window.nicks.len().to_string());
        let count_width = count_text
            .as_ref()
            .map(|text| {
                ui.painter()
                    .layout_no_wrap(
                        text.clone(),
                        egui::TextStyle::Small.resolve(ui.style()),
                        color,
                    )
                    .size()
                    .x
                    + 10.0
            })
            .unwrap_or(4.0);
        let name_width = width - (indent + 6.0 + 6.5 + 5.0) - count_width;
        let shown = if self.settings.tree_hide_hash && window.kind == Kind::Channel {
            window.title.trim_start_matches('#')
        } else {
            &window.title
        };
        let galley = super::fitted_text(ui.painter(), shown, font, color, name_width);
        let height = galley.size().y + 4.0;
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
        if selected {
            ui.painter()
                .rect_filled(rect, egui::CornerRadius::same(2), color_hilite());
        } else if response.hovered() {
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::same(2),
                ui.visuals().widgets.hovered.weak_bg_fill,
            );
        }
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(rect.left() + indent + 6.0, rect.center().y),
            egui::Vec2::splat(13.0),
        );
        icons::paint(ui.painter(), icon_rect, kind_icon(window.kind), color);
        ui.painter().galley(
            egui::pos2(
                icon_rect.right() + 5.0,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            color,
        );
        if window.kind == Kind::Channel && !window.nicks.is_empty() {
            let count = ui.painter().layout_no_wrap(
                window.nicks.len().to_string(),
                egui::TextStyle::Small.resolve(ui.style()),
                if selected {
                    color
                } else {
                    super::muted_on_face()
                },
            );
            ui.painter().galley(
                egui::pos2(
                    rect.right() - count.size().x - 6.0,
                    rect.center().y - count.size().y / 2.0,
                ),
                count,
                color,
            );
        }
        response
    }

    /// Script popups for a window's tab menu: label, body, `$nick`, `$chan`.
    pub(super) fn window_popups(&self, index: usize) -> Vec<(String, String, String, String)> {
        let Some(window) = self.windows.get(index) else {
            return Vec::new();
        };
        let (menu, nick, chan) = match window.kind {
            Kind::Channel => ("channel", String::new(), window.title.clone()),
            Kind::Query => ("query", window.title.clone(), String::new()),
            Kind::Status => ("status", String::new(), String::new()),
            Kind::List | Kind::Dcc => return Vec::new(),
        };
        self.popups_for(menu)
            .into_iter()
            .map(|popup| (popup.label, popup.body, nick.clone(), chan.clone()))
            .collect()
    }

    pub(super) fn status_bar(&mut self, ui: &mut egui::Ui) {
        let session = self
            .active_server()
            .and_then(|id| self.sessions.iter().find(|session| session.id == id));
        let (state, state_color, detail) = match session {
            Some(session) if session.link.is_some() && session.connected_at.is_some() => {
                let network = session
                    .network
                    .clone()
                    .unwrap_or_else(|| session.host.clone());
                let mut detail = trf("{0} on {1}", &[&session.nick, &network]);
                if !session.umodes.is_empty() && session.umodes != "+" {
                    detail.push_str(&format!(" {}", session.umodes));
                }
                if let Some(reason) = &session.away {
                    detail.push_str(&trf("  ·  away: {0}", &[reason]));
                }
                (
                    tr("Connected"),
                    egui::Color32::from_rgb(40, 170, 80),
                    detail,
                )
            }
            Some(session) if session.link.is_some() => (
                tr("Connecting…"),
                egui::Color32::from_rgb(220, 160, 30),
                session.host.clone(),
            ),
            Some(session) if session.reconnect_at.is_some() => (
                tr("Reconnecting…"),
                egui::Color32::from_rgb(220, 160, 30),
                session.host.clone(),
            ),
            Some(session) => (
                tr("Disconnected"),
                egui::Color32::from_gray(150),
                session.host.clone(),
            ),
            None => (
                tr("Not connected"),
                egui::Color32::from_gray(150),
                self.server_label.clone(),
            ),
        };
        let online = session
            .and_then(|session| session.connected_at)
            .map(|since| {
                let secs = since.elapsed().as_secs();
                format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
            });
        let window = self.windows.get(self.active);
        let names = window
            .filter(|window| window.kind == Kind::Channel)
            .map(|window| {
                let ops = window
                    .nicks
                    .iter()
                    .filter(|nick| nick.starts_with(['@', '~', '&', '%']))
                    .count();
                format!(
                    "{}, {}",
                    super::plural(window.nicks.len(), "{0} user", "{0} users"),
                    super::plural(ops, "{0} op", "{0} ops")
                )
            });
        let queued = session.map_or(0, |session| session.queued);
        let frame = egui::Frame::new()
            .fill(color_face())
            .inner_margin(egui::Margin::symmetric(8, 3));
        egui::Panel::bottom("status").frame(frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(10.0, 14.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 4.0, state_color);
                ui.label(egui::RichText::new(state).strong());
                ui.separator();
                ui.label(detail);
                if let Some(window) = window {
                    ui.separator();
                    // A channel shows its modes, without the key.
                    let modes = window.modes.split_whitespace().next().unwrap_or("");
                    if window.kind == Kind::Channel && !modes.is_empty() && modes != "+" {
                        ui.label(format!("{} {modes}", window.title));
                    } else {
                        ui.label(&window.title);
                    }
                }
                if let Some(names) = names {
                    ui.separator();
                    ui.label(names);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(online) = online {
                        ui.label(trf("online {0}", &[&online]));
                    }
                    if queued > 0 {
                        ui.separator();
                        ui.label(trf("{0} lines queued", &[&queued]));
                    }
                    if self.sessions.len() > 1 {
                        ui.separator();
                        ui.label(trf("{0} servers", &[&self.sessions.len()]));
                    }
                });
            });
        });
        if session.is_some_and(|session| session.connected_at.is_some()) {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs(1));
        }
    }
}

pub(super) fn kind_icon(kind: Kind) -> Icon {
    match kind {
        Kind::Status => Icon::Status,
        Kind::Channel => Icon::Channel,
        Kind::Query => Icon::Query,
        Kind::List => Icon::List,
        Kind::Dcc => Icon::Chat,
    }
}

/// Switchbar and tree text color for a window's unread activity.
pub(super) fn activity_color(activity: Activity) -> egui::Color32 {
    let face = color_face();
    let base = text_on(face, super::color_ink());
    match activity {
        Activity::None => base,
        Activity::Event => super::readable(egui::Color32::from_rgb(0, 128, 140), face, base),
        Activity::Message => super::readable(super::color_attention(), face, base),
        Activity::Highlight => super::readable(egui::Color32::from_rgb(210, 40, 160), face, base),
    }
}

/// Width of the column at the left of every menu row, where a check mark
/// goes. Every row has it, ticked or not, so all labels line up.
const MENU_GUTTER: f32 = 14.0;

/// The empty check-mark column, for rows and submenu headers.
pub(super) fn menu_gutter() -> egui::Atom<'static> {
    egui::Atom::custom(egui::Id::new("menu-gutter"), egui::vec2(MENU_GUTTER, 0.0))
}

/// One menu row: the check-mark column (ticked when `checked`), the label,
/// and shortcut text on the right.
pub(super) fn menu_row(
    ui: &mut egui::Ui,
    label: &str,
    checked: bool,
    shortcut: Option<String>,
    enabled: bool,
) -> egui::Response {
    // Menus grow to fit a long row instead of wrapping it.
    let mut button =
        egui::Button::new((menu_gutter(), label)).wrap_mode(egui::TextWrapMode::Extend);
    if let Some(text) = shortcut {
        button = button.shortcut_text(text);
    }
    let response = ui.add_enabled(enabled, button);
    if checked {
        let x = response.rect.left() + ui.spacing().button_padding.x + MENU_GUTTER / 2.0;
        let color = ui.style().interact(&response).text_color();
        ui.painter().text(
            egui::pos2(x, response.rect.center().y),
            egui::Align2::CENTER_CENTER,
            "✓",
            egui::TextStyle::Button.resolve(ui.style()),
            color,
        );
    }
    response
}

/// A bold heading in a menu, such as a nick, lined up with the rows.
pub(super) fn menu_title(ui: &mut egui::Ui, text: &str) {
    ui.add(
        // An invisible frame keeps the same padding as the rows below.
        egui::Button::new((menu_gutter(), egui::RichText::new(text).strong()))
            .fill(egui::Color32::TRANSPARENT)
            .stroke(egui::Stroke::NONE)
            .sense(egui::Sense::hover()),
    );
}
