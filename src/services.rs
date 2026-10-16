//! WHO and NickServ command lines.
//!
//! These are ordinary IRC messages. NickServ is addressed by name; the
//! network decides whether that service exists.

use crate::message::Message;

pub fn who_line(target: &str) -> String {
    Message::command("WHO", [target]).encode()
}

pub fn identify_line(password: &str) -> String {
    Message::command(
        "PRIVMSG",
        ["NickServ".to_string(), format!("IDENTIFY {password}")],
    )
    .encode()
}

pub fn nickserv_line(text: &str) -> String {
    Message::command("PRIVMSG", ["NickServ".to_string(), text.to_string()]).encode()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn who_and_identify_are_normal_irc() {
        assert_eq!(who_line("#openirc"), "WHO :#openirc");
        assert_eq!(identify_line("secret"), "PRIVMSG NickServ :IDENTIFY secret");
        assert_eq!(nickserv_line("INFO bob"), "PRIVMSG NickServ :INFO bob");
    }
}
