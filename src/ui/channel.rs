//! Channel central (topic, modes, and the ban, exception, and invite lists
//! of one channel) and the channels list from `/LIST`.

use crate::i18n::{tr, tr_key, trf};
use eframe::egui;

use super::{
    Kind, ListEntry, OpenIrcApp, color_ink, color_paper, field, muted_color, paper, section,
};

/// Simple on/off channel flags shown as checkboxes.
const FLAGS: [(char, &str); 6] = [
    ('t', tr_key("Only operators change the topic")),
    ('n', tr_key("No messages from outside the channel")),
    ('i', tr_key("Invite only")),
    ('m', tr_key("Moderated: only voiced users talk")),
    ('s', tr_key("Secret: hidden from the channels list")),
    ('p', tr_key("Private")),
];

/// Lists a channel keeps, with the mode letter and reply numerics.
const LISTS: [(char, &str); 3] = [
    ('b', tr_key("Bans")),
    ('e', tr_key("Exceptions")),
    ('I', tr_key("Invites")),
];

pub(super) struct Central {
    channel: String,
    server: Option<u64>,
    topic: String,
    flags: Vec<(char, bool)>,
    key: (bool, String),
    limit: (bool, String),
    tab: char,
    new_mask: String,
    /// Mode string the form was filled from, to detect what the user changed.
    pub(super) loaded: String,
    loaded_topic: String,
}

/// One row of the channels list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ListedChannel {
    pub name: String,
    pub users: u32,
    /// The topic without codes, for searching.
    pub topic: String,
    /// The topic with its colors and styles, for showing.
    pub shown_topic: String,
    /// Channel modes when the server puts them before the topic (`[+nt]`).
    pub modes: String,
}

pub(super) struct ChannelList {
    pub open: bool,
    pub listing: bool,
    /// The server the list belongs to; replies from others are ignored.
    pub server: Option<u64>,
    /// Seconds between joins when several channels are joined at once.
    join_delay: u32,
    note: String,
    /// The channels listed. Changed only through `clear` and `push`, which
    /// count each change in `generation`.
    entries: Vec<ListedChannel>,
    /// Goes up with every change to `entries`, or to the server they come
    /// from, so the shown rows are made again after any of them.
    generation: u64,
    filter: String,
    in_topics: bool,
    min_users: u32,
    /// Most users to show; 0 has no limit.
    max_users: u32,
    /// Channels to join, separated by commas or spaces.
    join_many: String,
    sort: Sort,
    descending: bool,
    /// The shown rows (indexes into `entries`) and what they were made from,
    /// so a big list is filtered and sorted only when something changed.
    shown: Option<(ShownKey, Vec<usize>)>,
}

#[derive(Clone, PartialEq, Eq)]
struct ShownKey {
    generation: u64,
    filter: String,
    in_topics: bool,
    min_users: u32,
    max_users: u32,
    sort: Sort,
    descending: bool,
}

impl Default for ChannelList {
    /// Biggest channels first.
    fn default() -> Self {
        Self {
            open: false,
            listing: false,
            server: None,
            join_delay: 0,
            note: String::new(),
            entries: Vec::new(),
            generation: 0,
            filter: String::new(),
            in_topics: false,
            min_users: 0,
            max_users: 0,
            join_many: String::new(),
            sort: Sort::Users,
            descending: true,
            shown: None,
        }
    }
}

impl ChannelList {
    pub fn entries(&self) -> &[ListedChannel] {
        &self.entries
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.generation += 1;
    }

    pub fn push(&mut self, entry: ListedChannel) {
        self.entries.push(entry);
        self.generation += 1;
    }

    /// The list now belongs to `server`.
    fn set_server(&mut self, server: Option<u64>) {
        if self.server != server {
            self.server = server;
            self.generation += 1;
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Sort {
    Name,
    #[default]
    Users,
}

/// Parsed form of a mode string such as `+ntkl secret 20`.
pub(super) fn parse_modes(modes: &str) -> (Vec<char>, Option<String>, Option<String>) {
    let mut parts = modes.split_whitespace();
    let letters: Vec<char> = parts
        .next()
        .unwrap_or("")
        .chars()
        .filter(|ch| *ch != '+')
        .collect();
    let args: Vec<&str> = parts.collect();
    let mut key = None;
    let mut limit = None;
    let mut next = args.iter();
    for letter in &letters {
        match letter {
            'k' => key = next.next().map(|arg| arg.to_string()),
            'l' => limit = next.next().map(|arg| arg.to_string()),
            _ => {}
        }
    }
    (letters, key, limit)
}

/// The MODE arguments that turn `before` into the form's choices, or
/// `None` when nothing changed.
pub(super) fn mode_change(
    before: &str,
    flags: &[(char, bool)],
    key: &(bool, String),
    limit: &(bool, String),
) -> Option<String> {
    let (letters, old_key, old_limit) = parse_modes(before);
    let mut plus = String::new();
    let mut minus = String::new();
    let mut plus_args = Vec::new();
    let mut minus_args = Vec::new();
    for (letter, on) in flags {
        let was = letters.contains(letter);
        if *on && !was {
            plus.push(*letter);
        } else if !*on && was {
            minus.push(*letter);
        }
    }
    let new_key = key.1.trim();
    match (&old_key, key.0 && !new_key.is_empty()) {
        (None, true) => {
            plus.push('k');
            plus_args.push(new_key.to_string());
        }
        (Some(old), true) if old != new_key => {
            minus.push('k');
            minus_args.push(old.clone());
            plus.push('k');
            plus_args.push(new_key.to_string());
        }
        (Some(old), false) => {
            minus.push('k');
            minus_args.push(old.clone());
        }
        _ => {}
    }
    let new_limit = limit
        .1
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|limit| *limit > 0);
    match (&old_limit, limit.0.then_some(new_limit).flatten()) {
        (None, Some(value)) => {
            plus.push('l');
            plus_args.push(value.to_string());
        }
        (Some(old), Some(value)) if *old != value.to_string() => {
            plus.push('l');
            plus_args.push(value.to_string());
        }
        (Some(_), None) => minus.push('l'),
        _ => {}
    }
    if plus.is_empty() && minus.is_empty() {
        return None;
    }
    // Removals first, so changing the key removes the old one before the
    // new one is set.
    let mut out = String::new();
    if !minus.is_empty() {
        out.push('-');
        out.push_str(&minus);
    }
    if !plus.is_empty() {
        out.push('+');
        out.push_str(&plus);
    }
    for arg in minus_args.into_iter().chain(plus_args) {
        out.push(' ');
        out.push_str(&arg);
    }
    Some(out)
}

/// One `322` reply: `#channel users topic`.
pub(super) fn parse_listed(text: &str) -> Option<ListedChannel> {
    let (name, rest) = text.split_once(' ')?;
    let (users, topic) = rest.split_once(' ').unwrap_or((rest, ""));
    let mut shown_topic = topic.trim().to_string();
    let mut modes = String::new();
    // Many servers start the topic with the channel's modes: `[+ntz] text`.
    if let Some(after) = shown_topic.strip_prefix("[+")
        && let Some((flags, text)) = after.split_once(']')
        && !flags.contains(' ')
    {
        modes = format!("+{flags}");
        shown_topic = text.trim_start().to_string();
    }
    Some(ListedChannel {
        name: name.to_string(),
        users: users.trim().parse().ok()?,
        topic: crate::colors::strip(&shown_topic),
        shown_topic,
        modes,
    })
}

/// One list reply (`367`, `348`, `346`): `#channel mask [setter [time]]`.
pub(super) fn parse_list_entry(text: &str) -> Option<(String, ListEntry)> {
    let mut parts = text.split_whitespace();
    let channel = parts.next()?.to_string();
    let mask = parts.next()?.to_string();
    let by = parts.next().unwrap_or("").to_string();
    Some((channel, ListEntry { mask, by }))
}

impl OpenIrcApp {
    pub(super) fn open_channel_central(&mut self) {
        let Some(window) = self.windows.get(self.active) else {
            return;
        };
        if window.kind != Kind::Channel {
            return;
        }
        let (letters, key, limit) = parse_modes(&window.modes);
        let central = Central {
            channel: window.title.clone(),
            server: window.server,
            topic: window.topic.clone().unwrap_or_default(),
            flags: FLAGS
                .iter()
                .map(|(letter, _)| (*letter, letters.contains(letter)))
                .collect(),
            key: (key.is_some(), key.unwrap_or_default()),
            limit: (limit.is_some(), limit.unwrap_or_default()),
            tab: 'b',
            new_mask: String::new(),
            loaded: window.modes.clone(),
            loaded_topic: window.topic.clone().unwrap_or_default(),
        };
        let channel = central.channel.clone();
        self.central = Some(central);
        self.send_user(format!("/raw MODE {channel}"));
        self.request_list(&channel, 'b');
    }

    fn request_list(&mut self, channel: &str, letter: char) {
        let index = self.find(Kind::Channel, channel);
        if let Some(index) = index {
            self.windows[index].mode_lists.insert(letter, Vec::new());
        }
        self.send_user(format!("/raw MODE {channel} +{letter}"));
    }

    pub(super) fn channel_central(&mut self, ctx: &egui::Context) {
        let Some(mut central) = self.central.take() else {
            return;
        };
        // The form's own server decides which names are the same channel.
        let map = self.casemap_of(central.server);
        let window = self.windows.iter().position(|window| {
            window.kind == Kind::Channel
                && window.server == central.server
                && map.same(&window.title, &central.channel)
        });
        let Some(window) = window else {
            return;
        };
        // When the server's mode reply arrives after the form opened, take
        // its values unless the user already changed something.
        let current_modes = self.windows[window].modes.clone();
        if current_modes != central.loaded {
            let (letters, key, limit) = parse_modes(&current_modes);
            let (old_letters, old_key, old_limit) = parse_modes(&central.loaded);
            let untouched = central
                .flags
                .iter()
                .all(|(letter, on)| *on == old_letters.contains(letter))
                && central.key == (old_key.is_some(), old_key.clone().unwrap_or_default())
                && central.limit == (old_limit.is_some(), old_limit.clone().unwrap_or_default());
            if untouched {
                for (letter, on) in &mut central.flags {
                    *on = letters.contains(letter);
                }
                central.key = (key.is_some(), key.unwrap_or_default());
                central.limit = (limit.is_some(), limit.unwrap_or_default());
            }
            central.loaded = current_modes;
        }
        let mut open = true;
        let mut apply = false;
        let mut close = false;
        let mut remove: Option<(char, String)> = None;
        let mut add: Option<(char, String)> = None;
        let mut refresh: Option<char> = None;
        let lists = self.windows[window].mode_lists.clone();
        egui::Window::new(trf("Channel central — {0}", &[&central.channel]))
            .id(egui::Id::new("channel-central"))
            .id(egui::Id::new("channel-central"))
            .collapsible(false)
            .resizable(true)
            .default_size([480.0, 460.0])
            .default_pos(super::dialog_pos(ctx, 5))
            .open(&mut open)
            .show(ctx, |ui| {
                section(ui, tr("Topic"));
                ui.add(
                    super::field_multi(&mut central.topic)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(6.0);
                section(ui, tr("Modes"));
                egui::Grid::new("central-flags")
                    .num_columns(2)
                    .spacing([16.0, 2.0])
                    .show(ui, |ui| {
                        for (index, (letter, description)) in FLAGS.iter().enumerate() {
                            let slot = &mut central.flags[index].1;
                            // The words first and the letter after, so every
                            // label starts in the same place.
                            ui.checkbox(slot, format!("{} (+{letter})", tr(description)));
                            if index % 2 == 1 {
                                ui.end_row();
                            }
                        }
                        // Key and limit in the same two columns.
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut central.key.0, tr("Key (+k)"));
                            ui.add_enabled(
                                central.key.0,
                                field(&mut central.key.1)
                                    .password(true)
                                    .desired_width(140.0),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut central.limit.0, tr("User limit (+l)"));
                            ui.add_enabled(
                                central.limit.0,
                                field(&mut central.limit.1).desired_width(60.0),
                            );
                        });
                        ui.end_row();
                    });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    for (letter, label) in LISTS {
                        let count = lists.get(&letter).map_or(0, Vec::len);
                        if ui
                            .selectable_label(
                                central.tab == letter,
                                format!("{} ({count})", tr(label)),
                            )
                            .clicked()
                        {
                            central.tab = letter;
                            if !lists.contains_key(&letter) {
                                refresh = Some(letter);
                            }
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button(tr("Refresh")).clicked() {
                            refresh = Some(central.tab);
                        }
                    });
                });
                paper().show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    egui::ScrollArea::vertical()
                        .id_salt("central-list")
                        .max_height(150.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            let entries = lists.get(&central.tab).cloned().unwrap_or_default();
                            if entries.is_empty() {
                                ui.label(
                                    egui::RichText::new(tr("The list is empty."))
                                        .color(muted_color()),
                                );
                            }
                            for entry in entries {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(&entry.mask)
                                            .monospace()
                                            .color(color_ink()),
                                    );
                                    if !entry.by.is_empty() {
                                        ui.label(
                                            egui::RichText::new(trf(
                                                "by {0}",
                                                &[&super::bare_setter(&entry.by)],
                                            ))
                                            .small()
                                            .color(muted_color()),
                                        );
                                    }
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui.small_button(tr("Remove")).clicked() {
                                                remove = Some((central.tab, entry.mask.clone()));
                                            }
                                        },
                                    );
                                });
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.add(
                        field(&mut central.new_mask)
                            .hint_text(tr("nick!user@host"))
                            .desired_width(220.0),
                    );
                    if ui.button(tr("Add")).clicked() && !central.new_mask.trim().is_empty() {
                        add = Some((central.tab, central.new_mask.trim().to_string()));
                        central.new_mask.clear();
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(tr("OK")).clicked() {
                        apply = true;
                        close = true;
                    }
                    if ui.button(tr("Apply")).clicked() {
                        apply = true;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
        self.central_actions(&mut central, window, refresh, remove, add, apply);
        if open && !close {
            self.central = Some(central);
        }
    }

    /// Send what the Channel Central form asked for. Everything goes to the
    /// form's own server, even when another server's window is in front now.
    pub(super) fn central_actions(
        &mut self,
        central: &mut Central,
        window: usize,
        refresh: Option<char>,
        remove: Option<(char, String)>,
        add: Option<(char, String)>,
        apply: bool,
    ) {
        let channel = central.channel.clone();
        // Lookups and names follow the form's server as well as the lines.
        self.on_server(central.server, |app| {
            if let Some(letter) = refresh {
                app.request_list(&channel, letter);
            }
            if let Some((letter, mask)) = remove {
                app.send_user(format!("/raw MODE {channel} -{letter} {mask}"));
                if let Some(list) = app.windows[window].mode_lists.get_mut(&letter) {
                    list.retain(|entry| entry.mask != mask);
                }
            }
            if let Some((letter, mask)) = add {
                app.send_user(format!("/raw MODE {channel} +{letter} {mask}"));
                let me = app.me().to_string();
                app.windows[window]
                    .mode_lists
                    .entry(letter)
                    .or_default()
                    .push(ListEntry { mask, by: me });
            }
            if apply {
                if central.topic.trim() != central.loaded_topic.trim() {
                    app.send_user(format!("/topic {channel} {}", central.topic.trim()));
                    central.loaded_topic = central.topic.clone();
                }
                if let Some(change) = mode_change(
                    &central.loaded,
                    &central.flags,
                    &central.key,
                    &central.limit,
                ) {
                    app.send_user(format!("/raw MODE {channel} {change}"));
                }
            }
        });
    }

    pub(super) fn open_channel_list(&mut self) {
        let server = self.active_server();
        self.show_channel_list(server, true);
    }

    /// Open the channels list for `server`; `fetch` asks the server for the
    /// list at once (opening after a connect leaves that to Get list).
    pub(super) fn show_channel_list(&mut self, server: Option<u64>, fetch: bool) {
        if self.channel_list.server != server {
            self.channel_list.clear();
            self.channel_list.listing = false;
            self.channel_list.note.clear();
        }
        self.channel_list.set_server(server);
        self.channel_list.open = true;
        if fetch && self.channel_list.entries.is_empty() && !self.channel_list.listing {
            self.start_listing();
        }
    }

    fn start_listing(&mut self) {
        self.channel_list.clear();
        self.channel_list.listing = true;
        match self.channel_list.server {
            Some(id) => self.send_to(id, "/raw LIST".into()),
            None => self.send_user("/raw LIST".into()),
        }
    }

    /// The network name of the list's server, for the title and file name.
    fn list_network(&self) -> String {
        self.sessions
            .iter()
            .find(|session| Some(session.id) == self.channel_list.server)
            .map(|session| {
                session
                    .network
                    .clone()
                    .unwrap_or_else(|| session.host.clone())
            })
            .unwrap_or_default()
    }

    /// Write the list (name, users, topic) to a file in the settings folder.
    fn save_channel_list(&mut self) {
        let network = self.list_network();
        let safe: String = network
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' {
                    ch
                } else {
                    '_'
                }
            })
            .collect();
        let path = crate::settings::config_dir().join(format!("channels-{safe}.txt"));
        let text: String = self
            .channel_list
            .entries
            .iter()
            .map(|entry| {
                format!(
                    "{}\t{}\t{}\t{}\n",
                    entry.name, entry.users, entry.modes, entry.topic
                )
            })
            .collect();
        self.channel_list.note = match std::fs::write(&path, text) {
            Ok(()) => trf("Saved to {0}.", &[&path.display()]),
            Err(err) => trf("Could not save: {0}", &[&err]),
        };
    }

    pub(super) fn channel_list_window(&mut self, ctx: &egui::Context) {
        if !self.channel_list.open {
            return;
        }
        let mut open = true;
        let mut join = None;
        let mut relist = false;
        let mut save = false;
        let mut close = false;
        let network = self.list_network();
        let title = if network.is_empty() {
            "Channels".to_string()
        } else {
            trf("Channels — {0}", &[&network])
        };
        let mut on_connect = self.settings.channels_on_connect;
        let list = &mut self.channel_list;
        egui::Window::new(title)
            .id(egui::Id::new("channels-list"))
            .collapsible(false)
            .resizable(true)
            .default_size([620.0, 460.0])
            .default_pos(super::dialog_pos(ctx, 4))
            .open(&mut open)
            .show(ctx, |ui| {
                // A title band and a short note, like the dialog people know.
                egui::Frame::new()
                    .fill(super::color_hilite())
                    .inner_margin(egui::Margin::symmetric(8, 5))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label(
                            egui::RichText::new(if network.is_empty() {
                                "Channels".to_string()
                            } else {
                                trf("Channels — {0}", &[&network])
                            })
                            .size(18.0)
                            .color(super::text_on(super::color_hilite(), egui::Color32::WHITE)),
                        );
                    });
                if list.entries.is_empty() && !list.listing {
                    ui.label(
                        tr("Press Get list to see this server's channels, or type the channels \
                         to join in the box below."),
                    );
                }
                let key = ShownKey {
                    generation: list.generation,
                    filter: list.filter.to_lowercase(),
                    in_topics: list.in_topics,
                    min_users: list.min_users,
                    max_users: list.max_users,
                    sort: list.sort,
                    descending: list.descending,
                };
                if list.shown.as_ref().is_none_or(|(made, _)| *made != key) {
                    let filter = &key.filter;
                    let mut picked: Vec<usize> = (0..list.entries.len())
                        .filter(|&at| {
                            let entry = &list.entries[at];
                            entry.users >= list.min_users
                                && (list.max_users == 0 || entry.users <= list.max_users)
                                && (filter.is_empty()
                                    || entry.name.to_lowercase().contains(filter)
                                    || (list.in_topics
                                        && entry.topic.to_lowercase().contains(filter)))
                        })
                        .collect();
                    match list.sort {
                        Sort::Name => {
                            picked.sort_by_cached_key(|&at| list.entries[at].name.to_lowercase())
                        }
                        Sort::Users => picked.sort_by_key(|&at| list.entries[at].users),
                    }
                    if list.descending {
                        picked.reverse();
                    }
                    list.shown = Some((key, picked));
                }
                let rows: Vec<&ListedChannel> = list
                    .shown
                    .as_ref()
                    .map(|(_, picked)| picked.iter().map(|&at| &list.entries[at]).collect())
                    .unwrap_or_default();
                let users: u64 = rows.iter().map(|entry| u64::from(entry.users)).sum();
                ui.label(
                    egui::RichText::new(trf(
                        if list.listing {
                            tr_key("Showing {0} ({1} users) of {2} channels so far. Double-click a channel to join it.")
                        } else {
                            tr_key("Showing {0} ({1} users) of {2} channels. Double-click a channel to join it.")
                        },
                        &[&rows.len(), &users, &list.entries.len()],
                    ))
                    .color(muted_color()),
                );
                // The list takes the room the controls below leave.
                let list_height = (ui.available_height() - 110.0).max(120.0);
                ui.allocate_ui(egui::vec2(ui.available_width(), list_height), |ui| {
                let name_width = 190.0;
                let users_width = 60.0;
                let modes_width = 96.0;
                ui.horizontal(|ui| {
                    for (label, sort, width) in [
                        (tr("Channel"), Sort::Name, name_width),
                        (tr("Users"), Sort::Users, users_width),
                    ] {
                        let arrow = if list.sort == sort {
                            if list.descending { " ▼" } else { " ▲" }
                        } else {
                            ""
                        };
                        let response = ui.add_sized(
                            [width, 20.0],
                            egui::Button::new(
                                egui::RichText::new(format!("{label}{arrow}")).strong(),
                            )
                            .frame(false),
                        );
                        if response.clicked() {
                            if list.sort == sort {
                                list.descending = !list.descending;
                            } else {
                                list.sort = sort;
                                list.descending = sort == Sort::Users;
                            }
                        }
                    }
                    ui.add_sized(
                        [modes_width, 20.0],
                        egui::Label::new(egui::RichText::new(tr("Modes")).strong()),
                    );
                    section(ui, tr("Topic"));
                });
                let no_words = std::collections::HashSet::new();
                paper().show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    let row_height = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
                    egui::ScrollArea::vertical()
                        .id_salt("channels-rows")
                        .auto_shrink([false, false])
                        .show_rows(ui, row_height, rows.len(), |ui, range| {
                            for (offset, entry) in rows[range.clone()].iter().enumerate() {
                                let index = range.start + offset;
                                let width = ui.available_width();
                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(width, row_height),
                                    egui::Sense::click(),
                                );
                                if index % 2 == 1 {
                                    ui.painter().rect_filled(
                                        rect,
                                        0.0,
                                        super::mix(color_paper(), color_ink(), 0.04),
                                    );
                                }
                                if response.hovered() {
                                    ui.painter().rect_filled(
                                        rect,
                                        0.0,
                                        super::color_hilite().gamma_multiply(0.12),
                                    );
                                }
                                let font = egui::TextStyle::Body.resolve(ui.style());
                                let y = rect.center().y;
                                let name = super::fitted_text(
                                    ui.painter(),
                                    &entry.name,
                                    egui::FontId::new(
                                        font.size,
                                        egui::FontFamily::Name("bold".into()),
                                    ),
                                    color_ink(),
                                    name_width - 8.0,
                                );
                                ui.painter().galley(
                                    egui::pos2(rect.left() + 4.0, y - name.size().y / 2.0),
                                    name,
                                    color_ink(),
                                );
                                ui.painter().text(
                                    egui::pos2(rect.left() + name_width + users_width - 6.0, y),
                                    egui::Align2::RIGHT_CENTER,
                                    entry.users.to_string(),
                                    font.clone(),
                                    color_ink(),
                                );
                                let modes = super::fitted_text(
                                    ui.painter(),
                                    &entry.modes,
                                    font.clone(),
                                    muted_color(),
                                    modes_width - 4.0,
                                );
                                ui.painter().galley(
                                    egui::pos2(
                                        rect.left() + name_width + users_width + 10.0,
                                        y - modes.size().y / 2.0,
                                    ),
                                    modes,
                                    muted_color(),
                                );
                                let topic_left =
                                    rect.left() + name_width + users_width + modes_width + 10.0;
                                let topic_rect = egui::Rect::from_min_max(
                                    egui::pos2(topic_left, rect.top()),
                                    rect.right_bottom(),
                                );
                                // The topic with its colors, on one line.
                                let line = super::topic_line(&entry.shown_topic);
                                let look = super::TextLook {
                                    font: font.size,
                                    mono: false,
                                    stamp: "",
                                };
                                let laid = super::line_galley(ui, &line, look, 4000.0, super::Spell { words: &no_words, skip: &no_words });
                                ui.painter().with_clip_rect(topic_rect).galley(
                                    egui::pos2(topic_left, y - laid.galley.size().y / 2.0),
                                    laid.galley.clone(),
                                    color_ink(),
                                );
                                let response = response.on_hover_text(if entry.topic.is_empty() {
                                    entry.name.clone()
                                } else {
                                    entry.topic.clone()
                                });
                                if response.double_clicked() {
                                    join = Some(entry.name.clone());
                                }
                                response.context_menu(|ui| {
                                    if super::chrome::menu_row(
                                        ui,
                                        &trf("Join {0}", &[&entry.name]),
                                        false,
                                        None,
                                        true,
                                    )
                                    .clicked()
                                    {
                                        join = Some(entry.name.clone());
                                        ui.close();
                                    }
                                });
                            }
                        });
                });
                });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(tr("Filter"));
                    ui.add(
                        field(&mut list.filter)
                            .hint_text(tr("channel name"))
                            .desired_width(160.0),
                    );
                    ui.checkbox(&mut list.in_topics, tr("in topics"));
                    ui.add_space(12.0);
                    ui.label(tr("Users from"));
                    ui.add(
                        egui::DragValue::new(&mut list.min_users)
                            .range(0..=100_000)
                            .speed(1.0),
                    );
                    ui.label(tr("to"));
                    ui.add(
                        egui::DragValue::new(&mut list.max_users)
                            .range(0..=100_000)
                            .speed(1.0),
                    )
                    .on_hover_text(tr("0 has no limit"));
                });
                ui.horizontal(|ui| {
                    ui.label(tr("Join channels"));
                    let response = ui.add(
                        field(&mut list.join_many)
                            .hint_text("#one, #two")
                            .desired_width(240.0),
                    );
                    ui.add_space(12.0);
                    ui.label(tr("Seconds between joins"));
                    ui.add(egui::DragValue::new(&mut list.join_delay).range(0..=60))
                        .on_hover_text(tr("For networks that limit how fast you may join"));
                    let enter = response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    if enter && !list.join_many.trim().is_empty() {
                        let names: Vec<&str> = list
                            .join_many
                            .split([',', ' '])
                            .filter(|name| !name.is_empty())
                            .collect();
                        join = Some(names.join(","));
                        list.join_many.clear();
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button(tr("Join")).clicked() && !list.join_many.trim().is_empty() {
                        let names: Vec<&str> = list
                            .join_many
                            .split([',', ' '])
                            .filter(|name| !name.is_empty())
                            .collect();
                        join = Some(names.join(","));
                        list.join_many.clear();
                    }
                    let label = if list.listing {
                        tr("Listing…")
                    } else if list.entries.is_empty() {
                        tr("Get list")
                    } else {
                        tr("Refresh")
                    };
                    if ui.add_enabled(!list.listing, egui::Button::new(label)).clicked() {
                        relist = true;
                    }
                    if ui
                        .add_enabled(!list.entries.is_empty(), egui::Button::new(tr("Save list")))
                        .on_hover_text(tr("Write the channels to a text file"))
                        .clicked()
                    {
                        save = true;
                    }
                    if ui.button(tr("Close")).clicked() {
                        close = true;
                    }
                    ui.checkbox(&mut on_connect, tr("Show this when connecting"));
                });
                if !list.note.is_empty() {
                    ui.label(egui::RichText::new(&list.note).small().color(muted_color()));
                }
            });
        if save {
            self.save_channel_list();
        }
        if on_connect != self.settings.channels_on_connect {
            self.settings.channels_on_connect = on_connect;
            self.persist();
        }
        if relist {
            self.start_listing();
        }
        if let Some(channels) = join {
            let delay = u64::from(self.channel_list.join_delay);
            let names: Vec<String> = channels.split(',').map(str::to_string).collect();
            for (index, channel) in names.into_iter().enumerate() {
                let command = format!("/join {channel}");
                if delay == 0 || index == 0 {
                    match self.channel_list.server {
                        Some(id) => self.send_to(id, command),
                        None => self.send_user(command),
                    }
                } else {
                    // Later channels wait, then join on the list's server.
                    let server = self.channel_list.server.or_else(|| self.scope());
                    self.timers.push(super::UiTimer {
                        next: std::time::Instant::now()
                            + std::time::Duration::from_secs(delay * index as u64),
                        every: std::time::Duration::from_secs(delay),
                        left: 1,
                        command,
                        server,
                        window: None,
                        origin: None,
                        chat: None,
                        reply: None,
                    });
                }
            }
        }
        if !open || close {
            self.channel_list.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown_names(app: &crate::ui::OpenIrcApp) -> Vec<String> {
        let list = &app.channel_list;
        list.shown
            .as_ref()
            .map(|(_, picked)| {
                picked
                    .iter()
                    .map(|&at| list.entries[at].name.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn a_refreshed_list_is_filtered_and_sorted_again() {
        let ctx = egui::Context::default();
        crate::ui::install_fonts(&ctx);
        let mut app = crate::ui::tests::app();
        app.channel_list.open = true;
        app.channel_list.min_users = 50;
        let fill = |app: &mut crate::ui::OpenIrcApp, rows: &[(&str, u32, &str)]| {
            app.numeric(321, String::new());
            for (name, users, topic) in rows {
                app.numeric(322, format!("{name} {users} :{topic}"));
            }
            app.numeric(323, String::new());
        };
        fill(
            &mut app,
            &[("#a", 100, "old"), ("#m", 3, ""), ("#b", 1, "")],
        );
        crate::ui::tests::frame(&ctx, &mut app, Vec::new());
        assert_eq!(shown_names(&app), vec!["#a".to_string()]);
        // A row that arrives while the list is shown appears at once.
        app.numeric(322, "#late 80 :came later".into());
        crate::ui::tests::frame(&ctx, &mut app, Vec::new());
        assert_eq!(
            shown_names(&app),
            vec!["#a".to_string(), "#late".to_string()]
        );
        // Refreshed between two frames: the same count, the same first and
        // last names, the user counts swapped.
        fill(
            &mut app,
            &[("#a", 1, "new"), ("#m", 3, ""), ("#b", 100, "")],
        );
        crate::ui::tests::frame(&ctx, &mut app, Vec::new());
        assert_eq!(shown_names(&app), vec!["#b".to_string()]);
        // A topic-only change is found by a topic search too.
        app.channel_list.min_users = 0;
        app.channel_list.in_topics = true;
        app.channel_list.filter = "fresh".into();
        crate::ui::tests::frame(&ctx, &mut app, Vec::new());
        assert!(shown_names(&app).is_empty());
        fill(
            &mut app,
            &[("#a", 1, "fresh"), ("#m", 3, ""), ("#b", 100, "")],
        );
        crate::ui::tests::frame(&ctx, &mut app, Vec::new());
        assert_eq!(shown_names(&app), vec!["#a".to_string()]);
    }

    #[test]
    fn mode_change_adds_removes_and_swaps_the_key() {
        let flags = vec![
            ('t', true),
            ('n', false),
            ('i', true),
            ('m', false),
            ('s', false),
            ('p', false),
        ];
        assert_eq!(
            mode_change(
                "+nt",
                &flags,
                &(false, String::new()),
                &(false, String::new())
            )
            .as_deref(),
            Some("-n+i")
        );
        let same = vec![
            ('t', true),
            ('n', true),
            ('i', false),
            ('m', false),
            ('s', false),
            ('p', false),
        ];
        assert_eq!(
            mode_change(
                "+nt",
                &same,
                &(false, String::new()),
                &(false, String::new())
            ),
            None
        );
        assert_eq!(
            mode_change(
                "+knt old",
                &same,
                &(true, "new".into()),
                &(true, "25".into())
            )
            .as_deref(),
            Some("-k+kl old new 25")
        );
        assert_eq!(
            mode_change(
                "+lnt 10",
                &same,
                &(false, String::new()),
                &(false, "10".into())
            )
            .as_deref(),
            Some("-l")
        );
    }

    #[test]
    fn parses_list_replies() {
        assert_eq!(
            parse_listed("#rust 120 \u{2}Rust\u{2} talk"),
            Some(ListedChannel {
                name: "#rust".into(),
                users: 120,
                topic: "Rust talk".into(),
                shown_topic: "\u{2}Rust\u{2} talk".into(),
                modes: String::new(),
            })
        );
        assert_eq!(
            parse_listed("#empty 3"),
            Some(ListedChannel {
                name: "#empty".into(),
                users: 3,
                topic: String::new(),
                shown_topic: String::new(),
                modes: String::new(),
            })
        );
        let moded = parse_listed("#chat 293 [+CMNRSnpt] \u{3}04Welcome\u{3} all").unwrap();
        assert_eq!(moded.modes, "+CMNRSnpt");
        assert_eq!(moded.topic, "Welcome all");
        assert_eq!(moded.shown_topic, "\u{3}04Welcome\u{3} all");
        let (channel, entry) = parse_list_entry("#c *!*@spam.example op!u@h 1700000000").unwrap();
        assert_eq!(channel, "#c");
        assert_eq!(entry.mask, "*!*@spam.example");
        assert_eq!(entry.by, "op!u@h");
    }
}
