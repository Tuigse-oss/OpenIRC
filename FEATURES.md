# Feature backlog

What an experienced IRC user would look for in OpenIRC, grouped by area:
the settings, windows, and habits that a full desktop IRC client is expected
to have, written in OpenIRC's own words.

Priority:
- **P1** core chat experience; people notice in the first hour.
- **P2** common power-user features.
- **P3** niche, legacy, or platform-specific; may never be built.

Status: **done** (in OpenIRC before round 2), **new** (added in round 2,
2026-10-13), **done (round 3)** or **(round 4)** (added while working down this list),
**partly**, **open**, **not planned**.

How the new items were checked: protocol and parsing pieces have unit tests;
keyboard and paste behavior is tested by feeding key and paste events into
real headless frames (`ui::tests`); the window was driven by mouse against a
local test server for the rest (screenshots in the review notes). Typing in
the live window could not be tested on the review machine (WSLg gives the
window no keyboard focus).

## Connecting

| Feature | Priority | Status |
|---|---|---|
| Server list with networks, groups, per-server ports and password | P1 | done (round 3) |
| Alternative nickname tried when the first is taken | P1 | new |
| Per-server login method (SASL PLAIN, SASL EXTERNAL, NickServ, server password) | P2 | done (round 3) |
| Client TLS certificate (CertFP / SASL EXTERNAL), create one | P2 | done (round 3) |
| Prompt to trust a server certificate that fails verification | P2 | done (round 3; pinned by SHA-256 per host) |
| Retry: timeout, attempts, delay, next server in group | P2 | done (round 3) |
| Per-server text encoding | P2 | done (round 3; auto, UTF-8, and 6 legacy encodings) |
| Connect on startup | P2 | new |
| Bind address, IPv6 preference, port ranges, UPnP | P3 | done (round 3; bind address with fallback, address order, DCC port range); UPnP not planned |
| Local host/IP lookup methods | P3 | done (round 3; local address, address the server reports, or set by hand) |
| Identd options (system name, port, only while connecting) | P3 | done (round 3) |
| Proxy per connection type, exception masks | P3 | done (round 3; servers, DCC, or both) |

## Channels and messages

| Feature | Priority | Status |
|---|---|---|
| Close a window (part, close query/list) from switchbar, tree, or keyboard | P1 | new |
| Channel central: topic, modes, key, limit, ban/except/invite lists with remove | P1 | new |
| Channels list window with search, user-count filters, sort, join on double-click | P1 | new |
| Nickname list position (left/right/hidden), counts, mode grouping | P1 | new |
| Short join/part lines; hide ping/pong; skip MOTD on connect | P1 | new |
| Timestamp format | P1 | new |
| Own-message prefix and event line styles (join/part/quit/kick/mode/notice) | P1 | new |
| Line marker for the last read line | P1 | new |
| Rejoin on kick, join on invite, whois when a query opens | P2 | new |
| Quit message and finger reply settings | P2 | new |
| Highlight rules: per-rule channels/nicks, match type, case, regex, color, sound, flash | P2 | done (round 3) |
| Nickname color rules (by mode, level, idle) | P2 | done (round 3; by mask, mode and idle time, plus hash colors) |
| Ignore with per-type options (private, channel, notice, CTCP, DCC, invite, codes) and timeouts | P2 | done (round 3; also per network) |
| Auto-op / auto-voice / protect lists | P3 | done (round 3) |
| Strip colors and codes from incoming messages | P2 | new |
| Split long outgoing messages | P2 | done |
| Outgoing flood control (queue own messages) | P1 | new |
| Incoming flood protection details (CTCP and whois reply limits, query limit) | P2 | done (round 3; CTCP answers counted over everyone, new-query limit) |
| Show away/CTCP/invite/notice/query/whois in the active window | P2 | done (round 3) |
| Single message window, copy messages to query | P3 | done (round 3; open queries still get their own messages) |
| Typing notifications, STS, other IRCv3 extensions | P2 | done (round 3; CAP LS 302 with multi-prefix, away-notify, account-notify, extended-join, chghost, server-time, message-tags, invite-notify, cap-notify, userhost-in-names; typing; STS upgrade and remembered policy. Not used: echo-message, batch, labeled-response) |
| End-to-end encrypted private messages with key verification | P3 | not planned (no agreed IRC standard; home-made cryptography would be unsafe) |

## Input and keyboard

| Feature | Priority | Status |
|---|---|---|
| Command history (Up/Down) | P1 | new |
| Tab completion of nicknames with a suffix | P1 | new |
| Formatting shortcuts (bold, underline, italic, reverse, plain) and a color picker | P1 | new |
| Confirm before pasting many lines | P2 | new |
| Window shortcuts: next/previous window, close window, Options, channel list, scripts, find, full screen | P1 | new |
| Multi-line editbox | P3 | done (round 3; Enter sends, Shift+Enter breaks the line) |
| Configurable command prefix, line separator | P3 | done (round 3) |
| Double-click commands per window and per nickname | P3 | done (round 3) |
| Command to run when a file is dropped on a window | P3 | done (round 3; `/dcc send $nick $file` by default) |

## Display

| Feature | Priority | Status |
|---|---|---|
| Application icon (taskbar, window, tray) | P1 | new |
| Toolbar with icons and tooltips | P1 | new |
| Switchbar and treebar icons, activity colors (event / message / highlight) | P1 | new |
| Color schemes: several named schemes, add/delete | P2 | new (built-in schemes; custom scheme is the Colors page) |
| Editable 16-color message palette | P2 | done (round 3) |
| Font family for the transcript (proportional or monospace) | P2 | new |
| Full screen, always on top | P2 | new |
| Line shading (alternate rows) and line spacing | P3 | new (shading) |
| Desktop notifications ("tips") per event | P2 | new (in-window notices plus taskbar attention) |
| Tray options: minimize to tray, animate on activity | P3 | done (round 3; Linux tray; Windows and macOS have no tray icon yet) |
| Window tiling/cascading, desktop windows, window order rules | P3 | partly (round 3: order as opened or by kind and name; tiling and desktop windows not planned in the single-window design) |
| Network name and own nick in titles | P2 | new |
| Online timer | P3 | new |

## Logging, URLs, history

| Feature | Priority | Status |
|---|---|---|
| Per-window logs with timestamps | P1 | done |
| Log options: strip codes, date-stamped file names, per-network folders, size trim, auto-reload | P2 | done (round 3) |
| Log browser: search, view, merge, rename, delete | P2 | done (round 3; search by name and text, view, rename, delete; merge not planned) |
| URL catcher list (open, copy, delete) | P2 | done (list window) |
| Clear history (servers, channels, URLs, finds) | P3 | done (round 3; typed lines, URLs, find text, channel list, Messages window) |

## DCC and files

| Feature | Priority | Status |
|---|---|---|
| Chat, send, receive, resume, passive | P1 | done (round 13: resume works both ways, when receiving and when sending; a file offered without listening, passive, cannot be resumed) |
| Accept prompt with resume / overwrite / rename when the file exists | P2 | done (round 3) |
| Auto-accept from trusted users; ignore file types | P2 | done (round 3) |
| Download folders by file type | P3 | done (round 3) |
| Transfer window with progress and speed | P2 | done (round 3; with cancel) |
| DCC file server, DCC server mode | P3 | not planned (a known limitation) |

## Scripting

| Feature | Priority | Status |
|---|---|---|
| Events, aliases, popups, variables, timers | P1 | done |
| Script editor with several files, line numbers, find/replace, go to line, bracket check | P2 | done (round 3) |
| Listen/debug view for events and raw lines | P3 | done (round 3; `/debug`, passwords masked) |
| User levels | P3 | done (round 3; numbered and named levels on `on` events) |
| Identifiers such as `$left`, `$replace`, `&&`/`||`, `/halt`, `/return` | P2 | done (round 3; also `elseif`, `!`, `>=`/`<=`, `isin`/`iswm`, `$+`, token and math identifiers, aliases as identifiers) |

## Typed commands

| Feature | Priority | Status |
|---|---|---|
| The typed commands IRC users expect (`/server -m`, `/list`, `/describe`, `/notify`, `/close`, ...) | P1 | done (round 4; see `COMMANDS.md` for every command) |
| Unknown commands sent to the server as typed (`/lusers`, `/motd`, `/oper`, ...) | P1 | done (round 4) |

## Round 9: features the first list missed

A second pass over what experienced users expect of the panels, the notify
list, the input line, and the habits of a session, in OpenIRC's own words.

| Feature | Priority | Status |
|---|---|---|
| Channels list that opens when a server connects (Get list, join box with a delay, save, do-not-show option) | P2 | done (round 10) |
| Notify list that shows who is online and offline (MONITOR 730/731), with a notice | P1 | done (round 9; ISON every minute on servers without MONITOR) |
| Docked monitor panel along the bottom: highlights, private messages, notices, server messages; F2 shows or hides it | P2 | done (round 9) |
| Away automatically after some idle minutes; back when you type | P2 | done (round 9) |
| Reopen the channels and queries of the last session on connect | P2 | done (round 9) |
| Tab completes channels (`#op` → `#openirc`) and commands (`/jo` → `/join`) as well as nicks | P2 | done (round 9) |
| Input history kept between sessions | P3 | done (round 10; lines that may hold a password are never saved) |
| Custom right-click menus per window type (status, channel, query, nick list) | P2 | done (round 9) |
| Search every window at once | P2 | done (round 9; Tools menu, Ctrl+Shift+F) |
| Color the whole line in the speaker's nick color | P3 | done (round 9) |
| Custom title bar text | P3 | done (round 9) |
| Hide channel and status prefixes in the treebar and nick list | P3 | done (round 9) |
| Script editor syntax coloring | P3 | done (round 9) |
| Emoticon shortcuts (`:)` and friends) shown as emoji | P3 | not planned (the bundled fonts have no emoji; adding an emoji font would add megabytes) |
| Toolbar buttons chosen by the user | P3 | done (round 10; Options, Display) |
| Keep the computer awake during DCC transfers | P3 | done (round 10; Windows, and Linux with systemd) |
| Fonts per area, background images, transparency, plugins, undocking panels into windows | — | not planned (known limitations and the single-window design) |

## Round 11: a closer look at the options

The options a desktop IRC client is expected to offer were gone through once
more, one by one, for things neither list had covered.

| Feature | Priority | Status |
|---|---|---|
| Main window opens where it was closed: place, size, maximized | P2 | done (round 11; a place off every screen is left to the window manager) |
| Spell check the input line as you type, with suggestions and "Add to the dictionary" on right-click | P2 | done (round 11; the word still being typed is not marked) |
| Spell check leaves the channel's nicknames, the channel name, and your nick alone | P2 | done (round 11; transcript and input line) |
| Show the speaker's rank (`@`, `+`) before the nick in messages | P2 | done (round 11; Options, Display) |
| Choose the text around nicks (`<nick>`, `[nick]`, `nick:`) | P3 | done (round 11; Options, Display) |
| Load a script from a file and save one to a file | P2 | done (round 11; Scripts, Script files; a loaded script is not run until ticked) |
| Search the web for the word under the pointer (right-click), search address of your choice | P3 | done (round 11; Options, Messages) |
| Mouse back and forward buttons step through the windows | P3 | done (round 11) |
| Raw log window with its own buffer | P3 | done earlier as `/debug` (round 3) |
| Ignore repeated DCC resume requests, transfer buffer sizes, flash and tray rules per window | P3 | not planned unless someone asks |

## Round 12: other languages

Asked for after round 11: menus and dialogs in several languages. Chatting
in any language already worked (UTF-8, with a per-server encoding).

| Feature | Priority | Status |
|---|---|---|
| Spell check in any alphabet (accents, Cyrillic, Greek, Indian scripts), Hunspell `.dic` word lists | P2 | done (round 12; scripts written without spaces between words are left alone) |
| Chinese, Japanese, Korean, Thai, and Indian scripts shown instead of empty boxes | P2 | done (round 12; a font on the computer is found and loaded only when such text appears) |
| Menus, dialogs, options, and event lines in other languages, chosen in Options or following the computer | P2 | done (round 12; German, Spanish, French, Brazilian Portuguese built in; more by dropping a file in the settings folder) |
| Right-to-left text (Arabic, Hebrew) in the right order, with joined letters | P3 | not planned (the drawing library has no right-to-left layout) |
| Output of typed commands (`/help`, `/timer`, ...) in other languages | P3 | open (English for now) |

## Not planned

Speech output, DDE automation, sound requests between users, and a play
queue are tied to one platform or to old habits, and are not planned unless
someone asks.
