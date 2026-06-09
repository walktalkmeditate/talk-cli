use crate::settle::Settle;

/// Which mode's chrome to show.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode { Reflect, Journal, Ephemeral }

/// The tone a line paints in: Settled = bright core text, Edge = dim live edge,
/// Chrome = the dimmest border/hint/status tone.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LineKind { Chrome, Settled, Edge }

/// Everything the screen needs, with no I/O. The live session rebuilds this each
/// frame from the Settle machine + clock + listening flag.
pub struct View<'a> {
    pub mode: Mode,
    pub question: Option<&'a str>, // reflect only
    pub held_label: Option<&'a str>, // e.g. "held 3 days"; None hides the box line
    pub settle: &'a Settle,
    pub listening: bool,           // VAD currently hears speech
    pub elapsed: &'a str,          // "2:14"
    pub cleanup: &'a str,          // "Light"
    pub show_raw: bool,            // `u` toggle: show raw verbatim instead of clean
    pub paused: bool,              // `p` toggle: timer frozen, source not drained
    pub confirm_cancel: bool,      // esc pressed once: showing the discard prompt
}

/// Compose the full screen as (line, tone) pairs (top to bottom). Pure — unit-testable.
pub fn compose(v: &View) -> Vec<(String, LineKind)> {
    let mut out: Vec<(String, LineKind)> = Vec::new();
    out.push((header_line(v), LineKind::Chrome));
    out.push((String::new(), LineKind::Chrome));

    if let (Mode::Reflect, Some(q)) = (v.mode, v.question) {
        if let Some(h) = v.held_label {
            out.push((format!("┌─ {} ", h) + &"─".repeat(60), LineKind::Chrome));
        } else {
            out.push(("┌".to_string() + &"─".repeat(64), LineKind::Chrome));
        }
        out.push((format!("│  {}", q), LineKind::Chrome));
        out.push(("└".to_string() + &"─".repeat(64), LineKind::Chrome));
        out.push((String::new(), LineKind::Chrome));
    }
    if v.mode == Mode::Ephemeral {
        out.push(("Say it. This keeps nothing.".to_string(), LineKind::Chrome));
        out.push((String::new(), LineKind::Chrome));
    }

    // Settled blocks (bright/locked), then the committing block, then the edge.
    let mut body = 0usize;
    for b in v.settle.settled() {
        out.push((if v.show_raw { b.raw.clone() } else { b.clean.clone() }, LineKind::Settled));
        body += 1;
    }
    if let Some(c) = v.settle.committing() {
        out.push((if v.show_raw { c.raw.clone() } else { c.clean.clone() }, LineKind::Settled));
        body += 1;
    }
    // Empty body, not listening: a single dim placeholder keeps the region deterministic.
    if body == 0 && !v.listening {
        out.push(("  …".to_string(), LineKind::Edge));
    }
    out.push((String::new(), LineKind::Chrome));
    out.push((edge_line(v), LineKind::Edge));
    out.push(("─".repeat(66), LineKind::Chrome));
    out.push((status_line(v), LineKind::Chrome));
    out
}

fn header_line(v: &View) -> String {
    let label = match v.mode {
        Mode::Reflect => "talk · reflect",
        Mode::Journal => "talk · journal",
        Mode::Ephemeral => "talk · unburden",
    };
    let privacy = if v.mode == Mode::Ephemeral {
        "● local · no network · ✦ nothing saved"
    } else {
        "● local · no network"
    };
    format!("{}{}{}", label, " ".repeat(privacy_gap(label, privacy)), privacy)
}

fn privacy_gap(label: &str, privacy: &str) -> usize {
    66usize.saturating_sub(label.chars().count() + privacy.chars().count()).max(2)
}

fn edge_line(v: &View) -> String {
    // Settle-on-pause: the live edge is a listening indicator, not partials.
    if v.listening { "  …".to_string() } else { String::new() }
}

fn status_line(v: &View) -> String {
    if v.confirm_cancel {
        return "discard this reflection? [y] yes · [n] keep going".to_string();
    }
    if v.paused {
        return format!("⏸ paused  {}   {}    [p] resume · [space] done · esc cancel", v.elapsed, v.cleanup);
    }
    let dot = if v.listening { "● listening" } else { "○ ready" };
    match v.mode {
        Mode::Ephemeral => format!("{}  {}   ✦ ephemeral    [space] release · esc cancel", dot, v.elapsed),
        _ => format!(
            "{}  {}   {}    [space] done · u raw⇄clean · p pause · esc cancel",
            dot, v.elapsed, v.cleanup
        ),
    }
}

/// The closing screen after `[space]` in reflect/journal.
pub fn compose_close(path: &str, provenance: &str, phrase: &str) -> Vec<String> {
    vec![
        format!("  → {}     {}", path, provenance),
        format!("  \"{}\"", phrase),
    ]
}

/// The ephemeral release screen.
pub fn compose_released() -> Vec<String> {
    vec!["Released. Nothing was written.".to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settle::Settle;

    /// Join just the line text (drops the LineKind) for `.contains` asserts.
    fn text(v: &View) -> String {
        compose(v).iter().map(|(s, _)| s.clone()).collect::<Vec<_>>().join("\n")
    }

    fn settled_one() -> Settle {
        let mut s = Settle::new();
        s.commit("um the raw words", "The clean words.");
        s.finalize();
        s
    }

    fn base<'a>(mode: Mode, settle: &'a Settle) -> View<'a> {
        View {
            mode, question: None, held_label: None, settle, listening: false,
            elapsed: "0:01", cleanup: "Light", show_raw: false,
            paused: false, confirm_cancel: false,
        }
    }

    #[test]
    fn reflect_shows_question_box_and_settled_text() {
        let s = settled_one();
        let mut v = base(Mode::Reflect, &s);
        v.question = Some("What am I avoiding?");
        v.held_label = Some("held 3 days");
        v.elapsed = "2:14";
        let joined = text(&v);
        assert!(joined.contains("talk · reflect") && joined.contains("● local · no network"));
        assert!(joined.contains("What am I avoiding?"));
        assert!(joined.contains("held 3 days"));
        assert!(joined.contains("The clean words."));
        assert!(joined.contains("[space] done"));
    }

    #[test]
    fn raw_toggle_shows_verbatim() {
        let s = settled_one();
        let mut v = base(Mode::Reflect, &s);
        v.question = Some("Q?");
        v.show_raw = true;
        let joined = text(&v);
        assert!(joined.contains("um the raw words"));
        assert!(!joined.contains("The clean words."));
    }

    #[test]
    fn ephemeral_shows_keeps_nothing_chrome() {
        let s = Settle::new();
        let mut v = base(Mode::Ephemeral, &s);
        v.listening = true;
        v.elapsed = "0:48";
        let joined = text(&v);
        assert!(joined.contains("✦ nothing saved"));
        assert!(joined.contains("Say it. This keeps nothing."));
        assert!(joined.contains("[space] release"));
    }

    #[test]
    fn listening_flag_drives_the_indicator() {
        let s = Settle::new();
        let mk = |listening| {
            let mut v = base(Mode::Journal, &s);
            v.listening = listening;
            v.cleanup = "Medium";
            text(&v)
        };
        assert!(mk(true).contains("● listening"));
        assert!(mk(false).contains("○ ready"));
    }

    #[test]
    fn empty_state_renders_edge_and_status_without_panicking() {
        let s = Settle::new();
        let v = base(Mode::Reflect, &s); // settled+committing empty, not listening
        let lines = compose(&v);
        let joined = lines.iter().map(|(t, _)| t.clone()).collect::<Vec<_>>().join("\n");
        assert!(joined.contains("talk · reflect")); // chrome present
        assert!(joined.contains("○ ready"));        // status present
    }

    #[test]
    fn paused_status_renders_paused_marker() {
        let s = Settle::new();
        let mut v = base(Mode::Reflect, &s);
        v.paused = true;
        assert!(text(&v).contains("⏸ paused"));
    }

    #[test]
    fn confirm_cancel_renders_discard_prompt() {
        let s = Settle::new();
        let mut v = base(Mode::Reflect, &s);
        v.confirm_cancel = true;
        assert!(text(&v).contains("discard this reflection?"));
    }

    #[test]
    fn close_frame_shows_path_and_phrase() {
        let lines = compose_close("~/talk/what-am-i-avoiding.md", "entry 3 · held 3 days", "Stillness carries forward.");
        let joined = lines.join("\n");
        assert!(joined.contains("→ ~/talk/what-am-i-avoiding.md"));
        assert!(joined.contains("entry 3 · held 3 days"));
        assert!(joined.contains("Stillness carries forward."));
    }

    #[test]
    fn released_frame_is_the_keeps_nothing_line() {
        assert_eq!(compose_released(), vec!["Released. Nothing was written.".to_string()]);
    }
}
