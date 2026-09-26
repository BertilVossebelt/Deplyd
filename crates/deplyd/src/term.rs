//! Terminal shape: how wide it is, and whether it can take a link.

use anstyle::{AnsiColor, Color, Style};

pub const CYAN: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Cyan)));
pub const GREEN: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Green)));
pub const YELLOW: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Yellow)));
pub const RED: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Red)));
pub const DIM: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::BrightBlack)));
pub const BOLD: Style = Style::new().bold();

/// The column values line up in, so a verdict and the fields under it share an edge.
pub const FIELD: usize = 10;

/// Marks beside a verdict. Tied to colour support, which is the same question as
/// "is a person reading this": a pipe or NO_COLOR gets plain ASCII.
pub struct Glyphs {
    pub ok: &'static str,
    pub warn: &'static str,
    pub bad: &'static str,
    pub dot: &'static str,
}

pub fn glyphs() -> Glyphs {
    if links_supported() {
        Glyphs {
            ok: "\u{2713}",
            warn: "!",
            bad: "\u{2717}",
            dot: "\u{b7}",
        }
    } else {
        Glyphs {
            ok: "+",
            warn: "!",
            bad: "x",
            dot: "-",
        }
    }
}

/// Used when the width is unknown, which is what piping into a file looks like.
const ASSUMED_WIDTH: usize = 100;

/// How wide the terminal is, or nothing when output is not going to one. Nothing
/// is truncated in that case: a redirected report has no width to exceed.
pub fn width() -> Option<usize> {
    // COLUMNS wins, so a narrow report can be asked for and tested.
    if let Some(columns) = std::env::var_os("COLUMNS")
        && let Some(parsed) = columns.to_str().and_then(|text| text.trim().parse().ok())
        && parsed > 0
    {
        return Some(parsed);
    }
    terminal_size::terminal_size().map(|(terminal_size::Width(w), _)| w as usize)
}

/// Cuts text to fit, marking where it was cut.
pub fn truncate(text: &str, room: usize) -> String {
    if room == 0 {
        return String::new();
    }
    let characters: Vec<char> = text.chars().collect();
    if characters.len() <= room {
        return text.to_string();
    }
    if room == 1 {
        return "…".to_string();
    }
    let kept: String = characters[..room - 1].iter().collect();
    format!("{}…", kept.trim_end())
}

/// Truncates to whatever room is left on the line after `used` columns.
pub fn fit(text: &str, used: usize) -> String {
    match width() {
        Some(total) => fit_to(text, used, total),
        // Not a terminal, so nothing wraps.
        None => text.to_string(),
    }
}

pub fn fit_to(text: &str, used: usize, total: usize) -> String {
    let room = total.saturating_sub(used);
    if room == 0 {
        // A blank column is worse than a wrapped one.
        return truncate(text, ASSUMED_WIDTH.saturating_sub(used).max(8));
    }
    truncate(text, room)
}

/// Whether links are worth emitting. Off when colour is off, which answers piping,
/// dumb terminals and NO_COLOR at once. Asked once per process.
pub fn links_supported() -> bool {
    static SUPPORTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        matches!(
            anstream::AutoStream::choice(&std::io::stdout()),
            anstream::ColorChoice::Always | anstream::ColorChoice::AlwaysAnsi
        )
    })
}

/// An OSC 8 hyperlink, or the text alone where that would be noise.
pub fn link(text: &str, url: &str) -> String {
    if url.is_empty() || !links_supported() {
        return text.to_string();
    }
    format!("\x1b]8;;{url}\x1b\\{text}\x1b]8;;\x1b\\")
}

/// Where a repository lives on the web, for building links into it.
#[derive(Debug, Clone, Default)]
pub struct WebBase(pub String);

impl WebBase {
    pub fn pull_request(&self, number: u32) -> String {
        if self.0.is_empty() {
            return String::new();
        }
        format!("{}/pull/{number}", self.0)
    }

    pub fn commit(&self, sha: &str) -> String {
        if self.0.is_empty() {
            return String::new();
        }
        format!("{}/commit/{sha}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_marks_where_it_cut() {
        assert_eq!(truncate("short", 20), "short");
        assert_eq!(truncate("exactly-ten", 11), "exactly-ten");
        assert_eq!(truncate("a longer subject line", 10), "a longer…");
    }

    #[test]
    fn truncation_handles_the_awkward_widths() {
        assert_eq!(truncate("anything", 0), "");
        assert_eq!(truncate("anything", 1), "…");
        assert_eq!(truncate("", 10), "");
    }

    #[test]
    fn truncation_counts_characters_not_bytes() {
        // A subject with accents must not be cut mid-character.
        let text = "café-naïve-über-lang";
        let cut = truncate(text, 8);
        assert!(cut.chars().count() <= 8, "got {cut:?}");
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn fitting_leaves_room_for_what_comes_before_it() {
        // 80 wide, 20 already used, so 60 characters of room.
        let text = "x".repeat(100);
        let fitted = fit_to(&text, 20, 80);
        assert_eq!(fitted.chars().count(), 60);
        assert!(fitted.ends_with('…'));
    }

    #[test]
    fn a_line_that_already_fits_is_untouched() {
        assert_eq!(fit_to("short", 20, 80), "short");
    }

    #[test]
    fn an_overfull_prefix_still_shows_something() {
        // The columns before the title can exceed the terminal on their own. Showing
        // a sliver beats showing an empty column.
        let fitted = fit_to("a subject", 200, 80);
        assert!(!fitted.is_empty());
    }

    #[test]
    fn a_link_falls_back_to_plain_text_without_a_url() {
        assert_eq!(link("PR #1", ""), "PR #1");
    }

    #[test]
    fn web_urls_are_built_from_the_remote() {
        let base = WebBase("https://github.com/acme/widgets".into());
        assert_eq!(
            base.pull_request(412),
            "https://github.com/acme/widgets/pull/412"
        );
        assert_eq!(
            base.commit("abc123"),
            "https://github.com/acme/widgets/commit/abc123"
        );
        assert!(WebBase::default().pull_request(1).is_empty());
    }
}
