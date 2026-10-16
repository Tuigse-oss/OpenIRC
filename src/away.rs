use crate::message::Message;

/// The reason stored for the status bar. Blank or whitespace-only input
/// becomes the word `away`. Otherwise the trimmed reason.
pub fn display_reason(reason: &str) -> String {
    let trimmed = reason.trim();
    if trimmed.is_empty() {
        String::from("away")
    } else {
        trimmed.to_string()
    }
}

/// One IRC line that marks the user away. Empty input uses the reason `away`,
/// so the line is `AWAY :away`. A real reason is `AWAY :lunch`.
pub fn away_line(reason: &str) -> String {
    Message::command("AWAY", [display_reason(reason)]).encode()
}

/// One IRC line that clears away. RFC 2812 uses AWAY with no parameter: `AWAY`.
pub fn back_line() -> String {
    Message::command("AWAY", Vec::<String>::new()).encode()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn away_line_with_reason() {
        assert_eq!(away_line("lunch"), "AWAY :lunch");
    }

    #[test]
    fn away_line_with_blank_reason() {
        assert_eq!(away_line("  "), "AWAY :away");
    }

    #[test]
    fn back_line_has_no_params() {
        assert_eq!(back_line(), "AWAY");
    }

    #[test]
    fn display_reason_trims() {
        assert_eq!(display_reason(" lunch "), "lunch");
    }

    #[test]
    fn display_reason_blank_is_away() {
        assert_eq!(display_reason(""), "away");
    }
}
