# OpenIRC review findings and fixes

This file records the peer review of OpenIRC 0.1.0 (Rust 2024, eframe/egui 0.36)
and the work done on each finding afterwards. It is meant for the next reviewer.

- **Seen**: reproduced in the window (driven over X11 under WSLg) or on the wire,
  against a local fake IRC server.
- **Code**: found by reading the source.
- **Status** says what changed and how it was checked. "Verified in the window"
  means the same steps that showed the bug were repeated after the fix.

## Checks to run

`scripts/check.sh` runs every check below in order and stops at the first
failure; CI (`.github/workflows/check.yml`) runs the same script on Linux and
clippy plus the tests natively on Windows. `--quick` skips the Windows
type-check and the benchmarks; `CARGO_NET_OFFLINE=true` uses the advisory
databases already on disk. The benchmarks are compared with
`target/bench-baseline.txt` (made by the first run without a comparison),
and the run fails when one is more than `BENCH_TOLERANCE` percent (default
50) slower, or when the baseline cannot be read or does not list exactly the
benchmarks that ran. `--save-bench` skips that comparison and keeps the new
results as the baseline, for a new machine or an intended slowdown. The
tests and benchmarks run under a memory cap, so a runaway test stops
alone. Where systemd can make a scope, that scope may use at most
`TEST_MEMORY_MAX` in all (default 2G; the tests use about 250 MB) and no
swap. Elsewhere, as on CI runners without a user systemd, each process may
reserve at most four times that much address space (`ulimit -v`). That is a
looser per-process limit, not a total memory ceiling.

The individual commands (the test count here is from the first round):

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --features bench -- -D warnings
cargo test                                    # 118 tests
cargo audit                                   # RustSec, see .cargo/audit.toml
cargo deny check                              # RustSec + licenses + sources, see deny.toml
cargo run --release --features bench --example bench
```

All of these pass. Clippy was also run for `x86_64-pc-windows-gnu` (with the
host `gcc` standing in for ring's C build, which `cargo clippy` only needs to
type-check): clean. The macOS target was not checked (no target installed).

Before the work: build, 76 tests, and clippy passed; `cargo fmt --check`
failed with 41 diffs; the Windows target did not compile.

## Bugs

### B1. One non-UTF-8 byte from any user disconnects the session
- Where: `connection.rs` `Connection::read_effect`
- Seen: `PRIVMSG #openirc :caf\xe9` ended the session ("stream did not contain valid UTF-8").
- **Status: fixed.** Lines are read as bytes and decoded by `message::decode_line`
  (UTF-8, else Latin-1). Same for DCC chat. Tests:
  `connection::tests::split_lines_survive_idle_timeouts_and_latin1_does_not_disconnect`,
  `message::tests::decode_line_*`. Verified in the window and in `--cli`: "café" shown, no disconnect.

### B2. Server lines split across packets lost their first half
- Where: `connection.rs` `read_effect`; `dcc.rs` `split_chat`
- Seen: a line sent in two writes 400 ms apart showed only "across two packets".
- **Status: fixed.** New `message::LineBuffer` keeps partial lines across idle
  timeouts (and caps a runaway line at 16 KiB). Used by the IRC and DCC chat
  readers. Taking a line moves an offset instead of draining the front, so a
  7 MB burst is linear (was 17 s for 200k lines in a worst-case buffer, now 8 ms).
  Verified in the window.

### B3. Remote command injection through legacy remotes
- Where: `script.rs` `commands`, `expand`
- Seen: remote `on text *hi* /notice $nick you said $text` plus a message
  `hi | /raw PRIVMSG NickServ :DROP` made the client send `PRIVMSG NickServ :DROP`.
- **Status: fixed.** The body is split on ` | ` before substitution, and `expand`
  substitutes in one pass so inserted text (`%secret`, `$me`) is never expanded
  again. Outgoing lines also have CR/LF replaced before writing.
  Test: `script::tests::text_from_other_users_cannot_add_commands_or_read_variables`.
  Verified on the wire: one NOTICE with the literal text, nothing to NickServ.

### B4. The nick list was never on screen
- Where: `ui.rs` `OpenIrcApp::client`
- **Status: fixed.** The client area is now panels: header (topic, Find),
  input at the bottom, a resizable nick list on the right, transcript in the
  middle. Test: `ui::tests::a_frame_lays_out_the_nick_list_inside_the_window`
  (headless frame). Verified in the window.

### B5. Right-click on a transcript line never opened a menu
- **Status: fixed.** Each painted line has its own click-sensing response with
  Whois, Query, Copy line, and the user's popups. Popups run through the
  command dispatcher. Verified in the window (Slap sent `ACTION slaps alice`).

### B6. One bad settings line wiped the whole file
- Where: `settings.rs` `parse_settings`, `load`, `save`
- **Status: fixed.** Each line stands alone; bad lines are skipped and reported
  in Status; on any warning the original is copied to `settings.txt.bak`
  first. If the file exists but cannot be read, saving is refused (`locked`).
  Saves go through a temporary file of their own (`settings.txt.<pid>-<n>.tmp`,
  created fresh) and a rename; see F25 in the thirteenth round.
  Tests: `settings::tests::one_bad_line_is_skipped_and_reported_and_the_rest_still_loads`,
  `a_locked_settings_value_refuses_to_save`.

### B7. Favorites without a channel (and empty vars) were dropped on load
- **Status: fixed.** Only line endings are stripped. The file is now format 2,
  which escapes `\`, tab, CR, and LF in every value, so any text round-trips;
  format 1 files still load. Tests: `every_text_field_survives_a_round_trip`,
  `format_one_files_still_load`. Verified in the window (NoChan favorite kept).

### B8. QUIT was shown in the wrong channels
- **Status: fixed.** Shown only in channels where the nick was a member (checked
  before removal) and in its query. Test: `ui::tests::quit_is_shown_only_where_the_nick_was`.
  Verified in the window.

### B9. NICK was written into every window
- **Status: fixed.** Shown in member channels, the renamed query, and Status for
  your own nick; nick lists re-sorted. Test:
  `ui::tests::nick_changes_touch_member_channels_and_rename_the_query`. Verified in the window.

### B10. Script output always went to the server
- **Status: fixed.** One dispatcher (`OpenIrcApp::dispatch`) handles typed
  lines, aliases (settings and script), event handlers, popups, timers,
  perform, and dialog buttons. Local commands work offline: `/echo`, `/clear`,
  `/help`, `/timer`, `/set`, `/unset`, `/dialog`. Nesting is capped at 8.
  Tests: `local_commands_work_without_a_server`,
  `script_aliases_can_call_local_commands_and_other_aliases`, `runaway_aliases_stop`.
  Verified in the window (`on START { echo started }` prints "started").

### B11. Dialog editor lost work
- **Status: fixed.** Drags update the dialog text and save when the drag ends;
  positions are clamped to the canvas and keep sub-pixel movement. Commands
  keep everything after the position (commas included); labels and titles
  escape quotes. Add buttons are disabled with no dialog picked; Delete and
  Open were added. Test: `dialog::tests::commas_in_commands_and_quotes_in_labels_round_trip`.
  Verified in the window.

### B12. Live dialogs rendered edit and check as labels and ignored positions
- **Status: fixed.** Real text fields and checkboxes at the dialog's
  coordinates. Button commands can use `$did(id)` for a control's value.
  Test: `ui::tests::did_fills_dialog_values`. Verified in the window
  (`/echo name= remember=1, done`).

### B13. Dialogs committed changes without Save
- **Status: fixed.** Options edits a draft (live preview of colors and font);
  OK/Apply copy it back, Cancel or × drop it. Connect edits a draft; Cancel
  drops it. Scripts edits a draft; the running script changes only on
  "Save script", which validates first and shows errors in the window.
  Verified in the window (Cancel left `timestamps true` and `font_size 13` on disk).

### B14. Windows build could not compile
- **Status: fixed.** `clock.rs` owns local time (`localtime_r` on Unix,
  `localtime_s` on Windows). The Windows target now type-checks and passes clippy.

### B15. Colors did not recolor old lines; one Ink could not fit both backgrounds
- **Status: fixed.** Lines store roles (`Tone`), resolved at paint time.
  Chrome text is chosen for contrast against Face; selection text against
  Highlight; nick, link, and message colors are nudged to 4.5:1 against Paper.
  The Colors page warns when Ink on Paper is below 4.5:1.
  Test: `readable_colors_reach_aa_contrast`. Verified in the window with near-black Paper and white Ink.

### B16. SASL could hang registration and used the wrong account
- **Status: fixed.** `CAP NAK` sends `CAP END`. The account is the new
  "SASL account" option, else the configured nick (never `nick_`).
  Test: `pump::tests::sasl_nak_ends_cap_and_the_account_is_not_the_retry_nick`.

### B17. Flood limiting dropped joins and desynced the nick list
- **Status: fixed.** Held joins still add the nick (new `Event::Member`) without
  a line. Messages over the limit are dropped and the notice now says so.
  Test: `flood_drops_extra_messages_and_hides_extra_join_lines`.

### B18. DCC filename could escape the downloads folder on Windows
- **Status: fixed.** `safe_filename` drops path parts, drive colons, reserved
  characters and device names. New downloads never overwrite (`unique_path`).
  Filenames with spaces are quoted in offers.
  Tests: `safe_filename_keeps_downloads_inside_the_folder`, `unique_path_*`, `filenames_with_spaces_*`.

### B19. Settings stored passwords world-readable and showed them unmasked
- **Status: fixed.** settings.txt is written 0600 on Unix (verified: mode 600);
  proxy and SASL password fields are masked.

## Usability

- **U1 Network list (fixed).** Every match is listed in a scrolling box with a
  count; the directory downloads only on "Update list". Verified in the window.
- **U2 Scrolling and font size (fixed).** Options and Scripts scroll with their
  buttons always reachable; topic, nick list, input, and window titles follow
  the font size. Verified at font 28.
- **U3 Selected items (fixed).** Selected text is chosen against Highlight.
- **U4 "talking to" noise (fixed).** Tab switches use a quiet `/target -q`.
- **U5 Logs (fixed).** Every line logged with date/time and full text (nick,
  joins, parts, quits, topic, notices); one file per window, server status
  logs named by host; the Log window shows the last 256 KiB, read once, with Refresh.
- **U6 Dialog placement and pickers (fixed).** Dialogs open at staggered
  positions; swatches have borders; the picker has no alpha.
- **U7 Smaller gaps (fixed).** Disconnect is disabled when not connected; a
  click selects a nick, a double-click opens a query; links are styled and only
  the link opens; Play reports a missing or non-WAV file; script errors show in
  the Scripts window; no tray retries.
- **U8 Quit and DCC waits (fixed).** Quitting sends QUIT on every server and
  waits up to 500 ms (verified on the wire); a vanished network thread is
  noticed; DCC listeners give up after 120 s; file sends no longer wait for an
  ack per 4 KiB chunk.
- Also: without a system tray (WSLg, Wayland-only) the tray is not created,
  which removes the `Gtk-CRITICAL` warning seen at start.

## Maintainability

- **M1 Scripting systems (partly).** The three systems still exist (legacy
  remotes in `script.rs`, settings aliases, `slang.rs`), but they share one
  wildcard matcher (now real `*`/`?` globbing), one dispatcher, and the parsed
  script is cached (`slang::Cache`). Folding remotes into slang is left open.
- **M2 `ui.rs` size (partly).** Still one large module. It now has 9 tests
  (event routing, dispatcher, scrollback, contrast, a headless layout frame), and
  the parallel `send_offers`/`send_from` vectors are one vector of pairs.
- **M3 Transcript cost (fixed).** See Performance.
- **M4 Tray retry (fixed).** One attempt.
- **M5 Tests (fixed).** Tests no longer write to the real config dir; the GTK
  test skips without GTK; flood test names match behavior; round-trip and
  split/Latin-1 tests added.
- **M6 rustfmt (fixed).** `cargo fmt --check` is clean.

## Performance and profiling

**Profiler.** Set `OPENIRC_PROFILE=1`. The window then times each frame and its
sections (`logic`, `logic.network`, `ui`, `ui.client`, `ui.transcript`).
View, Profiler shows live count/mean/p50/p95/max. On exit the summary is
printed to stderr and written to `profile.txt` in the config directory.
When the variable is unset, the cost is one branch per section.

**Benchmarks.** `cargo run --release --features bench --example bench`
(set `XDG_CONFIG_HOME` to a scratch directory). The UI cases drive the real
`App::ui` headlessly at 980x640 through egui.

Release build, this machine (WSL2), 120 frames each:

| Case | Before | After |
|---|---|---|
| Transcript frame, 5000 lines | 24.2 ms | 0.42 ms |
| Transcript frame, 200 lines | 0.85 ms | 0.40 ms |
| 5000 lines, one new line per frame | – | 0.45 ms |
| 5000 lines, Find active | rescan per frame | 0.41 ms |
| 5000 lines, window resized every frame | – | 4.0 ms |
| Script event, 40 handlers | 76.8 µs (parse each time) | 4.9 µs |
| Wire split + decode, 200k lines | 17 s worst case | 5.4 ms |

"Before" for the transcript is `bench_legacy_transcript`, a reproduction of the
old per-frame loop (clone every line, one label per span, spell and URL scan per
span) kept in the bench build for comparison; the original source tree itself
was lost when the review machine crashed, so a byte-for-byte rerun was not possible.

What changed:
- Lines are laid out as one galley each and only lines in the viewport are
  painted. Heights are cached per (width, font, timestamps); after a resize or
  font change, on-screen lines are measured at once and up to 128 others per
  frame, so a long scrollback converges without a long frame.
- Scrollback is capped at 5000 lines per window.
- Find results are cached until the text, window, or its lines change.
- The egui style is rebuilt only when colors or font size change.
- The parsed script is cached until its text changes.
- `LineBuffer` makes socket line splitting linear.

## Security and dependencies (RustSec)

- `cargo audit` (1310 advisories, 417 crates): no vulnerabilities.
- One "unmaintained" notice, `RUSTSEC-2026-0192` (`ttf-parser`), reaches OpenIRC
  only through winit's Wayland decorations (`winit -> sctk-adwaita -> ab_glyph`).
  OpenIRC does not parse fonts with it. It is ignored with this reason in
  `deny.toml` and `.cargo/audit.toml`; revisit when eframe/winit drop sctk-adwaita.
- `webpki-roots` moved from 0.26 to 1.x.
- `cargo deny check`: advisories, bans, licenses, and sources all pass.
  Duplicate crate versions inside the GUI stack are reported as warnings only.

## Not covered

- Typing could not be tested by hand: WSLg never gave the window X keyboard
  focus. Text paths are covered by unit tests and by seeded settings.
- Real networks, TLS fingerprints, SASL against a real server, proxies, and
  DCC between two clients were not run live (unit tests cover the wire formats
  and a localhost transfer).
- macOS was not built. The tray icon on Windows and macOS is still not implemented
  (a known gap, reported honestly at runtime).

## Second round: feature backlog and UI quality (2026-10-13)

A feature inventory with priorities and status is in `FEATURES.md`.
Highlights of what was added:

- **Application icon.** Drawn in code (`src/icon_art.rs`): a blue tile with a
  speech bubble and `#`. Set on the window at run time (X11/Wayland taskbars
  and WSLg, which showed a penguin before) and embedded as an `.ico` in Windows
  builds by `build.rs` (`winresource`; skipped with a warning if no resource
  compiler is installed).
- **Window chrome** (`src/ui/chrome.rs`, `src/ui/icons.rs`): painted icon
  toolbar with tooltips; switchbar tabs and treebar rows with kind icons and
  activity colors (event, message, highlight); per-network tree with a
  connection dot and user counts; status bar with connection state, network,
  user/op counts, queued lines, and an online timer; menus rebuilt around one
  `Command` type with shortcut hints; keyboard shortcuts (Alt+E/O/J/L/R/B/U/K/S,
  Alt+Z or Ctrl+W close, Ctrl+Tab and Ctrl+PageUp/Down, Alt+1..9, Ctrl+F, F11).
- **Channel view** (`src/ui/view.rs`): channel header with modes (key hidden)
  and user count; double-click the topic for Channel central; nick list left,
  right, or hidden, with status badges, groups, and counts; unread line marker;
  optional line shading and monospace transcript; bold, italic, underline,
  strikethrough, reverse, and background colors from message codes (DejaVu Sans
  Bold/Oblique and Sans Mono are now bundled); event lines in their own colors;
  highlighted lines and private messages raise a corner notice.
- **Input line**: history (Up/Down), nick completion (Tab, cycling, `: ` after
  the first word), Ctrl+B/U/I/R/O/K formatting with a color palette, and a
  confirm prompt for multi-line pastes.
- **Channel central and channels list** (`src/ui/channel.rs`): topic, mode
  checkboxes, key and limit, and ban/exception/invite lists with add and remove;
  a sortable, filterable channel list with double-click to join.
- **Protocol**: alternative nick, quit and finger replies, INVITE and NETWORK
  events, channel flags and member status kept current from MODE (the nick list
  did not update on +o/+v before), outgoing flood queue (`SendQueue`: burst of
  5, then one line per 2 s), and a reconnect that keeps the auto-join channel
  even when the first attempt failed.
- **Schemes**: Classic (default), Daylight, Midnight, High contrast.

Checks after this round: `cargo fmt --check`, clippy with `-D warnings` (Linux
and Windows targets, with and without `bench`), 118 tests, `cargo audit`, and
`cargo deny check` all pass. Frame times are unchanged or better (5000-line
channel: 0.40 ms idle, 4.5 ms while resizing every frame).

## Third round: working down FEATURES.md (2026-10-13)

Every row of `FEATURES.md` now has a final status: done, partly (with what is
left), or not planned (with the reason). New modules: `src/servers.rs`,
`src/certs.rs`, `src/net.rs`, `src/rules.rs`, `src/ui/servers.rs`,
`src/ui/rules.rs`, `src/ui/logs.rs`, `src/ui/transfers.rs`, `src/ui/scripts.rs`.
New dependencies: `encoding_rs`, `rcgen` (ring backend only), `socket2`, `regex`.

- **Connecting**: a server list grouped by network, with per-server port,
  password, login (SASL PLAIN, SASL EXTERNAL, NickServ), nick, encoding, and
  channels; client certificates (create one, CertFP); a prompt to trust a
  server certificate that does not verify, pinned by SHA-256 per host; timeout,
  retry count and delay, and moving to the next server of the network.
- **Network page**: address order (IPv4/IPv6 first), bind address with
  fallback, DCC port range and DCC address (local, as the server sees it, or
  set by hand), proxy for servers, DCC, or both with exception masks, and
  identd system name, port, and connecting-only mode.
- **Rules pages**: highlight rules (words, wildcards, regular expressions,
  where, case, color, sound, flash, notice), typed and timed ignores (per
  network; `/ignore -pcntdik -uSECONDS -wNETWORK mask`), nick color rules (by
  mask, mode, idle time), and auto-op/voice/halfop with protect.
- **Messages**: CTCP answers limited over all senders, a limit on new query
  windows (the overflow goes to a Messages window), whois/away/notice/CTCP/
  invite shown in the active window as chosen, and an optional single Messages
  window. CTCP requests and replies are shown, and `/ctcp nick ping` reports the
  round trip.
- **IRCv3**: `CAP LS 302` negotiation with multi-prefix, away-notify,
  account-notify, extended-join, chghost, server-time, message-tags,
  invite-notify, cap-notify, and userhost-in-names; typing notices both ways;
  STS (a plain connection is upgraded at once; the policy is remembered from a
  TLS connection for its duration). Away people are dimmed in the nick list;
  the tooltip shows the away reason and services account.
- **Input**: multi-line input (Enter sends, Shift+Enter breaks the line),
  command prefix and separator, double-click commands for nicks and for
  channel, query, and status windows, and a command for dropped files
  (`/dcc send $nick $file` by default). New local commands: `/channel`,
  `/dcc send|chat`, `/debug`.
- **Display**: an editable 16-color message palette, minimize to tray and a
  blinking tray icon (Linux), and window order as opened or by kind and name.
- **Logs**: per-network folders, a new file per day or month, keep or strip
  codes, size trim, the last N lines shown when a window opens (with the
  logged times), and a Logs window to find, read, rename, and delete logs.
  Lines that arrive with a server time are logged with that time.
- **DCC**: an Incoming file prompt (save, resume, overwrite, or a new name),
  trusted senders accepted without asking, refused file types, folders by file
  type, and a Transfers window with progress, speed, cancel, and Show folder.
- **Scripts**: the language gained `&&`, `||`, `!`, grouping, `>=`, `<=`,
  `isin`, `iswm`, `elseif`, `/return`, `/halt`, `$+`, aliases as identifiers
  (`$name(args)`), a real `$calc`, and `$left`, `$right`, `$mid` (negative
  counts), `$replace`, `$remove`, `$pos`, `$str`, `$gettok`, `$numtok`,
  `$istok`, `$chr`, `$asc`, `$abs`, `$int`, `$round`, `$rand`, `$strip`,
  `$ctime`, `$date`. `on` events take user levels (numbers or names; Options,
  Users). The editor has several script files that run in order, line numbers,
  find and replace, go to line, and a bracket and syntax check with line
  numbers. `/debug` shows raw lines (passwords masked) and script events.

Bugs found and fixed on the way:

- A channel window opened in the background (for example by a topic or
  names reply) took the focus from the active window: inserting it shifted
  the windows after it without moving the active index.
- The Options window changed width from page to page.
- The selected row in rule lists and the server list was black text on the
  dark selection color.
- In the Debug window a script event could appear before the raw line that
  caused it.
- `/ignore -n` was ambiguous (notices or network); the network flag is `-w`.

Checked live against a local test server:
STS upgrade from 6667 to TLS 6697 with the client certificate, and the
remembered policy on the next start; away dimming, typing notice, server-time
playback, invite-notify; every new Options page; the script editor (new file,
check); the Logs window (view, delete with confirmation); a 3 MB DCC receive
through the prompt, with progress and speed, saved byte-identical, then the
same offer again (Resume disabled for a complete file, Save as `report (1).bin`);
Clear history; `/debug` by a double-click command.

Not covered in this round: typing in the live window (WSLg gives it no
keyboard focus; key paths have headless frame tests), the tray icon (WSLg has
no system tray; the options say so when there is none), and IRCv3 against a
real network.

Checks after this round: `cargo fmt --check`, clippy with `-D warnings` (Linux
and Windows targets, with and without `bench`), 161 tests, `cargo audit`, and
`cargo deny check` all pass. Frame times are unchanged (5000-line channel:
0.41 ms idle); parsing a PRIVMSG went from about 0.30 to 0.37 µs because the
server-time tag is read now.

## Fourth round: typed commands (2026-10-13)

Prompted by `/server -m` not existing. `COMMANDS.md` lists the typed
commands by name, with what each does in OpenIRC's words and
its status (had, added, sent to the server, script, or not planned with a
reason). New module `src/ui/commands.rs`.

- **Unknown commands now go to the server** as `COMMAND arguments`, as most
  IRC clients do, so `/lusers`, `/motd`, `/stats`, `/oper`, `/userhost`,
  `/ison`, `/map`, `/version`, and the rest work. Before, they were refused.
- **Connections**: `/server [-m] [-e] [-p password] [-j #channels] host
  [[+]port] [password]` moves the current connection (its channel and query
  windows close) or, with `-m`, opens another one; a saved server is found by
  address or network name. `/server` alone reconnects. Also `/disconnect`,
  `/exit`, `/scon -a` (a command on every connected server), and `/dns`.
- **Messages**: `/join` takes several channels and keys and adds a missing
  `#`; `/partall`, `/describe`, `/say`, `/amsg`, `/omsg`, `/onotice`,
  `/ctcpreply`, `/ping`, and `/cs` `/ms` `/hs` `/bs` `/os` for services.
  `/admin` without a nick asks the server.
- **Windows and settings**: `/list`, `/url`, `/close [-m|window]`,
  `/closemsg`, `/clearall`, `/savebuf`, `/echo -s` and `/echo <window>`,
  `/editbox`, `/notify [-r]`, `/log on|off`, `/alias`, `/timers [off]`,
  `/fullname`, `/anick`, `/auser`, `/ruser`, `/ulist`, `/load`, `/unload`,
  `/ban -u<seconds>`, `/run`, `/play`, `/beep`, `/flash`, `/tip`.
- Switchbar labels for several servers use the network name, else a short
  host that keeps IP addresses whole (`127.0.0.1` showed as `127`, and
  `irc.libera.chat` as `irc`).

Bug found while testing live: `/server -m host 6667` picked up the TLS
setting of a saved entry for the same host on port 6697 and sent a TLS
handshake to the plain port. An explicit port that differs from the saved
one now takes its TLS setting from the command.

Checked live against the local test server through double-click commands
(typing is not possible under WSLg): `/server -m` opened a second connection
(two groups in the tree, "2 servers" in the status bar); `/scon -a /lusers`
sent `LUSERS` to both; `/server -j #second host` moved a connection, closed
its old channel window, and joined `#second`.

Checks: `cargo fmt --check`, clippy with `-D warnings` (Linux and Windows,
with and without `bench`), 166 tests, `cargo audit`, and `cargo deny check`
pass.

## Fifth round: toolbar depth and icons (2026-10-13)

- New icon set (`src/ui/icons.rs`), drawn in code: filled, colored shapes
  with a dark outline, a gloss toward the top left, and a gray look when
  disabled. Used in the toolbar, switchbar, and tree.
- The toolbar has a gripper, etched separators between groups, a raised band
  edge, and buttons that are flat at rest, raised on hover, and sunken while
  pressed. New buttons: notify list, log files, URL list, transfers, about.
- Fixed while doing it: egui fills a convex shape only when its points run
  clockwise, so the pen now turns every filled shape clockwise.

## Sixth round: transcript details (2026-10-13)

From a review of a live session in a 700-user channel:

- Joining no longer prints every name. The window shows "Now talking in
  #channel", the topic, who set it and when (333), a one-line summary such as
  "#chan: 736 users (12 ops, 68 voiced)", the channel modes without the key
  (324, asked for on join), and when the channel was created (329). `/names`
  still prints the full list. Others' joins show their address:
  "**nick** (user@host) joins #channel".
- Bug: topic changes by other people were not tracked. The TOPIC line was
  shown raw in the active window and the channel header kept the old topic.
  Now the header updates and the channel says "**nick** changes topic to: ...".
- The status bar shows your user modes (`nick on Network +iSx`) and the
  channel's modes.
- Timestamps were drawn at about 2.3:1 contrast; they now use a half mix of
  ink and paper (about 4:1).
- Long channel names in the tree no longer run into the user count, and long
  nicks in the nick list end in "…" with the full nick on hover.
- Per-window fixed-width font: right-click a switchbar tab, a tree row, or a
  transcript line, "Fixed-width font". Remembered by window name, so bot
  channels and server MOTDs can line up while chat stays proportional.
- Channels list: a Modes column (from `[+modes]` at the start of a topic),
  topics with their colors, a maximum-users filter, and a box to join several
  channels at once.

Checks: fmt, clippy (Linux and Windows, with and without `bench`), 170
tests, `cargo audit`, `cargo deny check`.

## Seventh round: alignment sweep and a TLS update (2026-10-14)

Started from a report that the "Logging" item in the Tools menu sat further
right than its neighbours. Every menu, context menu, Options page, and
dialog was then checked on screen for the same kind of problem.

- **Menus.** Checkable items faked their check column with three spaces,
  so they sat at a different indent from plain items, and a ticked item at
  a different indent from an unticked one. Every menu row (menu bar,
  submenus, and the tab, tree, transcript, nick list, and channels list
  context menus) now has the same fixed check column (`menu_row` in
  `src/ui/chrome.rs`). Nick headings in context menus line up with the rows.
  The Window menu's window list shows Alt+1 to Alt+9 as shortcuts on the
  right instead of number prefixes that pushed the first nine titles over.
- **Forms.** Label-and-field grids each sized their own label column, so
  sections on one page had fields starting in different places. All form
  grids share one label width (`form_grid`, 175 px). Rows that put the label
  above the field, or ran as a sentence with a control in the middle, are
  now grid rows: ident user ID, font size, dictionary, the logging options,
  the timestamp format, window order, the proxy rows, perform, the DCC
  folder, refused types and "if the file exists", favorites, the DCC dialog,
  and the default user level. The Servers form fields share one width.
- **Lists with buttons.** Notify, perform, address book, favorites,
  trusted certificates, and incoming DCC offers put a button after a label,
  so buttons drifted with the label length. They are grids now.
- **Small fixes.** Channel central labels read "Invite only (+i)" instead of
  "+i  Invite only" (letters differ in width) and the key and limit join the
  same columns. Palette swatches no longer stretch in the last column. Sound
  event names are capitalized.
- **Security.** Four rustls advisories published today (RUSTSEC-2026-0346 to
  0349, including a key-exchange downgrade) are fixed by rustls 0.23.46.
  `cargo audit` and `cargo deny check` pass again; a TLS connection to
  Libera.Chat registered normally after the update.

Checks: fmt, clippy (Linux and Windows, with and without `bench`), 170
tests, `cargo audit`, `cargo deny check`.

## Eighth round: the Commands menu (2026-10-14)

The Commands menu had Join, Part, Channel central, Names, Hop, Whois, Who,
Invite, Away, Back, and Clear. Query, notice, CTCP, ban, kick, ignore,
unignore, change nick, and quit with a reason could only be typed. The four
existing input dialogs were also rough: labels above the fields, Enter did
not submit, Whois had no label, and Invite always used the current channel.

- One prompt dialog for every menu command that needs input
  (`src/ui/prompts.rs`): labels in the shared form column, Enter runs the
  command, and fields start filled in with the nick picked in the nick list
  (or the query's nick) and the active channel. Missing fields are named
  in the dialog instead of sending a broken command.
- Commands menu: Join channel, Part channel, Channel central; Query user,
  Send notice, Whois user, Send CTCP (Version, Ping, Time, Finger, Client
  info, Other), Set away (Away, Back); Invite, Ban (with an optional kick
  reason), Kick, Ignore, Unignore (lists the current ignores to pick from);
  Names, Who, Hop, Clear window; Change nick, Quit IRC (with a reason, and
  no automatic reconnect afterwards).
- Checked live against a local test server: with a nick picked, Send CTCP,
  Version sent `PRIVMSG carol :\x01VERSION\x01`; Kick user sent
  `KICK #openirc carol`; Whois user sent `WHOIS carol carol`.

Checks: fmt, clippy (Linux and Windows, with and without `bench`), 172
tests, `cargo audit`, `cargo deny check`.

## Ninth round: features the first list missed (2026-10-14)

A second pass over what experienced users expect of the panels, the notify
list, and the input line; `FEATURES.md` has a new "Round 9" table.

- **Notify that works.** OpenIRC sent its notify list but never read the
  answers. The session now watches with MONITOR where the server offers it
  (730/731 replies) and otherwise checks with ISON every minute (303). People
  coming online or leaving get a status line and a notice; the Address book
  shows a green dot for those online. All notify edits (Options, Address
  book, `/notify`) send the list to every server, which works out the change.
- **Monitor panel** (`src/ui/monitor.rs`): a resizable bottom panel with
  Highlights, Private messages, Notices, and Server messages from every
  window and server, unread counts on the tabs, Clear, and double-click to
  go to the source window. View menu or F2.
- **Away when idle**: after the minutes set on the IRC page, every connected
  server is set away (not ones you set away yourself); typing comes back.
- **Last session's windows**: channels and queries open at exit (and as you
  join, close, or get messaged) are reopened on the next connect to that
  server.
- **Tab completion** of channels (`#op`) and commands (`/jo`) besides nicks.
- **Popups per menu**: script popups can go in the transcript line, nick
  list, channel tab, query tab, or status tab menu.
- **Search all windows** (Tools, Ctrl+Shift+F), with double-click to open
  the window with the text in its Find bar.
- **Display**: whole messages in the nick's color, custom title bar text
  (`$me`, `$network`, `$window`), hide nick list badges, hide `#` in the
  treebar.
- **Script editor syntax coloring** for comments, block words,
  `$identifiers`, `%variables`, and brackets, in light and dark schemes.

Not planned: emoji for emoticon shortcuts (no emoji font is bundled).
Still open: input history between sessions, a user-chosen toolbar, keeping
the computer awake during DCC transfers.

Checked live against a local test server: MONITOR sent on connect, "alice
is online" notice and Address book dot, the monitor panel tabs and counts,
and the colored script editor. Checks: fmt, clippy (Linux and Windows, with
and without `bench`), 181 tests, `cargo audit`, `cargo deny check`.

## Tenth round: channels list on connect, and the last open rows (2026-10-14)

- **Channels list on connect.** When a server finishes connecting, its
  channels list opens, without fetching the list until Get list is
  pressed (large networks have tens of thousands of
  channels). The window now belongs to one server: its title names the
  network, Get list asks that server, and list replies from other servers
  are ignored. The layout follows the familiar dialog: a title band and a
  short note, the list, then the filter and user limits, a join box with
  seconds between joins, and Join, Get list, Save list, and Close, with a
  "Show this when connecting" box (also on the IRC page).
- **Typed history between sessions**: the last 100 lines are kept, saved
  every ten lines and at exit; anything that may hold a password (IDENTIFY,
  PASS, OPER, NickServ, REGISTER, GHOST, "password") is never written.
- **Toolbar buttons** can be turned off one by one (Options, Display); a
  group with nothing left also loses its separator.
- **Keep awake during transfers** (`src/awake.rs`): Windows is asked not to
  sleep while a DCC transfer runs; Linux takes a systemd sleep inhibitor.

Checked live against a local test server: the window opened on connect,
Get list sent LIST and filled the list with colored topics and modes.
Checks: fmt, clippy (Linux and Windows, with and without `bench`), 184
tests, `cargo audit`, `cargo deny check`.

## Eleventh round: window place, spell check as you type, nick format (2026-10-14)

FEATURES.md had no open rows left, so the options were gone through again
for gaps; the new rows are under "Round 11".

- **Window place.** The main window opens where it was closed, at the same
  size, maximized if it was. A saved place far off any screen is ignored.
  Under WSLg the window manager reports a frame that differs from the one it
  adds when placing a window, so saving the frame's corner made the window
  creep up and left by 32 pixels on every start. The content corner is
  saved instead, less the offset measured between the place asked for and
  where the content landed. Checked over two restarts: 120,70 both times.
- **Spell check.** With a dictionary file set, the input line underlines
  unknown words once they are finished; right-clicking one offers close
  dictionary words (two edits at most) and "Add to the dictionary", which
  appends to the file. Nicknames in the channel, its name, and your nick
  are never marked, in the input line or the transcript.
- **Nick format.** The speaker's `@`/`+` can be shown before the nick, and
  the text around nicks can be chosen (Options, Display).
- **Scripts on disk.** Script files load from and save to a path; a loaded
  script waits until it is ticked.
- **Web search** for the right-clicked word, with a search address of your
  choice (Options, Messages).
- **Mouse side buttons** step back and forward through the windows.
- **Fixes found on the way.** On Windows, web addresses were opened through
  `cmd /C start`, where an `&` or `%` in the address is read by the shell;
  they now go to the URL handler directly. Long menu rows no longer wrap;
  menus widen to fit.

Checked live against a local test server: `<+carol>` with the rank on,
nicks not underlined, the input-line menu replacing "wrold" with "world"
and "Frend" with "Friends", "helo" added to the word file, the side
buttons, the Options rows, script Load and Save to disk, the maximized
restore. Checks: fmt, clippy (Linux and Windows, with and without
`bench`), 193 tests, `cargo audit`, `cargo deny check`.

## Twelfth round: other languages (2026-10-14)

- **Spell check in any alphabet.** Words are runs of letters in any
  script, with combining marks (accents written separately, Indian vowel
  signs and viramas) kept inside the word; comparison is Unicode
  lowercase, and the typographic apostrophe counts as `'`. Chinese,
  Japanese, Thai, and similar text is left alone: without spaces a word
  list cannot tell where words end. Hunspell `.dic` files load (the count
  line and `/FLAGS` are dropped; affix rules are not applied), and word
  lists in an older code page are read as Windows-1252. Fixed on the way:
  suggestions capitalised the first byte, which broke on "é".
- **Fonts for other scripts** (`src/fonts.rs`). When Chinese, Japanese,
  Korean, Thai, or Indian-script text first appears (in a line, a nick, a
  channel list entry, or the input line), a font for it is looked for on
  the computer (fontconfig on Linux, then the Windows fonts folder, which
  WSL can see; known files on Windows and macOS) and read on a background
  thread, then added after DejaVu. Fonts are installed in a fixed order,
  so a character two fonts share always comes from the same one. If none
  is found, the status window says so once. Five such fonts cost about
  105 MB in the debug build, paid only when that text shows up.
- **Translations** (`src/i18n.rs`, `lang/`). The English text is the key:
  `tr("Quit")`, `trf("{0} joins {1}", ...)` with numbered places a
  translation may reorder, `tr_key` to mark text kept in constants, and
  `plural(n, "{0} user", "{0} users")` (which also fixes "2 matchs"). 792
  texts cover the menus, toolbar, status bar, every Options page, the
  dialogs, and the transcript's event lines; output of typed commands is
  still English. German, Spanish, French, and Brazilian Portuguese are
  built in (machine-assisted; native speakers should review them); a file
  in the settings folder adds or overrides a language without rebuilding
  (`lang/README.md`). The language follows the computer unless chosen in
  Options, Display, and switches at once. Background threads have no
  language, so text they build is translated before they start. A test
  scans the sources and fails if a built-in catalog misses a text, keeps
  an unused one, or drops a `{0}`.
- **Ambiguous texts split.** Where one English word meant two things, the
  texts were separated: File, Exit (leaving OpenIRC) versus Quit (leaving
  IRC); sound events are "Someone joins" and so on, so "Join" stays a verb;
  the status bar says "Connecting…" apart from the Connecting section.
- **Windows settings folder.** With no HOME (the normal case on Windows),
  settings went to a relative `openirc-config` folder wherever OpenIRC was
  started; they now go to `%APPDATA%\openirc`, unless a HOME-based folder
  already holds settings.
- **Script editor notes** are marked as failures with a flag instead of by
  their English wording.

Checked live: a German, Spanish, French, and Portuguese window (menus,
Options pages, the channels list, the Join prompt, event lines), the live
switch from German to French in Options, Chinese, Japanese, Korean, Thai,
Hindi, and Russian lines drawn with fonts from the Windows folder, and
accented and Cyrillic words checked against a mixed word list. A pseudo
language that wraps every text in «» was used to find texts not yet
translated. Checks: fmt, clippy (Linux and Windows, with and without
`bench`), 201 tests, `cargo audit`, `cargo deny check`.

## Thirteenth round: second peer review (2026-10-14)

A second reviewer read the whole snapshot (`openirc-review-2026-10-14.zip`)
without a Rust toolchain and reported 34 findings (9 high, 23 medium, 2 low)
plus low-priority notes. Each was checked against the code before it was
changed; every one held. Each fix has a regression test that drives the
real path (the event, the dialog, the wire) rather than the helper alone,
and several earlier claims in this file were narrower than they read:

- B3 (remote command injection) covered direct event expansion but not an
  event calling a quick alias, which filled `$1-` and then split the result
  on `|` (F02).
- B6 (damaged settings kept) did not stop a save after the backup itself
  failed, and two programs saving at once shared one temporary file (F25,
  F26).
- B16 (SASL) did not split a PLAIN answer of 400 bytes or more (F22).
- B18 (downloads never overwrite) checked for a free name without
  reserving it (F12).

| ID | Finding | Fix | Test |
|----|---------|-----|------|
| F01 | `http://é` panicked (byte slice) | prefix compared with `get(..8)`; the whole source was searched for other unchecked byte slices (all guarded) | `urls::a_letter_after_the_scheme_that_is_not_ascii_is_kept` |
| F02 | quick-alias arguments could add commands | the alias body is split before `$1` is filled; dialog `$did` values the same way | `settings::a_separator_in_alias_arguments_stays_text`, `ui::a_separator_in_remote_text_never_runs_a_second_command_through_an_alias` |
| F03 | a new owner of a nick got the old owner's trusted address | every line carries its sender's current `user@host` (`Effect::sender`), which replaces the cached one or clears it when missing; QUIT drops it, NICK moves it, a reconnect clears the server's | `ui::a_new_owner_of_a_nick_does_not_inherit_the_old_address` |
| F04 | any reply with a passive token took the upload | the reply must come from the offered nick, on the same server, with a port; offers expire after 30 minutes; tokens come from a randomly keyed hasher | `transfers::a_passive_reply_counts_only_from_the_person_and_server_it_went_to`, `dcc_send::passive_tokens_do_not_follow_each_other` |
| F05 | an ACCEPT at another position truncated the file | the answer must match nick, server, file, port and the exact offset asked for; the file must still be that long (`open_resume`), else nothing is written | `transfers::a_resume_answer_at_another_position_leaves_the_file_alone`, `dcc_send::resume_only_continues_a_file_of_the_expected_length` |
| F06 | the Messages window answered the last person on any server | the person to answer is kept per server and the reply goes to that server | `ui::the_messages_window_answers_the_person_on_its_own_server` |
| F07 | auto-op timers and delayed joins ran on the server in front | every timer keeps its server, runs there, and ends when that server is closed; `/ban -u` names the channel at once | `ui::an_auto_op_timer_runs_on_the_server_it_was_set_on` |
| F08 | Channel Central sent to the server in front | its actions run on its own server (`central_actions`); the Commands-menu prompts remember their server too | `ui::channel_central_changes_go_to_its_own_server` |
| F09 | Ident read without limit | 512-byte queries, 10 seconds per connection in all, 8 at once; stopping Ident ends them | `ident::an_endless_query_is_cut_off` |
| F10 | two script panics on non-ASCII text | `split_top` compares bytes; `$replace` matches letter by letter on the original text | `slang::letters_beyond_ascii_never_break_conditions_or_replace` |
| F11 | nested loops multiplied work | one budget of 20,000 steps per event or alias across loops, aliases and text built; values capped at 16 KiB, variables at 4 MiB, 200 commands per run; a typed line, event or timer leads to at most 2,000 commands; `$read` reads only plain files up to 1 MiB | `slang::nested_loops_and_growing_values_share_one_budget`, `slang::read_only_reads_plain_files`, `ui::aliases_that_fan_out_stop_at_the_command_budget` |
| F12 | two same-name downloads could share a file | New and Rename create the file when accepted (`create_new`, next free name); an empty one is removed if nothing arrives | `dcc_send::two_new_downloads_of_one_name_get_their_own_files` |
| F13 | short or long transfers showed Done | reads stop at the advertised size; an early end is an error that keeps the partial file; a receiver that hangs up before confirming every byte fails the send | `dcc_send::a_short_or_long_transfer_is_not_reported_as_complete` |
| F14 | Cancel could not stop a blocked send | sends and receives wait in half-second steps, checking Cancel, and fail after 60 seconds without progress; waiting for the other side to connect is cancellable | `dcc_send::cancel_ends_a_send_to_a_receiver_that_never_reads`, `dcc_send::cancel_ends_the_wait_for_a_receiver` |
| F15 | resume was receive-only | a RESUME for a file we offer is answered with ACCEPT and the send starts there; names with spaces are quoted; a file offered without listening (passive) cannot be resumed, and says so | `transfers::the_receiver_of_our_file_can_resume_it` |
| F16 | one own nick for every server | `me()` is the nick on the server being handled; our NICK updates that server's | `ui::each_server_keeps_its_own_nick` |
| F17 | unbounded queues and drains | 500 messages per server per frame (DCC chats: 500 for all of them together, shared out in turn, since round 18); DCC chat's queue holds 1,000 lines and then slows the peer through TCP | (frame budget; bench) |
| F18 | a local address of the other family was skipped silently | without fallback that address is not used; with fallback it connects unbound | `net::a_local_address_of_the_other_family_is_never_skipped_silently` |
| F19 | HTTP CONNECT lost bytes after the header | the reply is read byte by byte to the blank line, the status code is parsed, and headers are capped at 8 KiB | `socks::bytes_right_after_the_proxy_reply_are_kept`, `socks::only_a_200_status_opens_the_tunnel` |
| F20 | `dcc` or `action` in lower case escaped typed ignores | the CTCP word is read once, in capitals, for both | `pump::ctcp_case_never_slips_past_a_typed_ignore` |
| F21 | network ignores applied everywhere | the NETWORK= name is kept for the check; a rule for named networks waits until the name is known (auto-op rules too) | `pump::a_network_ignore_waits_for_the_network_name` |
| F22 | SASL answers of 400+ bytes | 400-byte AUTHENTICATE pieces, and `+` after an exact multiple | `pump::long_sasl_answers_are_sent_in_400_byte_pieces` |
| F23 | passwords in history, debug and logs | the history check reads every pasted line, any prefix character, `/raw` and `/quote`, `/server` passwords, `/join` keys and `/mode +k`; Debug hides JOIN keys and `+k` both ways; a services password shows as `****` in the window and log; new logs are private (0600) | `input` tests, `connection` tests, `pump::a_services_password_is_sent_but_shown_as_stars` |
| F24 | a log tail cut inside a letter became mojibake | the cut line is dropped as bytes, then each line is decoded alone | `settings::a_log_tail_cut_inside_a_letter_stays_utf8` |
| F25 | two saves at once mixed files | each save writes its own temporary file, made fresh | `settings::saves_at_the_same_time_each_leave_a_whole_file` |
| F26 | a failed backup still allowed the save | the settings are not saved over the file until a backup can be made | `settings::damaged_settings_are_never_saved_over_without_a_backup` |
| F27 | `#a/b` and `#a_b` shared a log | file names escape other characters as `%XX` (and `%` itself), plus edge dots and Windows device names; the log browser shows the real name | `settings::different_names_never_share_a_log_file` |
| F28 | windows ignored the server's case mapping | every server's CASEMAPPING is kept and the window compares nicks and channels with it; the folds were also corrected (`rfc1459` adds `~^`, `strict-rfc1459` adds `\|`) | `ui::windows_follow_the_servers_case_mapping`, `state` tests |
| F29 | a huge server-time year overflowed | fixed-width digits only | `clock::malformed_server_times_are_refused` |
| F30 | dictionary encodings were mixed | a `.dic` is read in its `.aff` `SET` encoding; other lists line by line; added words go to `<list>.added.txt` in UTF-8 | `spell::dictionaries_keep_their_encodings_and_added_words_stay_apart` |
| F31 | Windows opened paths through `cmd` | `ShellExecuteW` with a literal UTF-16 path for web addresses, folders and files (also `/run`) | Windows type-check only |
| F32 | every Indian script asked for a Hindi font | one script each (Devanagari, Bengali, Gurmukhi, Gujarati, Odia, Tamil, Telugu, Kannada, Malayalam, Sinhala) | `fonts::scripts_are_told_apart` |
| F33 | `{1}` in a nick was filled again | placeholders are filled in one pass | `i18n::placeholder_shaped_values_are_put_in_as_they_are` |
| F34 | faces of one collection collided | a font is its file and face; a font two scripts share is ordered by the earlier script | `ui::faces_of_one_collection_are_kept_apart_in_a_fixed_order` |

The low-priority notes were done as well:

- **Spell-check exclusions.** The cache of nicks to skip is keyed by the list's contents, not its length.
- **Nick list.** Rows out of sight are not laid out: a 2,000-person channel frame went from 3.1 ms to 0.65 ms.
- **Channels list.** The filtered and sorted rows are cached until the list or the filter changes: 20,000 channels went from 1.08 ms to 0.30 ms a frame.
- **Language list.** It is read from disk at most every 5 seconds.
- **Log browser.** Searching runs on its own thread, the shown log lays out only the rows in sight, and the matching lines are cached.
- **Flood table.** It forgets quiet senders past 1,024 entries.
- **Switches in settings.txt.** A value that is not `true` or `false` is reported and the default is kept.
- **Client certificate.** It is made private even when the file already existed.
- **Log write failures.** These are reported, once a minute at most.
- **Untranslated window texts.** The ones the reviewer cited, and the automatic Disconnected notes, now pass through translation.
- **Locale.** `LC_ALL=C` means English.
- **Language switching.** Switching back and forth reuses each catalog instead of leaking it.
- **Tray.** Minimize to tray hides the window only when a tray has really taken the icon.

Not changed:

- **Window-thread script work.** Scripts still run on the window's thread, but now within a fixed budget.
- **Ambient routing.** The window still routes commands through `routing` and `scope()`. Timers, dialogs, prompts and replies now carry their server explicitly; replacing the whole mechanism is a larger refactor.
- **Old log names.** Log files of names with characters other than letters, digits and `#&+!-._` start under the new escaped name.

New: `scripts/check.sh` and the CI workflow (see Checks to run).

The bench example takes `--save`, `--compare` and `--tolerance`. It has two new frame benchmarks, for a 2,000-person channel and a 20,000-entry channels list.

Checks:
- fmt;
- clippy (Linux and the Windows target, with and without `bench`);
- 241 tests;
- `cargo audit` and `cargo deny check`;
- benchmarks.

The Windows tests themselves were not run: the toolchain on that machine is older than egui 0.36 needs. The CI Windows job runs them.

## Fourteenth round: follow-up review of round 13 (2026-10-15)

The reviewer checked the round-13 archive against the original findings,
again without a Rust toolchain, and reported 21 follow-up findings (1 high,
17 medium, 3 low), a list of tests that did not prove what they claimed,
and smaller notes. Every finding held when checked against the code. The
round-13 summary above claimed more than was true in these places:

- "every fix drives the real path": the Ident, timer, concurrent-save,
  SASL, dictionary, Indic-font, TTC and random-token tests did not (see the
  second table below);
- F07, F08, F11, F13, F14, F15, F17, F20, F23 and F28 were only partly
  fixed (R01–R13, R18);
- "the low-priority notes were done as well": the flood table still kept
  held-back senders (R11), saved records still read damaged switches as
  `false` (R15), the new channels-list cache could show stale rows (R14),
  and several cited window texts were still untranslated (R21).

Each fix below was checked by putting the old code back and running its
test: the test failed every time (18 cases, including two tests that first
let the old code through and were then strengthened).

| ID | Finding | Fix | Test |
|----|---------|-----|------|
| R01 | a timer's `/disconnect` or `/debug` acted on the server in front, and `/debug` cleared the routing for the rest of an alias | `/disconnect`, `/quit`, `/debug`, `/list` and `/server` act on the server being handled (`scope()`); every place that switched routing for a moment now restores it (`on_server`) | `ui::every_command_a_timer_runs_stays_on_the_server_it_was_set_on` |
| R02 | `$replace` built its whole result before the 16 KiB cut | each replacement stops at the limit; comparing work counts against the 20,000 steps (one per 1,024 characters); text being built stops at the limit and the rest is not worked out; a cut gives its memory back | `slang::replacing_never_builds_more_than_one_value`, `slang::comparing_long_text_uses_up_the_run` |
| R03 | passwords still reached history, the transcript and Debug | the history check uses the `/server` parser itself, the configured command prefix, `/key`, any mode change with `k`, and the user's aliases (two levels); `/cs`, `/os`… and `/notice` to services show `****`; Debug finds a key with the server's own CHANMODES and PREFIX, and hides every argument from a letter it does not know | `ui::saved_history_leaves_out_passwords_however_they_are_typed`, `input::secrets_are_not_kept_and_history_restores`, `pump::a_services_password_is_sent_but_shown_as_stars`, `connection::debug_finds_keys_with_the_servers_own_mode_list` |
| R04 | answering a file offer used the server in front | offers in the prompt and in the DCC window keep the server they came on, and the answer and the transfer run there | `transfers::an_offer_is_answered_on_the_server_it_came_from` |
| R05 | the wait for the last acknowledgement ignored Cancel | acknowledgements are read as they come; the end of a send checks Cancel every 50 ms and fails after 60 s without progress | `dcc_send::cancel_ends_the_wait_for_the_last_acknowledgement` |
| R06 | an old 32-bit acknowledgement could confirm a 4 GiB+ send | acknowledgements are unwrapped in order into one position that never goes back or past what was written, and the full position is compared with the full size | `dcc_send::acknowledgements_unwrap_past_four_gibibytes_and_never_go_back` |
| R07 | an empty file always failed on the sending side | nothing to confirm is confirmed at once | `dcc_send::an_empty_file_is_sent_and_saved_on_both_sides` |
| R08 | the IRC queue to the window was unbounded | at most 5,000 messages wait; past that the server's thread waits and stops reading, so a flood is held back by the connection (the window takes messages even while hidden, about every 100 ms) | `ui::a_flood_waits_in_the_connection_instead_of_in_memory` (a real connection and 8 MB of lines) |
| R09 | a formatting code before `DCC` or `ACTION` slipped past a typed ignore once colors were stripped | stripping changes only what is shown; the CTCP command read from the line as it came decides both the ignore and the handling | `pump::color_stripping_never_turns_a_message_into_one_an_ignore_would_stop` |
| R10 | connecting to a proxy ignored the local address setting | the proxy is dialed like a server (`dial_with`), with the same no-fallback rule and a connect time limit | `socks::the_proxy_is_reached_only_from_the_chosen_local_address` |
| R11 | held-back senders were never forgotten, and a big table was scanned on every message | quiet senders are forgotten whether held or not; the table is cleared out when it has doubled or once a window has passed | `flood::held_back_senders_are_forgotten_once_quiet`, `flood::clearing_out_costs_little_with_many_active_senders` |
| R12 | `/scon`, Central and other lookups compared names the front server's way | window lookups use their own server's case mapping; `/scon` and Central set the whole context; changing the window in front changes it too | `ui::names_compare_the_way_their_own_server_does`, `ui::channel_central_reads_and_changes_its_own_channel` |
| R13 | a Quit confirmed for one server marked another as hung up | `/quit` marks the server it goes to; the prompt no longer marks one itself | `ui::a_quit_on_one_server_leaves_the_other_ready_to_reconnect` |
| R14 | the channels-list cache could keep stale rows after a refresh | the list counts every change (and a change of server) and the rows are made again after any of them | `channel::a_refreshed_list_is_filtered_and_sorted_again` |
| R15 | a damaged switch in a saved server, favorite, highlight, auto-mode or script record was read as `false` | one strict `true`/`false` reader for all of them; a damaged record is reported and the original file is backed up | `settings::a_damaged_switch_inside_a_record_is_reported_not_read_as_false` |
| R16 | the benchmark comparison passed when it could not compare | a missing, empty or damaged baseline, a benchmark missing on either side, a bad tolerance or a missing option value stops the run (status 2) | `benchcmp` tests |
| R17 | the window stayed hidden when the tray went away | it comes back to the taskbar as soon as the icon is no longer in a tray, with a note | `ui::a_hidden_window_comes_back_when_the_tray_goes_away` (state only; not tried with a real tray) |
| R18 | a resume request for an old file on the same port was accepted | the request must name the file offered on that port (or the common stand-in `file.ext`), and is agreed to only before the receiver connects; after that the start is fixed | `transfers::a_resume_request_must_name_the_file_on_that_port_and_come_in_time`, `dcc_send::a_resume_is_agreed_to_only_before_the_receiver_connects` |
| R19 | benchmark saving failed with `CARGO_TARGET_DIR` set elsewhere | `check.sh` creates `target/` first | `check.sh` run with stand-in `cargo` and a custom target folder |
| R20 | `--save-bench` could not accept a slowdown | `--save-bench` skips the old comparison and keeps the new baseline; every other check still runs | `check.sh` run with stand-in `cargo` |
| R21 | cited window texts were untranslated | translated (color tool tips, highlight and invite toasts, dialog editor buttons, profiler, log search, DCC and script notes, the TLS upgrade note); a new test reads the window code for widgets, tool tips, notices and toasts given plain words | `i18n::widget_calls_never_take_plain_words` |

Tests that did not prove their claim:

| Test | Now |
|------|-----|
| Ident close | the reply must end in a clean hang-up in time; the longest query is answered and one byte more is not; a byte-at-a-time sender is cut off at the deadline; a ninth connection is turned away while eight wait; stopping ends the waiting ones (`ident` tests) |
| closed-server timer | uses a valid `/timer 1 1` and checks the timer was set before its server goes |
| concurrent saves | every save must succeed, and the file is always exactly one of the saves |
| ingress | R08's test runs the real network thread and queue |
| SASL framing | goes through `CAP` and `AUTHENTICATE +` for 1, 2 and 3 pieces |
| dictionary additions | calls the window's own Add, checks the list's bytes are untouched, and reads both back in a new window (`ui::added_words_go_beside_the_list_and_come_back_after_a_restart`) |
| Indic fonts | each script asks fontconfig for its own language (Tamil `ta`), and the regular sans face is picked from a listing (`fonts` tests) |
| TTC faces | a real two-face collection is built in the test and face 1 is the one drawn (`ui::the_chosen_face_of_a_real_collection_is_the_one_drawn`); a face two scripts share moves to the earlier script's place in either load order |
| DCC and Central | R04 and R12's tests change the window in front between the offer or form and the answer |
| random tokens | the shape is checked for edge values, and spacing with a fixed hasher key; no collision-free claim |

Smaller notes:

- **Ident.** A query is now at most 512 bytes, line end included.
- **Server time.** A fraction must be digits, and the day must exist in its month (February 29 only in leap years).
- **Dictionaries.** A `SET` after a byte order mark is found. An encoding OpenIRC cannot read turns spell checking off with a note, and so does a list that cannot be read.
- **Acknowledgement policy.** It is now one rule, written at `send_over`: the last byte must be confirmed. A receiver that never sends any acknowledgement counts as done when it closes the connection.
- **32-bit sizes.** The amount left is compared as `u64` before the cast.
- **Idle DCC chat.** Closing the window hangs up even on a quiet peer (`dcc::closing_a_chat_hangs_up_on_a_quiet_peer`).
- **Stale docs.** The stale comments on `safe_name` and the B6 note on `settings.txt.tmp` are corrected.
- **Existing logs.** A log others could read is made private when the next line is written.
- **Windows open.** The `ShellExecuteW` thread sets up COM (single-threaded, no OLE1 DDE) first, as the shell documents. This is only type-checked; it has not been run on Windows.
- **Proxy handshakes.** One deadline (20 s) now covers the whole handshake, not each read (`socks::a_proxy_that_answers_a_byte_at_a_time_still_ends_in_time`). When the deadline runs out, the user sees "the proxy took too long to answer". The first version of this showed "Resource temporarily unavailable" on Linux, where a timed-out read says "would block"; running the checks from the unpacked archive caught it.
- **Not changed: UI performance.** Nick prefixes are still built for every row, and the spell-skip hash still reads every nick. Neither showed up in the measurements.

**An incident in this round.** The first mutation run put the old `$replace` back and ran its test, which then asked for about 1 TB. That test process grew to 10.8 GB, and Linux killed it along with the other sessions on the machine. Two changes followed:

- The test now uses sizes for which code without the limit builds only a few MB.
- `scripts/check.sh` now runs the tests and benchmarks under a memory cap (`TEST_MEMORY_MAX`, default 2G).

The normal suite peaks at about 215 MB.

**Checked in the window** against two local test servers:

- R01: a timer `/disconnect` set on server A while B's window was in front closed A only.
- R03: `/cs IDENTIFY #room swordfish` reached the server in full, while the window and `#room.log` show `IDENTIFY ****`.
- R08: a 23.6 MB flood of 200,000 lines raised the program's memory by 13 MB. Every line arrived, and the window kept responding.
- R09: with `/ignore -dk alice`, alice's bold-prefixed DCC CHAT showed only as a CTCP note, with no offer. Her plain one was ignored, and bob's appeared.
- R14: a refresh with the same names in the same order but swapped user counts reordered the list.

That run also showed that server A's own notes, such as `disconnected`, were written into B's window when B was in front. Lines a server sends now go to that server's status window unless one of its own windows is in front (`ui::a_servers_own_notes_stay_in_its_windows`). Typed commands and timers still answer in the window in front.

Checks:
- fmt;
- clippy (Linux and the Windows target, with and without `bench`);
- 278 tests (peak memory about 215 MB);
- `cargo audit` (1,329 advisories, 438 crates) and `cargo deny check`;
- benchmarks within the 50% limit. The largest moves between runs were up to +20%, on benchmarks timed in a single run or pulled up by a few slow frames; their medians did not move.

The Windows tests were still not run natively.

## Fifteenth round: review of round 14 (2026-10-15)

The reviewer checked the round-14 archive (`openirc-review-fixes-round14-2026-10-15.zip`, SHA-256 `35a112fb…dd15`), again without a Rust toolchain. They found three medium findings and four smaller notes. All held. The round-14 archive is unchanged; this round is a separate archive.

| ID | Finding | Fix | Test |
|----|---------|-----|------|
| R14-F1 | a typed `/signin swordfish` for a script alias `msg NickServ IDENTIFY $1`, or a quick alias three levels deep, could be saved in the input history | Whether an entry holds a password is now decided by what it really did. Every line sent while a typed entry runs (through quick aliases, script aliases, timers and pasted blocks) is checked, and a hit marks that history entry. The mark stays with the entry, so changing an alias later cannot let it be saved. The reading of the typed text itself follows quick aliases as deep as commands may nest, through each alias once. | `ui::a_line_that_sent_a_password_through_a_script_is_never_saved`, `ui::saved_history_leaves_out_passwords_however_they_are_typed` |
| R14-F2 | a timer set on server A could send `/me` or plain text to the DCC chat or Messages window in front when it fired | A timer remembers the window `/timer` was typed in. Text without a command, and `/me`, go there: to the channel or query by name (`/msg`, `/describe`), to the DCC chat it was set in, or to the Messages window's last sender. The window in front is used only for what the user typed in it. An event, `/scon` or anything else already bound to a server never borrows the focused chat. Also fixed: a timer's text used to go to whichever channel of that server was picked last. | `ui::a_timer_speaks_where_it_was_set_not_where_the_user_looks` |
| R14-F3 | `$rand(-9223372036854775808,9223372036854775807)` divided by zero, so it panicked in debug and release builds alike | the range is counted in 128-bit numbers, so all 2^64 values fit | `slang::random_numbers_cover_any_range_without_overflow` (passes in debug and release builds) |

Smaller notes:

- **Ingress test.** It now checks that every numbered line arrives exactly once and in order. Each effect is read as the window reads it: its events, else its lines. A separate test fills the queue to its limit and checks that one more message is refused (`ui::the_queue_to_the_window_holds_the_limit_and_no_more`).
- **Translation scanner.** The test is renamed `i18n::widget_calls_never_take_plain_words`. Its comment now says it sees only text written straight into a widget call. The texts found by hand that reached widgets through variables are translated: `Done` in Transfers, the dialog editor's `Applied.`, the server list's `Saved.` and `Deleted.`, and the popup menu names. Special window names (`Debug`, `Server notices`) stay English because the program finds those windows by name. Command output stays English, as FEATURES.md already says.
- **Memory fallback.** `check.sh` and the Checks section now say exactly what each cap is:
  - with systemd: one scope, at most `TEST_MEMORY_MAX` in all, no swap;
  - without: four times that much address space per process, which is looser.

Each fix was checked by putting the old code back. Eight of nine cases failed their test as they should. The ninth removed the alias cycle check, which only saves work: the depth limit already ends a loop, so nothing visible changes. The mutation script now touches each file it restores. Without that, cargo kept running the mutated build: the restored file's timestamp was older, so cargo thought nothing had changed.

**Checked in the window.** A timer `/me timer-A-only`, set in `#room`, fired while a real DCC chat with a test peer was in front. The server got `PRIVMSG #room :\x01ACTION timer-A-only\x01`, and the DCC peer received nothing. The history fix could not be tried in the window: WSLg gives the window no keyboard, and only typed lines enter the history. Its test runs the same Enter path (`enter_line`).

Checks:
- fmt;
- clippy (Linux and the Windows target, with and without `bench`);
- 282 tests;
- `cargo audit` (1,329 advisories, 438 crates) and `cargo deny check`;
- benchmarks, all within 7% of the baseline.

The Windows tests were still not run natively; that build is only type-checked here.

## Sixteenth round: review of round 15 (2026-10-15)

The reviewer checked the round-15 archive (SHA-256 `495a5784…2e4c`), again without a Rust toolchain. They found seven medium findings: three on history, three on timer destinations, and one timer-range panic. There were also two small notes. All held.

Round 15 had closed each reported case, but these findings went past those cases, so this round changes the designs underneath instead.

**History.**
- **Ids.** Each history entry now has an id that never changes or comes back. While a typed line runs, that id goes with everything it starts, including timers it sets and timers those timers set.
- **What is checked.** Every command is read before it runs, after the command prefix is applied and before aliases expand it. That covers local commands such as `/server -p`, every alias step, and every line sent.
- **Marking.** A command that may hold a password marks the entry it came from, and saved history is written again at once without it.
- **Not saved yet.** An entry is never saved while a timer it set is still waiting, or while its paste waits for confirmation.
- **Save order.** The periodic save now happens after the typed line has run, never before.

**Destinations.** Every window has an id that is never reused. A timer keeps the id of the window it was set in. One resolver now decides where words with no target of their own go: plain text, `/say` and `/me`.
- **Typed:** the window typed in.
- **A timer:** its window, by name: a channel or query (`/msg`, `/describe`), its DCC chat, or the Messages window's last sender.
- **Window gone:** if that window is closed, or its DCC chat has ended, nothing is sent. A note says what was dropped, even if a window of the same name has opened since.
- **An event or `/scon`:** the server's own target, never the focused chat.
- **Script aliases:** `$chan` (and the nick list) comes from the same window: the timer's window, the window typed in, or none for an event.
- **`/ban -u`:** names its channel the same way.

**Time.** Timer and `/ban -u` deadlines are added with checks and limited to a year. A longer interval is refused with a note, and a repeat past what the clock can hold ends the timer.

| ID | Finding | Test |
|----|---------|------|
| R15-F1 | the periodic save ran before a typed line, so a script-alias password could be written | `ui::history_is_never_saved_before_a_line_is_known`: script-alias logins at the 10th Enter and at the 200-entry cap, with every snapshot recorded and checked; a saved line marked later disappears at once |
| R15-F2 | a password sent later by a timer's alias was not traced to the line that set the timer | `ui::a_timer_line_waits_and_takes_the_blame_for_what_it_sends_later`: a timer running a script alias, a quick alias, a timer inside a timer, and an alias changed before firing, with unrelated typing in between |
| R15-F3 | a script alias calling `/server -p` sent the password without the history noticing | `ui::an_alias_that_logs_in_to_a_server_is_never_saved`: a local test server receives `PASS swordfish`, and the history leaves the line out |
| R15-F4 | a timer whose window had closed spoke in the server's current channel | `ui::a_timer_whose_window_closed_sends_nothing`: a closed DCC chat, a closed query, a window of the same name opened again, and a chat that ended with its window still open |
| R15-F5 | `/me` from the Messages window, and `/say`, ignored the timer's window | `ui::plain_words_say_and_me_go_where_the_timer_was_set`: channel, query, Messages and DCC origins, each with plain text, `/say` and `/me`, while another channel and then another network are in front |
| R15-F6 | a script alias run by a timer took `$chan` from the focused window | `ui::a_timer_runs_a_script_alias_for_the_window_it_was_set_in`: with another channel, another network and a DCC chat in front |
| R15-F7 | `/timer 18446744073709551615 1 …` panicked | `ui::a_timer_too_far_ahead_is_refused_not_a_crash`: also covers a year and one second, rescheduling past the clock, and `/ban -u` |

Smaller notes:
- The ingress test now reads on until the server hangs up and checks that nothing numbered comes after the last line.
- The doc comment for `random` is back on `random`.

**Mutation checks.** I put the old code back for each fix (11 cases, each capped at 1 GB). The first run let three through:
- the save order and the re-save were hidden by each other, because the test looked only after the re-save;
- a DCC chat that ended while its window stayed open was not tested.

The test now records every snapshot written and covers both cases, and all 11 are caught.

**Checked in the window** against a local test server:
- A `/timer 6 1 /say private-note` set in `#room` fired while `#public` was in front. The server got `PRIVMSG #room :private-note`.
- The same timer, set in `#public` and fired after `#public` was closed, sent nothing. Status said "The window this was set in is closed, so “private-note” was not sent."

The history fixes could not be tried in the window, because WSLg gives it no keyboard. Their tests use the same Enter path.

Checks:
- fmt;
- clippy (Linux and the Windows target, with and without `bench`);
- 289 tests;
- `cargo audit` (1,329 advisories, 438 crates) and `cargo deny check`;
- benchmarks within the 50% limit.

Benchmark moves were up to +20%, all on code this round did not touch. The largest, on the frozen old-transcript copy, came back at +8% when run again: machine noise.

The Windows tests were still not run natively; that build is only type-checked here.

## Seventeenth round: review of round 16 (2026-10-15)

The reviewer checked the round-16 archive (SHA-256 `6f0bffa0…6e38`), again without a Rust toolchain. They found four medium findings and two notes on tests. All held when checked against the code.

While fixing them I found one more problem of the same kind:
- **DCC lines to the wrong window.** Lines arriving on a DCC chat were matched to their window by name, using whichever server was in front. With another server in front, they could land in the wrong window, or in none.

The round-16 archive is unchanged; this round is a separate one.

| ID | Finding | Fix | Test |
|----|---------|-----|------|
| R16-F1 | timers due in the same tick left the protected set before they ran, so the first one's save could write the second one's password | due timers' lines stay protected until each has run (`running`), and so does the line running now (`provenance`) | `ui::a_timer_line_waits_and_takes_the_blame_for_what_it_sends_later` now checks every snapshot written, and fires two script-alias logins plus a nested timer in one tick |
| R16-F2 | `/scon -a` inside a timer spoke twice into the timer's window instead of once on each server | `/scon` sets the timer's window aside while it runs, so each server uses its own target; the history entry still answers for what it sends | `ui::scon_in_a_timer_speaks_on_every_server`: `/say`, `/me`, a script alias, a timer from a DCC chat, and a timer set inside `/scon` inside a timer, with exactly one line per server and none to the chat |
| R16-F3 | two DCC chats with the same name chose their connection by name | each connection belongs to its window's id: sending, incoming lines, closing, and a new chat in an old window all use that id | `ui::two_chats_with_the_same_name_keep_their_own_connections`: two `DCC bob` windows on two servers, each with its own connection. Lines typed or timed in B reach only B's peer; each peer's lines land in its own window; closing B leaves A working |
| R16-F4 | a chat whose peer hung up still looked connected, and a line sent to it was shown as sent | a hang-up is noticed when the window is polled: the connection is let go and the window says "The DCC chat ended". A send that fails is not shown as sent; Status names the line that was not sent. | `ui::a_dcc_chat_that_ends_sends_nothing_and_says_so`, through `attach_dcc` and a real hang-up (the peer's channels dropped), not a cleared table. It covers both orders: noticed before the timer fires, and the send failing first. |

The tests now attach DCC chats through the real `attach_dcc` (`fake_dcc`) instead of filling the connection table by hand.

Notes on tests:
- The ingress test fails on a timeout. Only the server's hang-up, or its thread ending, counts as the end of the stream.
- The destination matrix sets fresh timers for each window in front (another channel of the same server, then another network) and checks both.
- The closed-window test checks the note for its own line, not any earlier one.

**Mutation checks.** I put the old code back for each fix: 7 cases, each capped at 1 GB. All 7 were caught.

**Checked in the window** against two local test servers and a test DCC peer:
- A timer `/scon -a /say scoped-note`, set in `#room` on the first server, reached each server exactly once.
- A real DCC chat whose peer sent a line and then closed its socket showed the line, then "The DCC chat ended."

Checks:
- fmt;
- clippy (Linux and the Windows target, with and without `bench`);
- 292 tests;
- `cargo audit` (1,329 advisories, 438 crates) and `cargo deny check`;
- benchmarks within the 50% limit.

The largest benchmark move, +24%, was on the frozen old-transcript copy, which this round did not touch. Run again, it measured +4%.

The Windows tests were still not run natively; that build is only type-checked here.

## Eighteenth round: review of round 17 (2026-10-15)

The reviewer checked the round-17 archive (SHA-256 `ac0a3c35…9ba7`), again without a Rust toolchain. They confirmed all four round-16 fixes and found five medium DCC findings, all about how chat connections are created, replaced, polled and removed. All five held.

DCC chats are now separate connection objects (`DccLink`). Each one has:
- the window it belongs to;
- an id of its own, distinct from the window's id;
- a way out, which can close while what the peer already sent is still read;
- its way in.

The round-17 archive is unchanged; this round is a separate one.

| ID | Finding | Fix | Test |
|----|---------|-----|------|
| R17-F1 | a chat that finished connecting was given to the server in front, and could replace that server's same-named chat | chat offers we receive, and chats we offer, remember their server; a completed chat gets its window on that server, and one whose server was closed meanwhile is hung up with a note | `ui::a_dcc_chat_belongs_to_the_server_it_was_offered_on`, through the real offer, accept and completion paths over TCP with the other server in front: an offer received on server 1 (with a different bob already chatting on server 2) and our own offer from server 1. `ui::a_chat_that_connects_after_its_server_closed_is_hung_up`: the peer sees the hang-up |
| R17-F2 | a new chat in an old window took over that window's timers | a timer set in a DCC chat remembers that chat's id and speaks only into it: not into a later chat in the same window, and not after the chat has ended | `ui::an_old_dcc_timer_does_not_follow_a_replacement_connection` (the reviewer's case, plus a live chat replaced before the timer fires, and a timer set in the new chat reaching it) |
| R17-F3 | when a send failed, the peer's last line, still waiting to be read, was thrown away | a failed send closes only the way out; the peer's lines are still read, in order, and the window then says the chat ended, once | `ui::a_failed_dcc_send_keeps_the_peers_buffered_final_line` (the reviewer's case, plus checks for the order and a single end note) |
| R17-F4 | one busy chat could use the whole frame's budget and hold back every other chat, even hiding that one had hung up | the 500 lines a frame are shared among the chats, a different one first each frame | `ui::a_busy_dcc_chat_does_not_hold_back_the_others`: one chat sends 500 lines before every poll; another's line still shows, and a third's hang-up is still noticed |
| R17-F5 | closing a server removed its DCC windows but left their connections open | closing a server hangs up its chats, and any connection whose window is gone is let go on the next poll | `ui::closing_a_server_hangs_up_its_dcc_chats`: the closed server's peer is disconnected, and the other server's chat still works both ways |

Smaller notes:
- The `destinations` test fixture returns the peer handle instead of leaking it.
- The frame-budget comment, and F17 in the thirteenth-round table, now describe the shared DCC budget.
- Found while checking in the window: the "Connecting DCC chat" note went to the server in front. It now goes to the offer's own server.

**Mutation checks.** I put the old code back for each fix (6 cases, each capped at 1 GB). All 6 were caught.

Two first attempts had to be corrected. One pattern no longer matched after rustfmt. One mutation gave each chat a large share of its own, rather than the old single budget shared by all chats; once it reproduced the old shared budget, its test caught it.

**Checked in the window** against two local test servers and two test DCC peers, both named bob:
- bob on the second server offered a chat and was accepted.
- bob on the first server offered one, accepted with the second server's chat in front. It got its own window under the first server, and the second server's chat stayed connected.
- Closing the first server's status window hung up its bob (the peer saw the end of the stream). The other bob was still connected.

Checks:
- fmt;
- clippy (Linux and the Windows target, with and without `bench`);
- 298 tests;
- `cargo audit` (1,329 advisories, 438 crates) and `cargo deny check`;
- benchmarks, all within 13% of the baseline.

The Windows tests were still not run natively; that build is only type-checked here.

## Nineteenth round: review of round 18 (2026-10-15)

The reviewer checked the round-18 archive (SHA-256 `dff64b92…6c9a`) and confirmed all five round-17 fixes. They found three medium issues and one low. All held. The round-18 archive is unchanged; this round is a separate one.

| ID | Finding | Fix | Test |
|----|---------|-----|------|
| R18-F1 | a new chat replacing an old one in the same window threw away lines the old chat had received but not yet shown | the old chat's waiting lines (at most 1,000, the size of its queue) are shown first, then its end, then the new chat begins | `ui::replacement_keeps_the_old_peers_queued_final_line`: the reviewer's case with the order checked (old line, end, new chat, new line), plus a live chat holding 700 waiting lines with other chats open: all 700 show, once and in order, and the old chat is let go |
| R18-F2 | closing a chat could not stop its thread while it was stuck writing to a peer that does not read, and lines kept piling up behind it | every chat holds a handle (`dcc::Hangup`, a second handle on its socket) that shuts the socket down when the chat is let go, which ends a read or write the thread is stuck in. Writes also time out after 30 seconds, and the queue of lines to the peer holds 256; past that a line is refused with a note | `dcc::hanging_up_ends_a_chat_stuck_writing_to_a_peer_that_never_reads`, over a real TCP connection: the peer never reads, the queue fills (so it is bounded), and after the hang-up the thread ends within 3 seconds rather than when the write gives up |
| R18-F3 | a second chat being set up silently dropped the first | chats we offer and chats we accept are kept in lists, each followed on its own; a chat we offered that nobody accepted is named in its note | `ui::two_chats_set_up_at_once_both_open`: two offers of our own from two servers, answered in the opposite order, and two accepted offers connecting at the same time, all over TCP; every chat opens on its own server |
| R18-F4 | with more than 500 chats, one poll read more than 500 lines | the 500 lines are a true total: each chat's share is cut to what is left, and the next poll starts with the first chat not reached | `ui::the_dcc_budget_holds_however_many_chats_there_are`: with 1, 500 and 501 chats, at most 500 lines a poll and everyone within two polls; and 1,000 chats that keep sending, all reached in two polls |

Smaller notes:
- **Test accepts.** The TCP tests accept their connections with a 5-second limit (`dcc::accept_within`), so a broken connect fails instead of hanging.
- **Ended chat note.** A timer's line for a DCC chat that has ended, or been replaced, now says "The DCC chat this was meant for has ended…". The note saying the window is closed is kept for a window that really is closed.
- **Removed message.** "DCC chat was not accepted." is replaced by "{0} did not accept the DCC chat.", which names the person.

**Mutation checks.** I put the old code back for each fix (8 cases, each capped at 1 GB).
- The resume rule's change first went through: with 501 chats, starting the next poll one place further still reached the last one. The test now also uses 1,000 chats that keep sending, and catches it.
- One change, dropping the cut to what is left, cannot be caught. The loop also stops when nothing is left, so the two guards cover the same case.
- The other six were caught.

**Not checked in the window.** Keeping a chat waiting to connect, or a peer not reading, needs either typing (WSLg gives the window no keyboard) or timing that cannot be held on localhost. The tests for these run over real TCP sockets.

Checks:
- fmt;
- clippy (Linux and the Windows target, with and without `bench`);
- 302 tests;
- `cargo audit` (1,329 advisories, 438 crates) and `cargo deny check`;
- benchmarks within the 50% limit.

One benchmark moved more than the rest: `slang: event, cached parse` showed +15–24% against the saved baseline while the machine was busy. Alternating this build with round 18's, at the same time, put it a steady 7–10% slower (5.8 µs against 5.2–5.6 µs).

`slang.rs` and `bench.rs` are byte-for-byte the same as round 18's, so the code it runs did not change. My best guess is the compiler's placement of that code in the new binary; I have not proven it. It is noted here rather than hidden.

The Windows tests were still not run natively; that build is only type-checked here.

## Twentieth round: review of round 19 (2026-10-15)

The reviewer checked the round-19 archive (SHA-256 `0b8d1893…3f49`) and confirmed all four round-18 fixes. They found two medium issues and made two smaller notes. All held. The round-19 archive is unchanged; this round is a separate one.

| ID | Finding | Fix | Test |
|----|---------|-----|------|
| R19-F1 | replacing a chat showed all of the old chat's waiting lines at once, outside the frame budget; a peer that kept sending could keep that going without end | the old chat is hung up at once, so its peer can add nothing more. Its waiting lines are read by the normal poll, within the budget. The new chat waits, unread, until the old one has ended; then its opening line is shown | `ui::replacing_a_chat_whose_peer_keeps_sending_stays_within_the_budget`: over real TCP, a peer that writes without stopping. The arrival shows nothing from the old chat, and no poll goes over the budget. Another chat is heard in the first poll, the new chat starts within 5 seconds, and the flood lines shown run in order with none repeated. `ui::replacement_keeps_the_old_peers_queued_final_line`: 700 waiting lines arrive over several polls, every one once and in order, then the end, then the new chat |
| R19-F2 | closing a server left its unanswered chat offers listening, holding their ports until the 2-minute wait ran out | a pending offer (`PendingListen`) tells its listener to stop when dropped; closing a server drops that server's offers and connection attempts | `dcc::dropping_an_unanswered_chat_offer_frees_its_port`; `ui::closing_a_server_frees_the_port_of_its_unanswered_chat_offer`: the port can be bound again within 2 seconds, and the other server's chat still works |

**Budget rounds.** Fixing R19-F1 showed that the round-19 budget gave a busy chat only its fixed share, even when the other chats had nothing to send. With five chats open, a 700-line backlog took over four polls. The frame's 500 lines are now shared in rounds:
- first a share for every chat, starting where the last frame stopped;
- then what is left, among the chats that filled their share.

The 500 total and the order of turns are unchanged (`ui::the_dcc_budget_holds_however_many_chats_there_are` still passes).

Smaller notes:
- **Refused when the peer is behind.** A line for a peer that is not keeping up is refused with a note and is not shown as sent. The chat stays, and is used again once the peer reads (`ui::a_peer_that_is_not_keeping_up_gets_no_more_until_it_does`, which ends with a `/me`).
- **Failure notes.** A failed chat or connection attempt is noted in its own server's Status and names the person: "The DCC chat with {0} failed…". If that server is gone, the note goes to the window in front (`ui::a_failed_dcc_chat_is_noted_on_its_own_server_by_name`). The two "DCC connect failed" messages it replaces are gone from the catalogs.

**Mutation checks.** I put the old code back for each fix (6 cases, each capped at 1 GB). Five were caught.
- The unbudgeted drain first went through, because the test counted only lines read by polls. It now also checks that a new chat's arrival shows none of the old chat's lines, and catches it.
- Leaving out the early hang-up was not caught. Dropping the old chat's way out already makes its thread hang up; the early hang-up is a second guard for a thread stuck in a write.

**Not checked in the window.** These paths need typing in the window (WSLg gives it no keyboard). The tests use real sockets.

Checks:
- fmt;
- clippy (Linux and the Windows target, with and without `bench`);
- 307 tests;
- `cargo audit` (1,329 advisories, 438 crates) and `cargo deny check`;
- benchmarks within the 50% limit.

The largest benchmark move, +23% on the 20,000-entry channels list (which this round did not touch), came from slow frames in the tail; its median was 0.31 ms.

The script benchmark that was 7–10% slower in round 19 is at −2% again, with `slang.rs` still unchanged. That fits round 19's guess that the slowdown came from where the compiler placed the code.

The Windows tests were still not run natively; that build is only type-checked here.

## Twenty-first round: review of round 20 (2026-10-15)

The round-20 review report itself did not reach this machine. This round works from the reviewer's summary of its one open finding, R20-F1. It held. The round-20 archive is unchanged; this round is a separate one.

| ID | Finding | Fix | Test |
|----|---------|-----|------|
| R20-F1 | while a replaced chat's last lines were still being shown, a reply typed in the window, or a timer set there, went to the new chat before its opening line appeared, though the window still showed the old conversation | a chat counts as usable only once it has begun: it can send, and its opening line is shown (`DccLink::usable`). Until then a reply in that window is refused there, with a note naming the line ("The new chat begins once the old chat's last lines are shown…"). A timer set then belongs to no chat, and sends nothing when it fires | `ui::a_new_chat_takes_nothing_before_it_begins`: with 700 old lines still to show, `/say` and `/me` reach neither peer and each gets the note, and a timer set then sends nothing later. Once the new chat has begun, a reply reaches it |

**Test update.** `ui::an_old_dcc_timer_does_not_follow_a_replacement_connection` used to set its last timer while the newest chat had not yet begun, and the new rule now refuses that. The test therefore lets the replaced chat end first, as hanging up does for a real chat, and then checks that the timer reaches the newest chat.

**Mutation checks.** I put the old code back for both parts (the usable check, and the timer's choice of chat), each capped at 1 GB; both were caught.

**Not checked in the window.** It needs typing in the window, which WSLg does not allow.

Checks:
- fmt;
- clippy (Linux and the Windows target, with and without `bench`);
- 308 tests;
- `cargo audit` and `cargo deny check`;
- benchmarks.

**The first full run failed its benchmarks.** One frame benchmark was at +52%, with every benchmark 15–50% slower, including the frozen old-transcript copy no round has changed. The round-18 build, run alternately with this one at the same time, was just as slow (+23% to +45%), so the machine was busy, not the code; WSL showed a load average of 3.45.

Run again once the load fell below 1.0, the full check passed, with benchmarks 1–37% above the saved baseline. The baseline was left as it was, rather than reset to a busy machine.

The Windows tests were still not run natively; that build is only type-checked here.

## Twenty-second round: review of round 21 (2026-10-15)

The reviewer confirmed the R20-F1 fix and found two new medium issues. This round works from the reviewer's summary: the full report did not reach this machine. Both held. The round-21 archive is unchanged; this round is a separate one.

| Issue | Fix | Test |
|-------|-----|------|
| a DCC chat that connected could send text being typed for another window, or for an earlier chat in the same window, to the new peer | **Not by itself.** A window no longer comes to the front by itself while something is being typed or a paste waits. This covers a chat that connects, our own channel join, and `/debug` (`bring_forward`); such a window is only marked. **Draft tied to its chat.** A draft remembers which DCC chat was in its window when typing began. If another chat is there at Enter, nothing is sent: the text stays in the input line, and a note says to press Enter again to send it to the new chat. Switching windows by hand carries the draft to the new window, as before | `ui::a_chat_that_connects_never_takes_a_draft_meant_for_another_window`: a chat connects and a join completes while a draft is typed for `#ops`; the window stays, the chat is marked, and the draft goes to `#ops`. With nothing typed, a chat that connects still comes forward. `ui::a_draft_typed_for_one_chat_is_not_sent_to_the_next`: a reply typed after a chat ended, before a new one connects; and a reply typed for a chat that is then replaced. Each first Enter sends nothing and keeps the text, and the second sends on purpose |
| a paste waiting for confirmation went wherever was in front at confirmation, including a replacement chat in the same window | a paste remembers where it was pasted: the window, the DCC chat in it and the server. It is sent there when confirmed, and to a chat that has ended or been replaced it sends nothing, with a note | `ui::a_waiting_paste_goes_where_it_was_pasted_or_nowhere`: a chat connecting while a paste for `#ops` waits leaves `#ops` in front, and the paste goes to `#ops`. A paste into a chat that is replaced before confirmation reaches no one |

**Mutation checks.** I put the old code back for each part (4 cases, each capped at 1 GB); all 4 were caught.

**Not checked in the window.** It needs typing in the window, which WSLg does not allow.

Checks:
- fmt;
- clippy (Linux and the Windows target, with and without `bench`);
- 311 tests;
- `cargo audit` (1,329 advisories) and `cargo deny check`;
- benchmarks within the 50% limit.

Benchmarks were 0–27% above the saved baseline, across untouched code too, as in round 21 (load around 1).

The Windows tests were still not run natively; that build is only type-checked here.

## Twenty-third round: review of round 22 (2026-10-15)

The reviewer confirmed the round-22 DCC paths and found three medium issues, plus notes on tests from rounds 20–22. This round works from the reviewer's summary: the full report did not reach this machine. All three issues held, and R22-F3 reached further than reported. The round-22 archive is unchanged; this round is a separate one.

| ID | Finding | Fix | Test |
|----|---------|-----|------|
| R22-F1 | a background join left `#ops` in front, but moved the server side's target to the new channel, so Enter sent the draft there. The round-22 test checked only the queue of commands, not where they finally went | before typed lines, a confirmed paste, or a timer's commands run, the server side is pointed back at the channel or query they belong to (`sync_target`, a quiet `/target`). The server side also moves its target on our own parts, kicks and nick changes, and anything without a target of its own followed it, not only plain text: `/me`, and `/topic` or `/kick` with no channel named | `ui::a_draft_reaches_the_channel_it_was_typed_in_after_a_background_join` runs the real protocol engine (`Pump`), with the window's lines and the server's joins interleaved as they happen. It checks the final IRC lines: the draft becomes `PRIVMSG #ops`, and a timer's `/topic` with no channel, after another background join, becomes `TOPIC #ops` |
| R22-F2 | `/editbox` put text in the input line without noting which DCC chat it was for, so after a replacement the first Enter sent it to the new peer | `/editbox` starts a new draft noted for whoever is in the window then. The window also notes a draft every frame, which covers text put in by history recall, completion or the color palette | `ui::an_editbox_draft_is_kept_for_the_chat_it_was_put_in_for` |
| R22-F3 | a paste waiting in the Messages window went to whoever wrote last when it was confirmed, not to the person it answered | the person a Messages draft or paste answers is noted with it (`Recipient`). A paste goes to that person. A draft typed for one person, when another has written since, is held on the first Enter, like a changed DCC chat. The draft case was not in the report: a reply typed in the Messages window had the same problem | `ui::the_messages_window_answers_whom_it_answered_when_the_words_were_written`: a paste for carol confirmed after dave writes goes to carol. A reply typed for dave, after erin writes, is held on the first Enter and sent to erin on the second |

A paste under the confirmation limit used to be sent without the target resync or history tracking. It now goes through the same path as typed lines.

The reviewer's notes on tests:
- **Round 22 paste test.** The old chat's incoming side is now dropped, and the test checks that the new chat has begun and can take lines before the paste is confirmed. Before, the replacement chat had not begun, which hid whether the paste was tied to its chat.
- **Round 20, blocked writer.** `ui::replacing_a_chat_stuck_writing_to_a_peer_that_never_reads`, over real TCP: the window's own sends fill the queue to a peer that never reads, until "not keeping up". Then a new chat arrives, and the old one is hung up and ends; the new one begins within 3 seconds and takes a reply.
- **Round 20, flood test.** It checks that the other chat is heard right after the first poll, not only by the end. The old chat's lines must be shown, numbered from 0 with none missed. The peer's thread is joined, so a failure there fails the test, and its writes are capped at 2 million lines and 10 seconds.
- **Round 21 test.** Plain text now goes through Enter. Each refusal checks for no line in the form a sent line would take: `> text` for words, and `<nick> text` for `/me`.

The tests' `sent()` helper leaves out the quiet `/target` lines, which are bookkeeping.

**Mutation checks.** I put the old code back for each part (6 cases, each capped at 1 GB). All 6 were caught after the R22-F1 test was corrected twice:
- the timer's `/me` named its channel itself, so it now uses `/topic`;
- the server's lines were applied before the window's, so they are now interleaved as they happen.

**Not checked in the window.** It needs typing in the window, which WSLg does not allow.

Checks:
- fmt;
- clippy (Linux and the Windows target, with and without `bench`);
- 315 tests;
- `cargo audit` (1,329 advisories) and `cargo deny check`;
- benchmarks within the 50% limit (−3% to +28%, the same across untouched code as in the last rounds).

The Windows tests were still not run natively; that build is only type-checked here.

## Twenty-fourth round: review of round 23 (2026-10-15)

The reviewer confirmed the `/editbox` fix and the original saved-recipient paste case. They found four medium issues and made two notes. All held. This round works from the reviewer's summary. The round-23 archive is unchanged; this round is a separate one.

| ID | Finding | Fix | Test |
|----|---------|-----|------|
| R23-F1 | the round-23 target reset was a separate line, so the server side could handle a background join between it and the message, and send the message elsewhere. A timer's `/topic` was affected too. The round-23 test delivered both lines back to back, which hid this | every line from typed input, a paste or a timer now travels to the server together with its window, as one line (`/via <window> <line>`, handled by `Pump`). Nothing the server says can come between them. The separate resync is gone. An event's lines have no window, and keep the server side's target. A line that picks a target of its own (`/query`, `/target`) keeps it | `ui::a_draft_reaches_the_channel_it_was_typed_in_after_a_background_join`: the real protocol engine, with another background join before every single line the window sends. A typed draft, a timer's `/topic` and a typed `/topic` all reach `#ops` |
| R23-F2 | a timer set while answering carol in the Messages window answered whoever wrote last when it fired | a timer remembers whom its Messages window answered when it was set (`UiTimer.reply`). Timers inside it, and timers started by a confirmed paste, keep the same person | `ui::a_timer_set_in_the_messages_window_answers_whom_it_answered_then`: a typed timer, a nested one with `/me`, and one from a confirmed paste all answer carol after dave writes |
| R23-F3 | a paste held in a Messages window with nobody to answer (opened by our own `/msg`) went to dave once he wrote | a paste or timer made in the Messages window answers only the person saved with it. With nobody saved, it sends nothing and says there is nobody to answer | `ui::words_meant_for_nobody_stay_unsent_when_someone_writes` |
| R23-F4 | after carol became caroline, saved recipients still said carol, so whoever took that nick next would get the text | a nick change on a server renames, on that server only, the person the Messages window answers, the draft's and waiting paste's saved person, and every timer's (`follow_nick`) | `ui::saved_recipients_follow_a_nick_change_on_their_own_server`: a carol on another server renaming changes nothing; carol becoming caroline takes the paste and the timer along; a draft follows a second rename |

Notes:
- **N1.** The flood test's peer has a 2-second write timeout. The test now requires its writing to end by being hung up on, not by a timeout or its own limit. It passed five times out of five.
- **N2.** A color code put into the input line from the palette starts a draft in that same frame.

The tests' `sent()` helper unwraps `/via <window>`, so existing tests read the command as typed. The R23-F1 test feeds the raw lines, `/via` included, to the real engine.

**Mutation checks.** I put the old code back for each part (5 cases, each capped at 1 GB): the `/via` wrapping, the timer's saved person, the "nobody" rule, the nick follow and its per-server scope. All 5 were caught.

**Not checked in the window.** It needs typing in the window, which WSLg does not allow.

### A second reviewer on round 24: Codex

At the user's request, an independent Codex agent (`codex exec --yolo`, model `gpt-6.1-sol`, reasoning `xhigh`) reviewed round 24 before it was packaged.
- **Setup.** It worked on its own copy of the tree, beside the round-23 code, so it could change nothing here. It ran the tests under the same 2 GB memory cap.
- **What it found.** It confirmed R23-F2 to F4 and both notes. It found five problems, all in the new `/via` path, and showed each one with a failing check. All five held when checked against the code, and are fixed:

| ID | Finding | Fix | Test |
|----|---------|-----|------|
| C24-F1 (high) | `/via` split its target at any Unicode space, but an IRC channel name may hold one (a no-break space, say). `#ops<nbsp>private` sent the text to `#ops`, and `#ops<nbsp>/quit` turned "hello" into a QUIT | the target ends at an ASCII space only, which IRC names never hold | `ui::a_channel_name_with_an_unusual_space_stays_whole`, through the real engine |
| C24-F2 (high) | wrapping every line overrode an alias that picks its own target: `/query alice \| /say confidential` in `#ops` sent "confidential" to `#ops` | a line that picks a target on purpose (`/query nick`, `/target`, `/win`, `/join`) leads the rest of its run: the typed line, paste or timer firing it is in. The next run starts from its own window again. (The first version of this fix guessed the choice on the window side, `run_target`. The second pass below showed that guess could go wrong, so it was replaced.) | `ui::an_alias_that_picks_a_target_is_followed_for_the_rest_of_its_run`: typed and from a timer, both reach alice, and the next typed line is back in `#ops` |
| C24-F3 (medium) | a typed `/via #ops /join #locked swordfish` kept its key in the saved history | the history check reads through `/via` to the line it runs | `ui::a_typed_via_line_with_a_password_is_never_saved`, and cases in the `input` test |
| C24-F4 (medium) | 1,024 nested `/via` wrappers overflowed the stack and aborted | wrappers are unwrapped in a loop, at most 8 deep; deeper ones get a note | `pump::via_runs_a_line_for_a_target_and_cannot_be_nested_without_end` |
| C24-F5 (medium) | `/via #ops /target #ops` was undone, since choosing the same target looked like choosing none | the engine notes when a command chooses a target on purpose (`picked`), and `/via` keeps that choice | the same pump test, for `/target`, `/target -q`, `/join` and `/win` |

**Mutation checks.** I put the old code back for each of the five fixes (each capped at 1 GB). All were caught; the C24-F1 case was caught once its pattern was matched against the reformatted source.

### Codex, second pass

Codex then reviewed those fixes, using its first report and the previous tree as its base. It confirmed C24-F1 and C24-F5, and found the other three only partly closed. All five new findings held:

| ID | Finding | Fix | Test |
|----|---------|-----|------|
| C24b-F1 (high) | a popup's choice never ended. `/join #public` from a popup in `#ops` made a later popup's `/say confidential` in bob's DCC chat go to `#public`. The same happened across servers | the window side no longer keeps a choice at all (see below). Every click, drop, dialog button and prompt is one run, like a typed line | `ui::a_popup_choosing_a_target_leads_only_its_own_lines`: within one popup the choice holds. A later popup in bob's chat reaches bob only, and one on the other server reaches its `#ops` |
| C24b-F2 (high) | the window side read target choices its own way, so it disagreed with the engine. Cases: `/query<tab>alice`, `/j ,#private key`, and `/j &private` where the server makes only `#` a channel (it joins `#&private`) | the engine, which runs the command, is now the only reader of what it chose | `ui::a_target_chosen_any_way_the_server_side_reads_it_leads_the_run`: all three, plus a capitalized `/QUERY`, through the real engine |
| C24b-F3 (high) | a choice inside a `/via` of the alias's own was missed. A choice in the Messages window was too: `/query alice \| /say confidential` went to carol, whom the window answered | every line is wrapped, a `/via` of its own included. The Messages window's lines are wrapped with the person it answers, so the engine sees the choice there too | the same test for the explicit `/via`; `ui::a_target_chosen_in_the_messages_window_leads_the_run` |
| C24b-F4 (medium) | `/VIA` or `/via<tab>` inside a `/via` was not counted, so 1,024 of them still overflowed the stack | wrappers are recognized the way the engine reads any command name: any case, any whitespace after it. Nesting stops at 8, however it is spelled | the pump test, with `/VIA`, `/Via<tab>` and mixed nesting 1,024 deep |
| C24b-F5 (medium) | the history check missed `#VIA #ops /join #locked swordfish` (with `#` as the command prefix) and `/via<tab>#ops /oper …` | the history check reads `/via` with the engine's own functions (`pump::via_args`, `pump::unwrap_via`) after taking off any prefix. A `/via` it cannot read is not saved | `ui::a_typed_via_line_with_a_password_is_never_saved`, with seven spellings and prefixes |

**The redesign.** The window side numbers its runs: one typed line or paste, one timer firing, one click or button. Each line goes to the server as `/via =<run> <target> <line>`. The target is the channel or query, or in the Messages window the person answered. When a line in a run chooses a target on purpose, the engine keeps that for the rest of the run, but not for a line with a `/via` of its own. The choice ends once another run speaks, so a number typed by hand cannot plant one for later. Words with no target (plain text, `/say`, `/me`) no longer get a `/msg <name>` from the window side; the wrapper carries the target.

The changed tests' `said()` helper runs what the window sent through a real engine and reads back the final PRIVMSGs. The seven tests that compared `/msg nick text` lines now use it.

**Mutation checks.** I put the old behavior back for six parts (each capped at 1 GB): one run per popup, the engine keeping a run's choice, the Messages wrapper, a line's own `/via` winning, the any-case name check, and the history check's shared grammar. All six were caught.

### Codex, third pass

Codex reviewed the redesign. It confirmed C24b-F1 to F5 closed, rerunning its own checks. It also passed controls for confirmed pastes, nested aliases and timers, perform, events, `/scon`, and Messages-window `/topic`, `/mode`, `/kick` and `/notice`. It found four more, all real:

| ID | Finding | Fix | Test |
|----|---------|-----|------|
| C24c-F1 (high) | the tab and tree menus and the file-drop command ran each of their lines as a run of its own, so `/query alice \| /say confidential` from them went to `#ops` | every line of one menu choice, click or drop is one run (`run_with_fields`), and so is everything one script event or one perform runs | `ui::a_menu_choice_or_a_drop_is_one_run` |
| C24c-F2 (medium) | a run's chosen person did not follow a nick change. If alice became alice2 and someone else took "alice" between two lines of the run, the second line went to the newcomer | the engine renames the run's choice on NICK, with the server's case rules | the pump test: alice renames, mallory takes her nick, and the rest of the run reaches alice2 |
| C24c-F3 (medium) | the status window sends its lines unwrapped, so a typed `/via =2 #ops /query alice` could claim the next run's number, and that run's channel text went to alice | the window side refuses any `/via` whose target starts with `=` (run numbers are its own), and the engine accepts a run number only on the outermost `/via` | `ui::a_run_number_written_by_hand_is_refused`, for two spellings; bad markers in the pump test |
| C24c-F4 (medium) | older than this round: the history check read `/raw /raw /raw …` recursively, so 4,096 of them overflowed its stack | it reads at most eight `/raw` or `/quote` deep; deeper, the line is not saved | the `input` test, 4,096 deep |

**Mutation checks.** I put the old behavior back for five parts (each capped at 1 GB): the menu run, the nick rename, the refusal, the outermost-only rule, and the `/raw` depth limit. All five were caught.

### Codex, fourth pass

Codex found no new problems. It confirmed C24c-F1 to F4 closed: its 16 earlier regression checks pass unchanged. It passed seven new controls: event and perform routing, events at startup, a `/via` used on purpose in an alias or script, forged and nested run numbers, a choice ending with its run, three case mappings, and mixed `/raw`, `/quote` and `/via` nesting. All its tests ran under the 2 GB cap.

**Tried in the window.** I used a fake server and a channel double-click command (it runs through the same path as the menus). `/query alice | /say … | /join #second | /say …` sent its first message to alice and the second to `#second`. Then `/say first-$chan | /query alice | /say then-alice` was run in `#second`, then in `#room`. Each run's first line reached its own channel, so the choice made in the run before did not carry over.

## Twenty-fifth round: review of round 24 (2026-10-15)

The reviewer found that the `/via` wrapper closes the round-23 race. They confirmed R23-F2, F3 and N1, N2, and the inspected Codex fixes. They found four medium issues, all by reading the code, and made two notes. All held. The round-24 archive is unchanged; this round is a separate one.

| ID | Finding | Fix | Test |
|----|---------|-----|------|
| R24-F1 | `/via NickServ ID swordfish` sent a password but stayed in saved history: the history check dropped the `/via` target and read `ID swordfish` as plain text | the check keeps the target: anything a `/via` sends to a service counts as a possible password, as `/msg NickServ …` already did. So does a `/target` or `/win` naming a service, since the rest of its run may speak to it. Every line is now checked as it goes out, with the target it travels with, so words typed in a query with a service count too | `ui::a_password_to_a_service_never_reaches_saved_history`: eight cases (nested, capitalized, tab, `#` prefix, two aliases, a query window, `/msg` as a control). Each goes through a real engine, then every saved snapshot is checked |
| R24-F2 | a line queued for carol still said carol when the server side read her rename first, so it went to whoever took her old nick | the server side counts the nick changes it sees and remembers the last 256. Every line the window sends carries the count the window had seen (`/via =<run>:<count> …`). A nick written before a change is followed through every change since, in order. If changes it needs were forgotten, the line is refused, not sent to a guess. Channels and targets written by hand are left alone. Once the window has seen a change, a line meant for the new holder of the old nick goes to them | `ui::a_queued_line_follows_a_nick_change_the_server_side_saw_first`: a confirmed Messages paste, a typed query line and a timer, all queued before carol's rename and mallory taking "carol". Then `/msg carol` and a new query with the newcomer still reach the newcomer. Pump test: the 256 limit, channels, and case mapping |
| R24-F3 | in bob's DCC chat, `/query alice \| /say confidential` sent "confidential" to bob, since DCC words never reached the server side | the window side notes when a line of the run chooses a target, using `pump::picks_target`. The server side's own test holds that function to the choices it really makes, over 22 lines. Words of such a run meant for a DCC chat, or for an empty Messages window, go to the server side marked `-`: only what this run chose, and nothing if the choice did not take. Lines from DCC, status and events now carry their run too (`*`, the server side's own target) | `ui::a_dcc_run_that_chooses_someone_on_irc_never_tells_the_chat`: an alias, a popup and a timer in the chat (plain text, `/say`, `/me`) reach alice, never bob. A failed choice reaches bob, and the next run is bob's again. `ui::a_run_that_chooses_someone_in_an_empty_messages_window_reaches_them`; `pump::the_window_side_reads_a_target_choice_as_the_server_side_makes_it` |
| R24-F4 | Open query, with no message, left `#ops` in front, so the next Enter went to `#ops`. So did a typed `/query alice` on its own | both open alice's query on that server and put it in front. Inside an alias, `/query` still leads only the rest of the alias's run | `ui::opening_a_query_puts_it_in_front_for_what_is_typed_next`: the prompt on both servers, and a typed `/query bob` |

Notes:
- **N1.** COMMANDS.md now says that `/via` puts the talk target back unless the line itself chooses one, and that typed lines still go to their window. `/via` is in Tab completion.
- **Packaging.** The round-24 summary blamed the archive for the missing executable bit on `scripts/check.sh`. The reviewer showed the bit is stored (mode 100755). It is Python's extraction that drops it; `unzip` keeps it. This machine has no `unzip`, so the fresh-unpack check now uses a small extractor that applies the stored modes.

**Mutation checks.** I put the old behavior back for ten parts (each capped at 1 GB):
- the service target in `/via`;
- checking the line as it goes out;
- `/target` naming a service;
- following renames;
- the count the window sends;
- the DCC diversion;
- the `-` refusal;
- the typed `/query`;
- the prompt;
- `picks_target`'s `/query` rule.

All ten were caught.

**Tried in the window.** I used a fake server. A status-window double-click set to `/query alice | /say from-status` sent its words to alice. A channel double-click after it started from its own channel. Typing, the Open query prompt and DCC need keyboard input, which WSLg does not give; the tests above drive those paths.

### Codex on round 25

Codex reviewed round 25 against the round-24 archive and the outside review. Its independent checks confirmed:
- R24-F3, F4 and N1 closed;
- the R24-F1 `/via` cases;
- the R24-F2 queued-message cases, including two servers, our own nick changes, the window lagging, and the 255/256/257 limits.

It found three more medium issues, all real:

| ID | Finding | Fix | Test |
|----|---------|-----|------|
| C25-F1 | switching windows still sent a plain `/target -q carol`. Queued before carol's rename, it set the server side's own target back to "carol". Status-window words, which go to that target, then reached the newcomer | a window switch travels like any other line: `/via =<run>:<count> carol /target -q`. A bare `/target -q` now means "choose, quietly, the target this line came for", so a renamed carol is chosen as caroline | `ui::a_window_switch_queued_before_a_rename_follows_it` |
| C25-F2 | in the status window, `/target NickServ` and then, on another Enter, `/say ID swordfish`: the second entry was saved, since the window cannot see which target the server side holds | words sent for a target only the server side knows (`*`: its own; `-`: what the run chose) are kept out of saved history | `ui::words_for_the_server_sides_own_target_stay_out_of_saved_history`, through a real engine and every saved snapshot |
| C25-F3 | a refused line was shown in full: "`/identify swordfish`: carol changed nick too often…" | refusal notes never repeat the line | the pump test: `/identify`, a keyed `/join` and `/msg NickServ ID` refused, with nothing shown that holds the password |

**Mutation checks.** I put the old behavior back for four parts (each capped at 1 GB): the switch's wrapper, the bare `/target -q`, keeping such words back, and the refusal text. All four were caught.

### Codex on round 25, second pass

Codex confirmed C25-F1 to F3 closed on their reported paths. It found five medium issues and one low, all real:

| ID | Finding | Fix | Test |
|----|---------|-----|------|
| C25b-F1 | the window switch, as a run of its own, ended what the enclosing run had chosen: `/query alice \| /channel #ops \| /say …` reached `#ops` | a window switch carries the nick-change count but no run (`/via =:<count> …`), so it never touches a run's choice | `ui::a_window_switch_inside_a_run_keeps_what_the_run_chose` |
| C25b-F2 | a window switch refused for too many nick changes left the old target, so status-window words went there | a refused line also clears the server side's own target: words with no target go nowhere until a window says where | the pump test |
| C25b-F3 | `/describe`, `/ctcp` and `/ctcpreply` to a service stayed in saved history | the history check reads their target like `/msg`'s | `ui::a_password_to_a_service_never_reaches_saved_history`, three more cases |
| C25b-F4 | an action or CTCP to a service was shown, and logged, with its password | they are masked like messages are | `pump::echoes_to_a_service_hide_its_password`; `ui::words_never_sent_are_never_shown` (an action in a service's query) |
| C25b-F5 | the window side's own notes quoted unsent words ("…closed, so “ID swordfish” was not sent") | none of the five such notes quotes the words now ("…so a line was not sent"), in all four translations too | `ui::words_never_sent_are_never_shown` (a timer firing after its service query closed); six older tests now check the words are left out |
| C25b-F6 (low) | a typing notice queued before carol's rename told whoever took her nick | typing notices go like window switches (`/via =:<count> carol /typing active`), and `/typing <state>` alone is for the target there is | `pump::a_typing_notice_written_before_a_rename_follows_it` |

**Mutation checks.** I put the old behavior back for seven parts (each capped at 1 GB). All seven were caught.

### Codex on round 25, third pass

Codex confirmed:
- C25b-F1, F3 and F5 closed;
- C25b-F2, F4 and F6 closed on their reported paths.

It found one medium issue and two low ones, all real:

| ID | Finding | Fix | Test |
|----|---------|-----|------|
| C25c-F1 | the service mask read the first word as it stood, so CTCP framing (`\x01ID …`) or a leading `ACTION` hid the password from it. That affected the Debug window, and the transcript and log for `/ctcp NickServ ACTION ID …` | the mask reads the words inside CTCP framing and after `ACTION`, keeping both in place (`\x01ACTION ID ****\x01`) | `pump::echoes_to_a_service_hide_its_password`, with both `ACTION` forms and the Debug window's copy of every line sent |
| C25c-F2 (low) | after the window saw carol's rename, the "done typing" notice still named carol, so it went to her nick's new holder | the notice in flight follows nick changes on its server, like other saved recipients | `ui::a_typing_notice_follows_a_rename_the_window_saw` |
| C25c-F3 (low) | an expired typing notice cleared the server side's own target, so the next status-window line went nowhere | a refused line that chooses a target clears it: a window switch, or `/join`, `/query`, `/target` or `/win` with a name. Other refused lines, a typing notice among them, leave it as it was. Every refused line was refused whole, even one that never needed its window's nick. Round 26 narrows both (R25-F4): only a window switch clears the target, and only lines that need the window's nick are refused | the pump test |

**Mutation checks.** I put the old behavior back for four parts (each capped at 1 GB). All four were caught. Codex has not re-checked these three fixes; they are new in this archive.

## Twenty-sixth round: review of round 25 (2026-10-15)

The reviewer checked the round-25 archive from a fresh unpack. They found one test that fails on a slower machine, two medium issues in the code, and one low one where this file said more than the code did. All four held. Review of this round's fixes found two more, N1 and N2, also fixed here. The round-25 archive is unchanged; this round is a separate one.

| ID | Finding | Fix | Test |
|----|---------|-----|------|
| R25-F1 | `ui::replacing_a_chat_stuck_writing_to_a_peer_that_never_reads` failed on a 2-core machine with large socket buffers. Each round of its loop read every line of every window again, so it slowed as lines piled up. The peer hung up after 10 s, before the 256-line queue filled, and the 5 s guard fired | the loop reads only the newest line of each status window, where a line not sent is told, so every round costs the same. The peer waits until the test is done (a failing test drops the sender, so the peer still ends). The 60 s guard is only against a hang. The DCC twin, `dcc::hanging_up_ends_a_chat_stuck_writing_to_a_peer_that_never_reads`, gets the same changes. In both, the hang-up must take under 10 s instead of 3 s: still well short of the 30 s a write may wait | both tests, ten runs each in a row, pinned to two cores in a debug build under the 1 GB cap: all twenty passed (0.45 to 0.48 s, and 0.31 s, each). With 40 ms added to every round, as on a slower machine, the old forms failed and the new ones passed |
| R25-F2 | the nick-change log held only nick changes. When someone quit (a kill reaches others as a quit) and someone else took the nick, lines written for the one who left went to the newcomer. A line could also be followed through a quit: carol became caroline, caroline quit, mallory took "caroline", and a line for carol reached mallory | a quit by someone else goes into the same counted log, with no new nick. A line written before it, for that person, is refused: "* a line for carol was not sent: carol left meanwhile", never the line itself. A later change into that nick is someone else's and is never followed. A line written after the window saw the quit goes to whoever holds the nick then, like any line to a nick. Our own quit is not counted. Two more paths had the same fault and are closed too: the rest of a run that chose someone who then quit says nothing more (not to the newcomer, nor to the window the run began in; a line naming its own target still runs, and a new choice leads again), and the server side's own target is cleared when its nick quits. Accepted limit: if carol quits and comes back under her nick before the line arrives, the line is still refused, on the safe side | the reviewer's `pump::a_nick_freed_by_a_quit_and_retaken_is_not_followed`, as given. `pump::a_quit_ends_what_was_written_for_that_nick`: lines written after the quit are sent; someone else's quit stops nothing; channels; our own quit; rejoining; case mapping. `pump::quits_count_toward_the_kept_changes_like_renames`: one less than the limit, the limit, and one more, every other one a quit. `pump::what_was_chosen_for_someone_who_left_goes_nowhere`. `ui::a_queued_line_never_reaches_whoever_takes_the_nick_of_someone_who_left`: a Messages paste, a query line and a timer, through a real engine. Nothing is sent, and each of the five notes names carol only |
| R25-F3 | services were found by name only (ending in "serv", or `ns` and `cs`; the history check also knew `ms`, `hs`, `bs` and `os`). QuakeNet's Q and Undernet's X are not named like services. So `/msg Q AUTH me swordfish` was shown, logged, copied to the Debug window and saved to history, password and all | one `connection::is_service`, used by both, knows all eight names in any case, and takes off any `@host` first. Masking now reads the words too. To anyone but a channel, a message that opens with AUTH, LOGIN, IDENTIFY, ID, REGISTER, GHOST, RECOVER, RELEASE, REGAIN, NEWPASS, or SET PASSWORD (or PASS) is shown as `AUTH ****`, inside CTCP framing and after ACTION as well. A service keeps its full list (any SET). Every echo and the Debug window use this. The history check reads the same words, whatever the target: in plain text, and in what `/say`, `/me`, `/msg`, `/notice`, `/query`, `/describe`, `/ctcp`, `/ctcpreply` and `/raw PRIVMSG` or `NOTICE` send. Q and X are not added as service names: someone called x on another network is not a service | the reviewer's `pump::logins_to_q_and_x_are_masked`, as given. `pump::every_echo_of_a_login_to_a_bot_hides_its_password`: twelve commands, with the Debug copy of each. `ui::a_login_to_a_bot_is_masked_in_the_transcript_the_log_and_debug`: a real connection with Debug on; the query windows, the log files as written, and the Debug window show `AUTH ****` and `LOGIN ****`. `ui::a_password_to_a_service_never_reaches_saved_history`: four more cases (Q, `X@host`, typed in Q's query, the Messages window). `ui::words_that_only_look_like_a_login_are_shown_and_kept`. `input::a_login_counts_whatever_it_is_sent_to`. `connection::services_are_known_by_name_and_logins_by_their_words` |
| R25-F4 (low) | the C25c-F3 row said more than the code did. A refused line was refused whole, even when it never needed its window's nick: `/join #foo`, `/whois bob` and `/msg bob hi` typed in n0's query after 257 forgotten changes were all dropped. And any refused line that chose a target cleared the target, not only a window switch | when the nick a line was written for cannot be followed (its changes were forgotten, or the person left), only what needs that nick is refused: words (plain text, `/say`, `/me`), a window switch (`/target -q`), and a typing notice with no target of its own. Any other line runs with no default target while it runs. A target it names itself works; anything that would have used the window's nick gets the usual "no channel selected". Only a refused window switch clears the target. The C25c-F3 row now says what round 25 did | `pump::a_line_that_needs_no_nick_runs_when_its_window_nick_is_unknown`: `/join #foo`, `/whois bob`, `/msg bob hi` and `/identify` are sent; `/topic` gets "no channel selected"; words are refused and nothing of them is shown. The target stays as it was, except after `/join`, which chooses its channel as it always does, and after a refused window switch. The older pump test now expects only words to be refused |
| N1 | the same fault on the window side, found in review of this round. A draft, a waiting paste, a timer, or the Messages window, when it answered carol, followed her nick changes but not her quit. Once the window had seen carol quit and mallory take "carol", what was written for carol carried a count past the quit, so the server side sent it to mallory | when the window sees someone quit, the Messages window stops answering them and a typing notice to them is dropped. A draft, waiting paste or timer written to answer them is kept as written for someone who left. Its words are refused when they would be sent: "carol left, so a line was not sent." in the Messages window, or, for a draft, "carol left while you typed; press Enter again to send this to whoever this window answers now." Neither note quotes the words. A later change into or out of that nick is not followed. A new private message from whoever holds the nick starts afresh. Both notes are in all four translations | `ui::what_was_written_for_someone_who_left_never_reaches_whoever_takes_the_nick`: a paste, a timer and a draft, all for carol, through a real engine after the window saw the quit and the takeover. Nothing is sent; four "carol left" notes and the draft's note are shown; the words are shown nowhere; a second Enter goes to nobody; a new message from the new carol is answered. `ui::someone_who_left_is_not_followed_through_the_next_holders_rename` |

Notes:
- **Channels are not masked by their words.** The reviewer asked for the login words to be masked whatever the target. In the transcript, the log and the Debug window, a channel is the exception. A channel is no login bot, and everyone in it reads what is said there, so masking hides nothing. But it would spoil ordinary lines that open with these words ("Login page is broken", "Release is tomorrow"). Saved history has no such exception: the typed line alone does not say where it went, so plain text that opens with one of these words is kept out of history wherever it was typed.
- **NEWPASS** is on both lists. Q and X both use it to change a password.
- **N2. The log keeps 4,096 changes.** Now that quits count, one netsplit on a big channel brings hundreds at once, and with 256 kept every line queued across it was refused as "nicks changed too often". The log now keeps the last 4,096 nick changes and quits; the tests that go past the limit scale with it. Test: `pump::a_line_queued_across_a_netsplit_still_reaches_its_nick` (a thousand quits, with carol's rename among them).
- **Query windows keep their nick (N1's limit).** A paste or timer made in carol's query window before she quit goes, once the window has seen the quit, to whoever holds "carol" then: that window is the nick, the rule R24-F2 set. Only what is saved outside a query window follows a quit: the Messages window's answer and a typing notice are dropped, and a waiting paste, a timer or a draft written to answer someone there is kept as written for someone who left.

**Mutation checks.** I put the old behavior back for sixteen parts (each capped at 1 GB):
- R25-F1: the round-25 forms of both DCC tests, pinned to two cores. On this machine (20 cores, smaller socket buffers) both still passed, as expected: the fault needs a slower machine. With 40 ms added to every round to stand in for one, both old forms failed, the UI one with the reviewer's "the queue filled". Both new forms, slowed the same way, passed. The two slowed old forms are the two F1 checks;
- R25-F2: no log entry for a quit; an entry that does not stop the walk, so a later change into the nick is followed; a run's choice dropped when its person quits, instead of kept as left (its next words reached the channel the run began in); the server side's own target kept after its nick quit;
- R25-F3: masking by the target's name only; the history check without the login words; `is_service` without taking off `@host`;
- R25-F4: the whole-line refusal; refusing a line that chooses a target, and clearing the target for it; the window's target left in place while such a line runs (its `/topic` reached `#chan`);
- N1: following nick changes only, with nothing done on a quit (the timer's words reached mallory); a quit that makes the person answered nobody instead of someone who left (the "carol left" notes were missing); following one who left through the next holder's rename (the note named mallory2);
- N2: keeping 256 changes again (the line queued across the netsplit was refused).

All sixteen were caught.

**First run of the upstream CI.** The Linux job (the whole of `scripts/check.sh`, benchmarks included) passed on the first run. The Windows job, the first time the tests ran on Windows itself, found two faults in the tests, not in the client:
- `i18n::every_built_in_catalog_translates_every_text` reads the sources with `include_str!`, which keeps the line endings as checked out. Git on Windows checks out with CRLF, so a text continued over two lines no longer matched its catalog entry. The test now reads the sources as the compiler does (CRLF as LF), and `.gitattributes` keeps every checkout at LF.
- `ui::a_flood_waits_in_the_connection_instead_of_in_memory` wrote 8 MB and expected the write to block. Windows grows its loopback buffers past that, and a Linux machine with a large `tcp_rmem` would too. The server side now writes without blocking until nothing has been taken for 1.5 s, and fails only if the connection takes more than 64 MB with nobody reading. The count of lines is then whatever was written, read back once and in order.
- A later Windows run failed `settings::saves_at_the_same_time_each_leave_a_whole_file` once, with "Access is denied" from the rename that puts a finished save in place of the settings file. Windows refuses to replace a file that something else holds open just then (here another thread reading it; in use, an indexer or a virus scanner). `settings::replace_file` now tries the rename again for up to about a second on Windows for that error and a sharing violation, and both saves that replace a file (settings, log trimming) go through it. The test's own reads retry the same way.

**The repository's own guards.** With the code on GitHub, the checks that `scripts/check.sh` runs by hand are now enforced there as well:
- `check.yml` runs the whole script on Linux and clippy plus the tests on Windows for every push to `main` and every pull request, and once a week. Its benchmarks are compared with the base of the change (the tip of `main` for a pull request, the commit before for a push), which `scripts/bench-base.sh` builds and runs on the same runner just before: a change more than 50% slower on any benchmark fails. The first design kept the last results from `main` in the Actions cache and compared the next run with them; the second run, with the code unchanged, came out 39 to 126% slower on every benchmark, since shared runners differ from each other by that much. Two builds on the same runner a minute apart mostly differ by a few percent, but one run still showed two short benchmarks at twice their time while others were a third faster, on a change to the settings file alone: a moment of contention on the runner. So each side runs its benchmarks three times (`BENCH_ROUNDS`, `--rounds`) and keeps the best time of each, which such a moment cannot inflate. A benchmark the base does not know passes as new (`--allow-new`, `BENCH_ALLOW_NEW=1`); one that no longer runs still fails. A newer push to the same branch cancels the run still going.
- `codeql.yml` scans the sources and the workflow files with GitHub's security queries. Code scanning is only available on public repositories (or with Advanced Security), so while the repository is private the analysis is skipped and the check passes; it does its work as soon as the repository is public.
- `dependabot.yml`: security updates as soon as an advisory lands; version updates weekly, with minor and patch bumps gathered into one pull request after a three-day cooldown; the same for the workflow actions. Dependabot alerts and automatic security updates are switched on in the repository settings.
- `.github/rulesets/main.json` is the rule for `main`, ready to import once rulesets are available (public repository or GitHub Pro): changes come through pull requests only, squash or rebase, linear history, no force pushes or deletion, and `linux`, `windows`, `Analyze (rust)` and `Analyze (actions)` must pass on the branch brought up to date with `main`. The repository admin can bypass only through a pull request.
- `SECURITY.md` asks for private vulnerability reports through GitHub; a pull request template carries the checklist (the gate, a test on the path, a row in `REVIEW.md` or `FEATURES.md`, translations, no secrets or personal paths).
- The release profile now builds with one code unit and thin LTO. Measured against the round-26 baseline: the server-side line parse 10% faster, wire decoding 13%, scripts 6 to 8%, URL finding 7%; frame benchmarks within noise. A release build from clean takes about two minutes instead of one.
