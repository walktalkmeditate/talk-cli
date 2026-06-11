//! Personal-lexicon substitution (pure). The user authorizes these meaning
//! changes, so they run in the pre-layer BEFORE the content-word guard. Single
//! left-to-right pass over the original input: substituted output is never
//! re-scanned, so cyclic and value-contains-key maps terminate.

/// Apply word-bounded, case-insensitive substitutions. `corrections` MUST be
/// sorted by descending key length (longest-first), so the longest matching key
/// wins at each position. The value is emitted as written (later sentence-start
/// capitalization by `deterministic_light` may still re-case it — that is accepted).
pub fn apply_lexicon(text: &str, corrections: &[(String, String)]) -> String {
    if corrections.is_empty() {
        return text.to_string();
    }
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        let at_word_start = i == 0 || !b[i - 1].is_ascii_alphanumeric();
        let hit = if at_word_start {
            corrections.iter().find_map(|(key, val)| {
                let kb = key.as_bytes();
                let end = i + kb.len();
                if !kb.is_empty() && end <= b.len()
                    && b[i..end].eq_ignore_ascii_case(kb)
                    && (end == b.len() || !b[end].is_ascii_alphanumeric())
                {
                    Some((kb.len(), val.as_str()))
                } else {
                    None
                }
            })
        } else {
            None
        };
        match hit {
            Some((klen, val)) => {
                out.push_str(val);
                i += klen; // advance past the matched span — never re-scan output
            }
            None => {
                let ch = text[i..].chars().next().expect("i on a char boundary");
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        let mut p: Vec<(String, String)> =
            v.iter().map(|(k, val)| (k.to_string(), val.to_string())).collect();
        p.sort_by(|a, b| b.0.len().cmp(&a.0.len())); // longest-first
        p
    }

    #[test]
    fn substitutes_whole_words_only() {
        let c = pairs(&[("TOC", "talk")]);
        assert_eq!(apply_lexicon("open TOC now", &c), "open talk now");
        assert_eq!(apply_lexicon("buy STOCK today", &c), "buy STOCK today"); // not inside a word
    }

    #[test]
    fn matches_case_insensitively_value_as_written() {
        let c = pairs(&[("toc", "talk")]);
        assert_eq!(apply_lexicon("TOC Toc toc", &c), "talk talk talk");
    }

    #[test]
    fn longest_key_wins() {
        let c = pairs(&[("talk", "X"), ("talk CLI", "talk")]);
        assert_eq!(apply_lexicon("the talk CLI rocks", &c), "the talk rocks");
    }

    #[test]
    fn single_pass_terminates_on_cyclic_and_value_contains_key() {
        let cyclic = pairs(&[("a", "b"), ("b", "a")]);
        assert_eq!(apply_lexicon("a b", &cyclic), "b a"); // each swapped once, no loop
        let contains = pairs(&[("cloth", "Claude")]);
        assert_eq!(apply_lexicon("cloth", &contains), "Claude"); // "Cl..." not re-scanned
    }

    #[test]
    fn empty_corrections_is_identity() {
        assert_eq!(apply_lexicon("nothing changes", &[]), "nothing changes");
    }
}
