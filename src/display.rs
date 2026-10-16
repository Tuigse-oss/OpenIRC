use crate::pump::Event;

pub fn format_event(event: &Event) -> String {
    match event {
        Event::Registered { text, .. } => format!("* {text}"),
        Event::Chat {
            target,
            from,
            text,
            action,
            ..
        } => {
            if *action {
                format!("[{target}] * {from} {text}")
            } else {
                format!("[{target}] <{from}> {text}")
            }
        }
        Event::Notice { from, text } => format!("-{from}- {text}"),
        Event::UserAway { who, reason } => match reason {
            Some(reason) => format!("* {who} is away: {reason}"),
            None => format!("* {who} is back"),
        },
        Event::Account { who, account } => match account {
            Some(account) => format!("* {who} is logged in as {account}"),
            None => format!("* {who} logged out"),
        },
        Event::HostChange { who, address } => format!("* {who} is now {address}"),
        Event::InviteNotify { by, who, channel } => format!("* {by} invited {who} to {channel}"),
        Event::Typing { who, target, state } => format!("* {who} typing {state} in {target}"),
        Event::Sts { port, duration } => format!(
            "* the server asks for TLS{}{}",
            port.map(|port| format!(" on port {port}"))
                .unwrap_or_default(),
            duration
                .map(|secs| format!(" for {secs}s"))
                .unwrap_or_default()
        ),
        Event::Caps { enabled } => format!("* capabilities: {}", enabled.join(" ")),
        Event::Ctcp { from, command, .. } => format!("* CTCP {command} from {from}"),
        Event::CtcpReply {
            from,
            command,
            text,
        } => format!("* {from} {command} reply: {text}"),
        Event::Join { who, channel, .. } => format!("* {who} joined {channel}"),
        Event::ModeChange { by, channel, .. } => format!("* {by} changed modes in {channel}"),
        Event::Member { who, channel } => format!("* {who} is in {channel}"),
        Event::Invite { from, channel } => format!("* {from} invites you to {channel}"),
        Event::Network { name } => format!("* network {name}"),
        Event::Members { channel, names } => format!("* {channel}: {}", names.join(" ")),
        Event::ChannelModes { channel, modes } => format!("* {channel} modes {modes}"),
        Event::Part {
            who,
            channel,
            reason,
        } => match reason {
            Some(reason) if !reason.is_empty() => format!("* {who} left {channel} ({reason})"),
            _ => format!("* {who} left {channel}"),
        },
        Event::Quit { who, reason } => match reason {
            Some(reason) if !reason.is_empty() => format!("* {who} quit ({reason})"),
            _ => format!("* {who} quit"),
        },
        Event::Nick { old, new } => format!("* {old} is now {new}"),
        Event::Kick {
            who,
            channel,
            by,
            reason,
        } => match reason {
            Some(reason) if !reason.is_empty() => {
                format!("* {by} kicked {who} from {channel} ({reason})")
            }
            _ => format!("* {by} kicked {who} from {channel}"),
        },
        Event::Numeric { code, text } => format!("* {code:03} {text}"),
        Event::Topic { channel, topic } => match topic {
            Some(topic) if !topic.is_empty() => format!("* {channel} topic: {topic}"),
            _ => format!("* {channel} has no topic"),
        },
        Event::ChannelInfo { text, .. } => format!("* {text}"),
        Event::TopicChanged { by, channel, topic } => {
            format!("* {by} changes the {channel} topic to: {topic}")
        }
        Event::UserModes { modes } => format!("* your modes are {modes}"),
        Event::Notify { nick, online, .. } => {
            if *online {
                format!("* {nick} is online")
            } else {
                format!("* {nick} has left IRC")
            }
        }
        Event::Names { channel, names, .. } => {
            let list = if names.is_empty() {
                "empty".to_string()
            } else {
                names.join(" ")
            };
            format!("* {channel} names: {list}")
        }
        Event::DccChat { from, host, port } => {
            format!("* {from} offers DCC chat at {host}:{port}")
        }
        Event::DccSend {
            from,
            filename,
            host,
            port,
            size,
            token: _,
        } => match size {
            Some(size) => format!("* {from} offers {filename} ({size} bytes) at {host}:{port}"),
            None => format!("* {from} offers {filename} at {host}:{port}"),
        },
        Event::DccAccept {
            from,
            filename,
            port,
            position,
        } => format!("* {from} accepts resume of {filename} at {position} on port {port}"),
        Event::CaseMapping { map } => format!("* names compare as {map:?}"),
        Event::DccResume {
            from,
            filename,
            port,
            position,
        } => format!("* {from} asks to resume {filename} at {position} on port {port}"),
        Event::Away { reason } => match reason {
            Some(reason) => format!("* away: {reason}"),
            None => "* you are back".to_string(),
        },
        Event::Echo { text } => format!("* {text}"),
        Event::Clear => "* cleared".to_string(),
        Event::Ping => "* pong".to_string(),
        Event::Raw { text, .. } => format!("* {text}"),
        Event::Sent {
            target,
            text,
            action,
            from,
        } => {
            if *action {
                format!("[{target}] * {from} {text}")
            } else {
                format!("[{target}] <{from}> {text}")
            }
        }
    }
}
