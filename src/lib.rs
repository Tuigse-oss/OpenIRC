//! OpenIRC is a native Rust IRC client.
//!
//! A session starts by registering with the server, then pumps server and
//! user lines until quit or disconnect.

mod awake;
mod away;
mod beep;
#[cfg(feature = "bench")]
#[doc(hidden)]
pub mod bench;
#[cfg(any(test, feature = "bench"))]
#[doc(hidden)]
pub mod benchcmp;
mod certs;
mod cli;
mod clock;
mod colors;
mod connection;
mod dcc;
mod dcc_send;
mod dialog;
mod display;
mod flood;
mod fonts;
mod highlight;
mod i18n;
mod icon_art;
mod ident;
mod message;
mod net;
mod networks;
mod profile;
mod pump;
mod rules;
mod script;
mod servers;
mod services;
mod settings;
mod slang;
mod socks;
mod spell;
mod state;
mod theme;
mod tls;
mod tray;
mod ui;
mod urls;
mod wav;

use std::io::{self, BufRead, Write};
use std::process::ExitCode;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

pub use cli::{Args, parse_args};
pub use message::Message;
pub use pump::{Config, Effect, Event, Pump};

pub const NAME: &str = "OpenIRC";

pub fn entry() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let cli = args.iter().any(|arg| arg == "--cli");
    args.retain(|arg| arg != "--cli");
    if cli
        || args
            .iter()
            .any(|arg| matches!(arg.as_str(), "--help" | "-h" | "--version" | "-V"))
    {
        return run_from(args);
    }
    if args.is_empty() {
        return window_exit(None);
    }
    match parse_args(args) {
        Ok(parsed) => window_exit(Some(parsed)),
        Err(cli::ArgsError::Help(text) | cli::ArgsError::Version(text)) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(cli::ArgsError::Invalid(text)) => {
            eprintln!("{NAME}: {text}");
            eprintln!("Run `{NAME_BIN} --help` for usage.", NAME_BIN = cli::BIN);
            ExitCode::from(2)
        }
    }
}

fn window_exit(prefill: Option<Args>) -> ExitCode {
    match ui::run(prefill) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{NAME}: {err}");
            ExitCode::from(1)
        }
    }
}

pub fn run() -> ExitCode {
    run_from(std::env::args().skip(1).collect())
}

fn run_from(args: Vec<String>) -> ExitCode {
    match parse_args(args) {
        Ok(args) => match run_session(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("{NAME}: {err}");
                ExitCode::from(1)
            }
        },
        Err(cli::ArgsError::Help(text) | cli::ArgsError::Version(text)) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(cli::ArgsError::Invalid(text)) => {
            eprintln!("{NAME}: {text}");
            eprintln!("Run `{NAME_BIN} --help` for usage.", NAME_BIN = cli::BIN);
            ExitCode::from(2)
        }
    }
}

fn run_session(args: Args) -> io::Result<()> {
    let config = Config::new(
        args.nick.clone(),
        args.user.clone(),
        args.realname.clone(),
        args.channels.clone(),
        Vec::new(),
    );
    let stream = connection::connect(
        &args.host,
        args.port,
        args.tls,
        None,
        &connection::ConnectOptions::default(),
    )?;
    let mut conn = connection::Connection::new(stream, config);
    conn.register()?;

    println!("{NAME} {}", env!("CARGO_PKG_VERSION"));
    println!(
        "nick {} on {}:{}{}",
        args.nick,
        args.host,
        args.port,
        if args.tls { " (tls)" } else { "" }
    );
    println!("Type /help for commands.");

    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(line) => {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    loop {
        if let Some(effect) = conn.read_effect()? {
            render(&effect);
            if effect.quit {
                break;
            }
        }
        loop {
            match rx.try_recv() {
                Ok(line) => {
                    let effect = conn.handle_user(&line)?;
                    render(&effect);
                    if effect.quit {
                        return Ok(());
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
            }
        }
    }
    Ok(())
}

fn render(effect: &Effect) {
    for line in &effect.lines {
        println!("{line}");
    }
    let _ = io::stdout().flush();
}

pub(crate) fn idle_timeout() -> Duration {
    Duration::from_millis(100)
}
