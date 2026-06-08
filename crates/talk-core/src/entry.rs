/// One recorded turn, already cleaned by `cleanup`.
pub struct Entry<'a> {
    pub date: &'a str, // YYYY-MM-DD
    pub time: &'a str, // HH:MM
    pub raw: Option<&'a str>, // None when keep_raw = false or ephemeral
    pub clean: &'a str,
}

/// Reflect files hold many dates in one question-file, so their sections are
/// dates (`## 2026-06-06`); journal files are one-per-day, so their sections are
/// times (`## 08:14`) per spec §8.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode { Reflect, Journal }

/// Append `entry` to `body` (the part after frontmatter), returning the new body.
/// - Reflect: a new date appends `## DATE`; a repeat date nests `### HH:MM`.
/// - Journal: every turn appends `## HH:MM` (the file is already one date).
pub fn append(body: &str, entry: &Entry, mode: Mode) -> String {
    let block = render_turn(entry);
    match mode {
        Mode::Journal => join(body, &format!("\n## {}\n{}", entry.time, block)),
        Mode::Reflect => {
            let date_header = format!("## {}", entry.date);
            if body.contains(&date_header) {
                join(body, &format!("\n### {}\n{}", entry.time, block))
            } else {
                join(body, &format!("\n{}\n{}", date_header, block))
            }
        }
    }
}

fn render_turn(entry: &Entry) -> String {
    match entry.raw {
        Some(raw) => format!("<!-- raw: {} -->\n{}\n", sanitize_comment(raw), entry.clean),
        None => format!("{}\n", entry.clean),
    }
}

/// Neutralize anything that could break out of the HTML comment: the `--`
/// digraph (which terminates/malforms comments) and `<` (a nested `<!--`).
fn sanitize_comment(raw: &str) -> String {
    raw.replace('<', "&lt;")
        .replace("--", "&#45;&#45;")
        .replace('\n', " ")
}

fn join(body: &str, section: &str) -> String {
    format!("{}\n{}", body.trim_end_matches('\n'), section.trim_start_matches('\n'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn<'a>(date: &'a str, time: &'a str, raw: Option<&'a str>, clean: &'a str) -> Entry<'a> {
        Entry { date, time, raw, clean }
    }

    #[test]
    fn reflect_new_date_appends_a_dated_section() {
        let out = append("", &turn("2026-06-06", "08:14", Some("um the thing is"), "The thing is."), Mode::Reflect);
        assert!(out.contains("## 2026-06-06"));
        assert!(out.contains("<!-- raw: um the thing is -->"));
        assert!(out.contains("The thing is."));
    }

    #[test]
    fn reflect_repeat_date_nests_a_timestamped_subsection() {
        let body = "\n## 2026-06-06\nFirst.\n";
        let out = append(body, &turn("2026-06-06", "20:15", None, "Second."), Mode::Reflect);
        assert!(out.contains("### 20:15"));
        assert_eq!(out.matches("## 2026-06-06").count(), 1);
    }

    #[test]
    fn journal_sections_are_time_keyed() {
        let out = append("", &turn("2026-06-08", "08:14", None, "Morning."), Mode::Journal);
        assert!(out.contains("## 08:14") && !out.contains("## 2026-06-08"));
        let out2 = append(&out, &turn("2026-06-08", "21:30", None, "Night."), Mode::Journal);
        assert!(out2.contains("## 08:14") && out2.contains("## 21:30"));
    }

    #[test]
    fn raw_none_omits_the_comment() {
        let out = append("", &turn("2026-06-06", "08:14", None, "Clean only."), Mode::Reflect);
        assert!(!out.contains("<!-- raw"));
    }

    #[test]
    fn comment_breakout_chars_are_neutralized() {
        let out = append("", &turn("2026-06-06", "08:14", Some("end --> <!-- x"), "y"), Mode::Reflect);
        assert_eq!(out.matches("-->").count(), 1); // only the comment's own terminator
        assert!(!out.contains("<!-- x"));
    }
}
