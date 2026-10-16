//! Small dialogs that ask for a nick, a channel, or a reason and then run
//! a command: everything in the Commands menu that needs input. They share
//! one look (labels in a column, Enter runs the command) and start filled
//! in from the active window and the nick picked in the nick list.

use crate::i18n::{tr, tr_key, trf};
use eframe::egui;

use super::{Kind, OpenIrcApp, Origin, dialog_pos, field, form_grid, muted_on_face};

/// What a prompt asks for and the command it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PromptKind {
    Join,
    Query,
    Notice,
    Whois,
    /// A CTCP request; `None` asks for the request too.
    Ctcp(Option<&'static str>),
    Away,
    Invite,
    Ban,
    Kick,
    Ignore,
    Unignore,
    Nick,
    Quit,
}

pub(super) struct Prompt {
    kind: PromptKind,
    /// Label, hint, and the text typed.
    fields: Vec<(&'static str, &'static str, String)>,
    note: String,
    /// The server in front when the prompt opened; its command goes there.
    server: Option<u64>,
}

impl PromptKind {
    fn title(self) -> &'static str {
        match self {
            PromptKind::Join => tr("Join channel"),
            PromptKind::Query => tr("Query user"),
            PromptKind::Notice => tr("Send notice"),
            PromptKind::Whois => tr("Whois user"),
            PromptKind::Ctcp(_) => tr("Send CTCP"),
            PromptKind::Away => tr("Set away"),
            PromptKind::Invite => tr("Invite user"),
            PromptKind::Ban => tr("Ban user"),
            PromptKind::Kick => tr("Kick user"),
            PromptKind::Ignore => tr("Ignore user"),
            PromptKind::Unignore => tr("Unignore user"),
            PromptKind::Nick => tr("Change nick"),
            PromptKind::Quit => tr("Quit IRC"),
        }
    }

    fn button(self) -> &'static str {
        match self {
            PromptKind::Join => tr("Join"),
            PromptKind::Query => tr("Open query"),
            PromptKind::Notice => tr("Send"),
            PromptKind::Whois => tr("Whois"),
            PromptKind::Ctcp(_) => tr("Send"),
            PromptKind::Away => tr("Set away"),
            PromptKind::Invite => tr("Invite"),
            PromptKind::Ban => tr("Ban"),
            PromptKind::Kick => tr("Kick"),
            PromptKind::Ignore => tr("Ignore"),
            PromptKind::Unignore => tr("Unignore"),
            PromptKind::Nick => tr("Change"),
            PromptKind::Quit => tr("Quit"),
        }
    }
}

/// The commands a filled-in prompt runs, or what is missing.
pub(super) fn prompt_commands(kind: PromptKind, values: &[String]) -> Result<Vec<String>, String> {
    let value = |index: usize| values.get(index).map(|text| text.trim()).unwrap_or("");
    let need = |index: usize, what: &str| -> Result<String, String> {
        let text = value(index);
        if text.is_empty() {
            Err(trf("Enter {0}.", &[&tr(what)]))
        } else {
            Ok(text.to_string())
        }
    };
    let joined = |parts: &[&str]| -> String {
        parts
            .iter()
            .filter(|part| !part.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" ")
    };
    Ok(match kind {
        PromptKind::Join => vec![format!(
            "/join {}",
            joined(&[&need(0, tr_key("a channel"))?, value(1)])
        )],
        PromptKind::Query => {
            let nick = need(0, tr_key("a nick"))?;
            vec![format!("/query {}", joined(&[&nick, value(1)]))]
        }
        PromptKind::Notice => {
            vec![format!(
                "/notice {} {}",
                need(0, tr_key("a nick or channel"))?,
                need(1, tr_key("the text"))?
            )]
        }
        PromptKind::Whois => vec![format!("/whois {}", need(0, tr_key("a nick"))?)],
        PromptKind::Ctcp(Some(request)) => {
            vec![format!("/ctcp {} {request}", need(0, tr_key("a nick"))?)]
        }
        PromptKind::Ctcp(None) => vec![format!(
            "/ctcp {} {}",
            need(0, tr_key("a nick"))?,
            need(1, tr_key("the request"))?.to_ascii_uppercase()
        )],
        PromptKind::Away => vec![format!("/away {}", value(0)).trim_end().to_string()],
        PromptKind::Invite => vec![format!(
            "/invite {} {}",
            need(0, tr_key("a nick"))?,
            need(1, tr_key("a channel"))?
        )],
        PromptKind::Ban => {
            let nick = need(0, tr_key("a nick or mask"))?;
            let channel = need(1, tr_key("a channel"))?;
            let mut commands = vec![format!("/ban {channel} {nick}")];
            // A reason means kick as well.
            if !value(2).is_empty() && !nick.contains(['!', '@', '*']) {
                commands.push(format!("/kick {channel} {nick} {}", value(2)));
            }
            commands
        }
        PromptKind::Kick => {
            let nick = need(0, tr_key("a nick"))?;
            let channel = need(1, tr_key("a channel"))?;
            vec![format!("/kick {}", joined(&[&channel, &nick, value(2)]))]
        }
        PromptKind::Ignore => vec![format!("/ignore {}", need(0, tr_key("a nick or mask"))?)],
        PromptKind::Unignore => vec![format!("/unignore {}", need(0, tr_key("a nick or mask"))?)],
        PromptKind::Nick => {
            let nick = need(0, tr_key("a nick"))?;
            if nick.contains(char::is_whitespace) {
                return Err("A nick has no spaces.".into());
            }
            vec![format!("/nick {nick}")]
        }
        PromptKind::Quit => vec![format!("/quit {}", value(0)).trim_end().to_string()],
    })
}

impl OpenIrcApp {
    /// Open the prompt for `kind`, filled in from the active window.
    pub(super) fn open_prompt(&mut self, kind: PromptKind) {
        let window = self.windows.get(self.active);
        let channel = window
            .filter(|window| window.kind == Kind::Channel)
            .map(|window| window.title.clone())
            .unwrap_or_default();
        // The nick picked in the nick list, or the other side of a query.
        let nick = window
            .filter(|window| window.kind == Kind::Query)
            .map(|window| window.title.clone())
            .or_else(|| self.selected_nick.clone())
            .unwrap_or_default();
        let fields: Vec<(&'static str, &'static str, String)> = match kind {
            PromptKind::Join => vec![
                (
                    tr("Channel"),
                    tr("#channel, or several: #a,#b"),
                    String::new(),
                ),
                (
                    tr("Key"),
                    tr("only for a channel with a key"),
                    String::new(),
                ),
            ],
            PromptKind::Query => vec![
                (tr("Nick"), "", nick),
                (tr("Message"), tr("optional"), String::new()),
            ],
            PromptKind::Notice => vec![
                (
                    tr("To"),
                    tr("nick or #channel"),
                    if nick.is_empty() { channel } else { nick },
                ),
                (tr("Text"), "", String::new()),
            ],
            PromptKind::Whois | PromptKind::Ctcp(Some(_)) => vec![(tr("Nick"), "", nick)],
            PromptKind::Ctcp(None) => vec![
                (tr("Nick"), "", nick),
                (tr("Request"), tr("such as VERSION or TIME"), String::new()),
            ],
            PromptKind::Away => vec![(tr("Reason"), tr("blank uses \"away\""), String::new())],
            PromptKind::Invite => {
                vec![(tr("Nick"), "", nick), (tr("Channel"), "#channel", channel)]
            }
            PromptKind::Ban => vec![
                (tr("Nick or mask"), tr("nick, or nick!user@host"), nick),
                (tr("Channel"), "#channel", channel),
                (
                    tr("Kick reason"),
                    tr("blank bans without kicking"),
                    String::new(),
                ),
            ],
            PromptKind::Kick => vec![
                (tr("Nick"), "", nick),
                (tr("Channel"), "#channel", channel),
                (tr("Reason"), tr("optional"), String::new()),
            ],
            PromptKind::Ignore => vec![(tr("Nick or mask"), tr("nick, or nick!user@host"), nick)],
            PromptKind::Unignore => {
                let first = self
                    .settings
                    .ignores
                    .first()
                    .map(|rule| rule.full_mask())
                    .unwrap_or_default();
                vec![(
                    tr("Nick or mask"),
                    "",
                    if nick.is_empty() { first } else { nick },
                )]
            }
            PromptKind::Nick => vec![(tr("New nick"), "", self.me().to_string())],
            PromptKind::Quit => vec![(
                tr("Reason"),
                tr("blank uses your quit message"),
                String::new(),
            )],
        };
        self.prompt = Some(Prompt {
            kind,
            fields,
            note: String::new(),
            server: self.active_server(),
        });
    }

    /// Run what a filled-in prompt asks for, on the server it was opened
    /// for. /quit marks the hang-up on the server it runs on, so that
    /// server is the only one changed.
    pub(super) fn finish_prompt(
        &mut self,
        prompt: &Prompt,
        values: &[String],
    ) -> Result<(), String> {
        let commands = prompt_commands(prompt.kind, values)?;
        let server = prompt.server.or_else(|| self.active_server());
        self.on_server(server, |app| {
            app.in_run(|app| {
                for command in commands {
                    app.dispatch(command, Origin::Script);
                }
            });
            // Open query opens it: what is typed next goes there.
            if prompt.kind == PromptKind::Query
                && let Some(nick) = values.first().map(|nick| nick.trim())
                && !nick.is_empty()
            {
                app.open_query(nick);
            }
        });
        Ok(())
    }

    pub(super) fn prompt_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut prompt) = self.prompt.take() else {
            return;
        };
        let mut open = true;
        let mut run = false;
        let mut cancel = false;
        let ignores: Vec<String> = self
            .settings
            .ignores
            .iter()
            .map(|rule| rule.full_mask())
            .collect();
        egui::Window::new(prompt.kind.title())
            .id(egui::Id::new("command-prompt"))
            .default_pos(dialog_pos(ctx, 8))
            .collapsible(false)
            .resizable([false, false])
            .open(&mut open)
            .show(ctx, |ui| {
                if let PromptKind::Ctcp(Some(request)) = prompt.kind {
                    ui.label(trf("Ask for {0}.", &[&request]));
                }
                form_grid("prompt-fields")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        for (index, (label, hint, value)) in prompt.fields.iter_mut().enumerate() {
                            ui.label(*label);
                            let response =
                                ui.add(field(value).hint_text(*hint).desired_width(240.0));
                            if index == 0 && ui.memory(|memory| memory.focused().is_none()) {
                                response.request_focus();
                            }
                            if response.lost_focus()
                                && ui.input(|input| input.key_pressed(egui::Key::Enter))
                            {
                                run = true;
                            }
                            ui.end_row();
                        }
                    });
                if prompt.kind == PromptKind::Unignore && !ignores.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new(tr("Ignored:")).color(muted_on_face()));
                        for mask in &ignores {
                            if ui.small_button(mask).clicked() {
                                prompt.fields[0].2 = mask.clone();
                            }
                        }
                    });
                }
                if !prompt.note.is_empty() {
                    ui.colored_label(super::tone_color(super::Tone::Attention), &prompt.note);
                }
                ui.horizontal(|ui| {
                    if ui.button(prompt.kind.button()).clicked() {
                        run = true;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        cancel = true;
                    }
                });
            });
        if run {
            let values: Vec<String> = prompt
                .fields
                .iter()
                .map(|(_, _, value)| value.clone())
                .collect();
            match self.finish_prompt(&prompt, &values) {
                Ok(()) => return,
                Err(note) => prompt.note = note,
            }
        }
        if open && !cancel {
            self.prompt = Some(prompt);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_start_filled_in_from_the_window() {
        let mut app = super::super::tests::test_app();
        let channel = app.ensure(Kind::Channel, "#c");
        app.activate(channel);
        app.selected_nick = Some("ann".into());
        app.open_prompt(PromptKind::Kick);
        let prompt = app.prompt.as_ref().unwrap();
        assert_eq!(prompt.fields[0].2, "ann");
        assert_eq!(prompt.fields[1].2, "#c");
        let query = app.ensure(Kind::Query, "bob");
        app.activate(query);
        app.open_prompt(PromptKind::Ctcp(Some("VERSION")));
        assert_eq!(app.prompt.as_ref().unwrap().fields[0].2, "bob");
    }

    fn values(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn prompts_build_their_commands() {
        assert_eq!(
            prompt_commands(PromptKind::Join, &values(&["#a,#b", "key"])).unwrap(),
            vec!["/join #a,#b key"]
        );
        assert_eq!(
            prompt_commands(PromptKind::Query, &values(&["ann", ""])).unwrap(),
            vec!["/query ann"]
        );
        assert_eq!(
            prompt_commands(PromptKind::Ctcp(Some("VERSION")), &values(&["ann"])).unwrap(),
            vec!["/ctcp ann VERSION"]
        );
        assert_eq!(
            prompt_commands(PromptKind::Ctcp(None), &values(&["ann", "userinfo"])).unwrap(),
            vec!["/ctcp ann USERINFO"]
        );
        assert_eq!(
            prompt_commands(PromptKind::Ban, &values(&["troll", "#c", "bye"])).unwrap(),
            vec!["/ban #c troll", "/kick #c troll bye"]
        );
        assert_eq!(
            prompt_commands(PromptKind::Ban, &values(&["*!*@spam", "#c", "bye"])).unwrap(),
            vec!["/ban #c *!*@spam"],
            "a mask is banned, not kicked"
        );
        assert_eq!(
            prompt_commands(PromptKind::Kick, &values(&["troll", "#c", ""])).unwrap(),
            vec!["/kick #c troll"]
        );
        assert_eq!(
            prompt_commands(PromptKind::Away, &values(&[""])).unwrap(),
            vec!["/away"]
        );
        assert!(prompt_commands(PromptKind::Notice, &values(&["ann", ""])).is_err());
        assert!(prompt_commands(PromptKind::Nick, &values(&["two words"])).is_err());
        assert_eq!(
            prompt_commands(PromptKind::Quit, &values(&["bye all"])).unwrap(),
            vec!["/quit bye all"]
        );
    }
}
