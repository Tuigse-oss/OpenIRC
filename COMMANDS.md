# Typed commands

The commands IRC users type, and what OpenIRC does with each, in OpenIRC's
own words.

Status: **had** (before round 4), **added** (round 4, 2026-10-13),
**server** (sent to the server as typed; see below), **script** (part of
the script language, inside `alias` and `on` blocks), **not planned**.

Any `/command` OpenIRC does not handle itself goes to the server as
`COMMAND arguments`, as most IRC clients do. Server commands such
as `/lusers`, `/motd`, `/stats`, `/links`, `/map`, `/time`, `/info`,
`/version`, `/oper`, `/kill`, `/wallops`, `/userhost`, `/ison`, `/setname`,
`/monitor`, and `/rules` therefore work without OpenIRC knowing them. Before
round 4 they were refused as unknown commands.

`/help` lists the commands in the window.

## Connections

| Command | What it does | Status |
|---|---|---|
| `/server host [[+]port] [password]` | Move the current connection to another server; its channel and query windows close. A saved server (by address or network name) brings its login, encoding, and channels | added |
| `/server -m host ...` | Open another connection beside the current ones | added |
| `/server -e`, `+port`, `-p password`, `-j #channels` | TLS, server password, channels to join | added |
| `/server` | Connect the current server again | added |
| `/disconnect` | Leave the current server without closing its windows | added |
| `/quit [reason]` | Leave the current server with a reason | had |
| `/exit` | Close OpenIRC | added |
| `/scon -a <command>`, `/scid -a <command>` | Run a command on every connected server | added |
| `/nick`, `/anick`, `/fullname` | Nick now; alternative nick and real name for the next connect | had / added / added |
| `/away [reason]`, `/back` | Set or clear away | had |
| `/identd`, `/localinfo`, `/firewall`, `/emailaddr` | Change ident, local address, and proxy settings by command | not planned (Options, Network and Connect pages) |
| `/dns <host or nick>` | Look up an address; a nick uses the host seen in a channel | added |

## Channels and people

| Command | What it does | Status |
|---|---|---|
| `/join #a[,#b] [key]` | Join, with keys; a name without `#` gets one | had (keys and lists added) |
| `/part`, `/hop`, `/partall` | Leave one channel, rejoin it, leave every channel | had / had / added |
| `/list [filter]` | Open the Channels list and ask the server | added |
| `/channel [#chan]` | Open Channel central | had (round 3) |
| `/names`, `/topic`, `/mode`, `/kick`, `/invite`, `/knock` | Channel basics | had |
| `/op`, `/deop`, `/voice`, `/devoice`, `/halfop`, `/owner`, `/admin` (with a nick) | Give or take channel status | had |
| `/admin` (no nick) | Ask the server who runs it | added |
| `/ban [#chan] nick`, `/unban`, `/except` | Bans and exceptions | had |
| `/ban -u<seconds> ...` | A ban that lifts itself | added |
| `/banlist`, `/key`, `/limit`, `/umode`, `/silence` | Lists and modes | had |
| `/who`, `/whois`, `/whowas` | Who is who | had |
| `/uwho` | Show someone's details in a window | not planned (whois covers it) |
| `/ignore`, `/unignore` | Ignore rules with types and timeouts | had |
| `/notify [-r] [nick]` | Show, add to, or remove from the notify list | added |
| `/auser <levels> <mask>`, `/ruser <mask>`, `/ulist` | User levels for script events | added |
| `/guser`, `/iuser`, `/rlevel`, `/flush` | More user-level editing | not planned (Options, Users) |

## Messages

| Command | What it does | Status |
|---|---|---|
| `/msg`, `/query`, `/notice`, `/me` | Talk | had |
| `/say <text>` | Say text in the current window, even if it starts like a command | added |
| `/describe <target> <action>` | An action to any channel or nick | added |
| `/via <target> <line>` | Run one line (text or a command) for a channel or nick. The talk target is put back afterwards, unless the line itself chooses one (`/query`, `/target`, `/win`, `/join`), which then stays. A line typed in a window still goes to that window | added (round 24) |
| `/amsg`, `/ame` | A message or action to every channel | added / had |
| `/omsg`, `/onotice [#chan] <text>` | To the channel's operators only | added |
| `/ctcp <nick> <command>` | A CTCP request (PING shows the round trip) | had |
| `/ctcpreply <nick> <command> [text]` | Answer a CTCP request by hand | added |
| `/ping <nick>` | CTCP PING | added |
| `/nickserv`, `/ns`, `/identify` | NickServ | had |
| `/chanserv` `/cs`, `/memoserv` `/ms`, `/hostserv` `/hs`, `/botserv` `/bs`, `/operserv` `/os` | Other services | added |
| `/raw`, `/quote` | A raw line | had |
| `/dcc send <nick> <file>`, `/dcc chat <nick>` | Files and direct chat | had (round 3) |
| `/play <file>` | Say each line of a text file, paced by the send queue (500 lines at most) | added |
| `/sound`, `/splay`, `/speak` | Sound requests and speech | not planned |
| `/dccserver`, `/fserve` | DCC server and file server | not planned |

## Windows

| Command | What it does | Status |
|---|---|---|
| `/win`, `/window`, `/target` | Switch windows or the talk target | had |
| `/close [window]`, `/close -m`, `/closemsg <nick>` | Close a window, every query, or one query | added |
| `/clear`, `/clearall` | Empty this window or all of them | had / added |
| `/echo [-s] [window] <text>` | Show text in this, the status, or a named window | had (`-s` and window added) |
| `/url` | Open the URL list | added |
| `/savebuf [file]` | Save this window's lines to a file | added |
| `/editbox <text>` | Put text in the input line | added |
| `/beep`, `/flash [text]`, `/tip <text>` | Get attention | added |
| `/debug [on\|off]` | Show raw lines and script events | had (round 3) |
| `/titlebar`, `/background`, `/font`, `/color`, `/linesep` | Change looks by command | not planned (Options) |
| `/window @name`, `/aline`, `/dline`, `/iline`, `/rline`, `/sline`, `/cline` | Script-drawn custom windows | not planned |

## Settings, logs, and scripts

| Command | What it does | Status |
|---|---|---|
| `/log on\|off` | Turn logging on or off | added |
| `/logview` | Open the log browser | not planned (Tools, Log files) |
| `/alias [name [command]]` | List, add, or remove a quick alias | added |
| `/timer <secs> <repeats> <command>`, `/timers [off]` | Run later; list or stop timers | had / added |
| `/set`, `/unset` | Variables | had |
| `/inc`, `/dec`, `/hadd`, `/hdel`, `/hfree`, `/return`, `/halt`, `if`, `while` | Script statements | script |
| `/load <file>`, `/unload <name>` | Add a script file to the script list or remove one | added |
| `/reload`, `/remote on\|off`, `/events on\|off`, `/ctcps on\|off` | Reload and switch scripts off | not planned (the editor's tick boxes) |
| `/dialog`, `/did` | Script dialogs | had (`/dialog`) / not planned (`/did`) |
| `/flood <messages> <joins> <seconds> [ctcp]` | Incoming flood limits | had |
| `/run <address or file>` | Open with the system's handler | added |
| `/write`, `/writeini`, `/remini`, `/fopen` and other file commands | Script file access | not planned (scripts can read a line with `$read(file, n)`: a plain file up to 1 MiB, by full path or relative to the settings folder; nothing else) |
| `/sockopen` and other socket commands, `/dll`, `/com` | Script networking and native calls | not planned (unsafe from scripts) |
