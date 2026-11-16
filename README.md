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

Releases are published on the
[Releases](https://github.com/Tuigse-oss/OpenIRC/releases) page. Until
then, build OpenIRC from source.

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

## License

OpenIRC is licensed under either of the [MIT license](LICENSE-MIT) or the
[Apache License, Version 2.0](LICENSE-APACHE), at your option. Unless you
explicitly state otherwise, any contribution intentionally submitted for
inclusion in this project is licensed the same way.

The bundled DejaVu fonts are under their own license
([assets/DejaVu-LICENSE.txt](assets/DejaVu-LICENSE.txt)).

OpenIRC is a Tuigse LLC project.
