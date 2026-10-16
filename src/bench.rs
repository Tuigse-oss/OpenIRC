//! Micro-benchmarks for the hot paths, built only with `--features bench`.
//! Run them with:
//!
//! ```text
//! cargo run --release --features bench --example bench
//! ```

use std::time::{Duration, Instant};

pub struct Result {
    pub name: &'static str,
    pub iterations: usize,
    pub elapsed: Duration,
    pub note: String,
}

impl Result {
    pub fn per_op(&self) -> Duration {
        self.elapsed / self.iterations.max(1) as u32
    }
}

fn time(name: &'static str, iterations: usize, mut work: impl FnMut(usize)) -> Result {
    let started = Instant::now();
    for index in 0..iterations {
        work(index);
    }
    Result {
        name,
        iterations,
        elapsed: started.elapsed(),
        note: String::new(),
    }
}

pub fn run() -> Vec<Result> {
    let mut results = Vec::new();

    let mut pump = crate::pump::Pump::new(crate::pump::Config::new(
        "me",
        "me",
        "Me",
        Vec::new(),
        Vec::new(),
    ));
    pump.on_server_line(":me!u@h JOIN #bench");
    results.push(time("pump: PRIVMSG line", 200_000, |index| {
        let line = format!(
            ":user{}!u@h PRIVMSG #bench :hello \u{3}04there\u{3} {index}",
            index % 50
        );
        std::hint::black_box(pump.on_server_line(&line));
    }));

    let mut wire = Vec::new();
    for index in 0..200_000 {
        wire.extend_from_slice(format!(":a!b@c PRIVMSG #x :line {index}\r\n").as_bytes());
    }
    results.push(time("wire: split and decode 200k lines", 1, |_| {
        let mut buffer = crate::message::LineBuffer::default();
        let mut count = 0;
        for chunk in wire.chunks(4096) {
            buffer.extend(chunk);
            while let Some(raw) = buffer.next_line() {
                std::hint::black_box(crate::message::decode_line(raw));
                count += 1;
            }
        }
        assert_eq!(count, 200_000);
    }));
    results.push(time("wire: one 7 MB burst, 200k lines", 1, |_| {
        let mut buffer = crate::message::LineBuffer::default();
        buffer.extend(&wire);
        let mut count = 0;
        while let Some(raw) = buffer.next_line() {
            std::hint::black_box(crate::message::decode_line(raw));
            count += 1;
        }
        assert_eq!(count, 200_000);
    }));

    let script = (0..40)
        .map(|n| {
            format!(
                "on *:TEXT:*word{n}*:#:{{\n  if ($nick == x{n}) {{\n    msg $chan hi\n  }}\n}}\n"
            )
        })
        .collect::<String>();
    let fields = crate::slang::Fields {
        nick: "someone",
        chan: "#bench",
        me: "me",
        text: "nothing matches here",
        time: "12:00",
        args: &["nothing", "matches", "here"],
        nicks: &[],
        levels: &[],
    };
    let mut vars = Vec::new();
    let mut cache = crate::slang::Cache::default();
    results.push(time("slang: event, cached parse", 50_000, |_| {
        let program = cache.program(&script).unwrap();
        std::hint::black_box(crate::slang::run_event_in(
            &program, "text", &fields, &mut vars,
        ));
    }));
    results.push(time("slang: event, parse each time", 50_000, |_| {
        let program = crate::slang::Cache::default().program(&script).unwrap();
        std::hint::black_box(crate::slang::run_event_in(
            &program, "text", &fields, &mut vars,
        ));
    }));

    let remotes: Vec<crate::script::Remote> = (0..20)
        .map(|n| crate::script::Remote {
            event: "text".into(),
            mask: format!("*w{n}*"),
            body: "/notice $nick you said $text | /echo %count".into(),
        })
        .collect();
    let remote_fields = crate::script::Fields {
        nick: "someone",
        chan: "#bench",
        me: "me",
        text: "w1 w2 w3 hello",
        time: "12:00",
    };
    let remote_vars = vec![("count".to_string(), "3".to_string())];
    results.push(time("remotes: 20 masks", 100_000, |_| {
        std::hint::black_box(crate::script::commands(
            &remotes,
            &remote_vars,
            "text",
            &remote_fields,
            " | ",
        ));
    }));

    results.push(time("urls: find in a chat line", 500_000, |_| {
        std::hint::black_box(crate::urls::find_urls(
            "see https://example.com/a and http://irc.test/b, ok",
        ));
    }));

    for (name, lines) in [
        ("old transcript loop: 5000 lines", 5_000),
        ("old transcript loop: 200 lines", 200),
    ] {
        results.push(frame_result(
            name,
            crate::ui::bench_legacy_transcript(lines, 120),
        ));
    }
    for (name, lines, grow, find, resize) in [
        ("ui frame: 5000 lines, idle", 5_000, false, "", false),
        (
            "ui frame: 5000 lines, a line per frame",
            5_000,
            true,
            "",
            false,
        ),
        (
            "ui frame: 5000 lines, Find active",
            5_000,
            false,
            "message 4",
            false,
        ),
        (
            "ui frame: 5000 lines, resized every frame",
            5_000,
            false,
            "",
            true,
        ),
        ("ui frame: 200 lines, idle", 200, false, "", false),
    ] {
        results.push(frame_result(
            name,
            crate::ui::bench_frames(lines, 120, grow, find, resize),
        ));
    }
    results.push(frame_result(
        "ui frame: 2000 people in the channel",
        crate::ui::bench_frames_with(200, 2_000, 120, false, "", false, 0),
    ));
    results.push(frame_result(
        "ui frame: channels list of 20000",
        crate::ui::bench_frames_with(200, 300, 120, false, "", false, 20_000),
    ));
    results
}

fn frame_result(name: &'static str, mut frames: Vec<Duration>) -> Result {
    let total: Duration = frames.iter().sum();
    frames.sort_unstable();
    Result {
        name,
        iterations: frames.len(),
        elapsed: total,
        note: format!(
            "p50 {} p95 {} max {}",
            crate::profile::millis(crate::profile::percentile(&frames, 50)),
            crate::profile::millis(crate::profile::percentile(&frames, 95)),
            crate::profile::millis(*frames.last().unwrap_or(&Duration::ZERO)),
        ),
    }
}
