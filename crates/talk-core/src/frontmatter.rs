#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frontmatter {
    pub id: String,
    pub question: String,
    pub slug: String,
    pub pack: String,
    pub addressee: String,
    pub created: String, // YYYY-MM-DD
    pub entries: u32,
    pub last: String,    // YYYY-MM-DD
}

impl Frontmatter {
    /// Render as a `---`-delimited YAML block (trailing newline included).
    pub fn to_yaml(&self) -> String {
        format!(
            "---\nid: {id}\nquestion: {q}\nslug: {slug}\npack: {pack}\naddressee: {addr}\ncreated: {created}\nentries: {entries}\nlast: {last}\n---\n",
            id = self.id,
            q = quote(&self.question),
            slug = self.slug,
            pack = self.pack,
            addr = self.addressee,
            created = self.created,
            entries = self.entries,
            last = self.last,
        )
    }

    /// Parse the leading `---`-delimited block. Returns (frontmatter, rest_of_body).
    pub fn parse(input: &str) -> Option<(Frontmatter, &str)> {
        let rest = input.strip_prefix("---\n")?;
        let end = rest.find("\n---\n")?;
        let block = &rest[..end];
        let body = &rest[end + 5..];

        let mut map = std::collections::HashMap::new();
        for line in block.lines() {
            if let Some((k, v)) = line.split_once(": ") {
                map.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
        Some((
            Frontmatter {
                id: map.get("id")?.clone(),
                question: unquote(map.get("question")?),
                slug: map.get("slug")?.clone(),
                pack: map.get("pack")?.clone(),
                addressee: map.get("addressee")?.clone(),
                created: map.get("created")?.clone(),
                entries: map.get("entries")?.parse().ok()?,
                last: map.get("last")?.clone(),
            },
            body,
        ))
    }
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn unquote(s: &str) -> String {
    let trimmed = s.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(s);
    trimmed.replace("\\\"", "\"").replace("\\\\", "\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Frontmatter {
        Frontmatter {
            id: "avoidance-core".into(),
            question: "What am I avoiding?".into(),
            slug: "what-am-i-avoiding".into(),
            pack: "examen".into(),
            addressee: "self".into(),
            created: "2026-06-06".into(),
            entries: 3,
            last: "2026-06-08".into(),
        }
    }

    #[test]
    fn round_trips() {
        let fm = sample();
        let rendered = fm.to_yaml() + "\n## 2026-06-06\nbody text\n";
        let (parsed, body) = Frontmatter::parse(&rendered).unwrap();
        assert_eq!(parsed, fm);
        assert_eq!(body, "\n## 2026-06-06\nbody text\n");
    }

    #[test]
    fn quotes_questions_with_special_chars() {
        let mut fm = sample();
        fm.question = "Why \"this\": really?".into();
        let (parsed, _) = Frontmatter::parse(&(fm.to_yaml() + "\nx\n")).unwrap();
        assert_eq!(parsed.question, "Why \"this\": really?");
    }
}
