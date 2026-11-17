# OpenIRC

[![check](https://github.com/Tuigse-oss/OpenIRC/actions/workflows/check.yml/badge.svg)](https://github.com/Tuigse-oss/OpenIRC/actions/workflows/check.yml)
[![CodeQL](https://github.com/Tuigse-oss/OpenIRC/actions/workflows/codeql.yml/badge.svg)](https://github.com/Tuigse-oss/OpenIRC/actions/workflows/codeql.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

OpenIRC is a native desktop IRC client written in Rust, for Windows and
Linux. One window holds your servers, channels and private chats in a
tree or a switchbar, with a nick list, a toolbar, and a docked panel for
highlights and private messages.

## Features

- **Connecting:** a server list with networks and groups; TLS, with a prompt
  to trust a certificate that fails verification (pinned by SHA-256 per
  host); SASL PLAIN and EXTERNAL, NickServ, or a server password; client
  certificates (CertFP), which OpenIRC can create; an alternative nickname;
  retry and next-server rules; per-server text encoding; proxies; identd.
- **Channels and messages:** a channel window for the topic, modes, key,
  limit, and ban, exception and invite lists; a channels list that opens
  when a server connects; a notify list that shows who is online.
- **Typing:** command history kept between sessions; Tab completion of
  nicks, channels and commands; formatting shortcuts and a color picker;
  spell check with suggestions; a multi-line editbox. Any `/command`
  OpenIRC does not handle itself goes to the server as typed. See
  [COMMANDS.md](COMMANDS.md).
- **Display:** color schemes and an editable 16-color palette; proportional
  or monospace transcripts; line shading; desktop notifications; tray
  options; the window reopens where you closed it.
- **Logs and URLs:** per-window logs with a log browser; a URL catcher;
  search across every window.
- **DCC:** chat, send and receive, with resume, passive mode, auto-accept
  from trusted users, and a transfer window.
- **Scripting:** events, aliases, popups, variables and timers, with a
  script editor that has syntax coloring, find and replace, and a debug
  view.
- **Languages:** English, German, Spanish, French and Brazilian
  Portuguese. See [lang/README.md](lang/README.md).

[FEATURES.md](FEATURES.md) lists every feature and its status.

## Install

On Windows, download `OpenIRC-v<version>-x86_64-windows.zip` from the
[latest release](https://github.com/Tuigse-oss/OpenIRC/releases/latest),
unzip it, and run `openirc.exe`; nothing is installed. On Linux, build it
from source (below). To remove OpenIRC, delete its folder. Its settings,
and by default its logs and DCC downloads, are in `%APPDATA%\openirc` on
Windows and `~/.config/openirc` on Linux, or in `$XDG_CONFIG_HOME/openirc`
if that is set; delete that folder too to remove them.

`openirc.exe` is not code-signed yet, so SmartScreen may warn when you run
it. Check the zip against the release's `SHA256SUMS.txt` and its build
provenance first:

```powershell
Get-FileHash .\OpenIRC-v0.1.0-x86_64-windows.zip   # compare with SHA256SUMS.txt
gh attestation verify OpenIRC-v0.1.0-x86_64-windows.zip --repo Tuigse-oss/OpenIRC
```

## Building

You need a current stable Rust toolchain (`rustup` installs one).

```bash
cargo build --release
./target/release/openirc        # target\release\openirc.exe on Windows
```

`scripts/check.sh` runs every check a change must pass: formatting,
Clippy for Linux and Windows, the tests under a memory cap, `cargo audit`
and `cargo deny` against the RustSec database, and the benchmarks
compared with a baseline. CI runs it on every pull request, on every push
to `main`, and weekly.

## Security

OpenIRC talks to servers and to other people's clients (DCC), so parsing,
TLS and file handling are treated as security-relevant. To report a
vulnerability, see [SECURITY.md](SECURITY.md). [REVIEW.md](REVIEW.md)
records every review finding, its fix, and the test that covers it.

## Privacy

OpenIRC connects to the IRC servers, proxies and DCC peers you choose.
When you click **Update** in the network directory, it downloads a
public list of IRC networks from raw.githubusercontent.com over HTTPS,
not through your IRC proxy. Web links and searches open in your browser,
and scripts you add can run commands for you. Its ident server is off
unless you turn it on, and the program has no telemetry and never checks
for updates to itself.

## License

OpenIRC is licensed under either of the [MIT license](LICENSE-MIT) or the
[Apache License, Version 2.0](LICENSE-APACHE), at your option. Unless you
explicitly state otherwise, any contribution intentionally submitted for
inclusion in this project is licensed the same way.

The bundled DejaVu fonts are under their own license
([assets/DejaVu-LICENSE.txt](assets/DejaVu-LICENSE.txt)).

OpenIRC is a Tuigse LLC project.
