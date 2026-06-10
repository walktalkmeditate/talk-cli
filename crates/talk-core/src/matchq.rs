//! Near-match detection for bring-your-own questions, so a rephrasing offers to
//! continue the existing thread instead of silently forking it (spec §8).

use std::collections::HashSet;

fn content_set(q: &str) -> HashSet<String> {
    q.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2) // drop a/i/am/is-grade tokens
        .map(str::to_string)
        .collect()
}

/// Jaccard similarity over content tokens. 1.0 = same words, 0.0 = disjoint.
pub fn similarity(a: &str, b: &str) -> f32 {
    let (sa, sb) = (content_set(a), content_set(b));
    if sa.is_empty() || sb.is_empty() {
        return 0.0;
    }
    let inter = sa.intersection(&sb).count() as f32;
    let union = sa.union(&sb).count() as f32;
    inter / union
}

/// The existing question most similar to `q`, if it clears the near-match bar
/// and isn't an exact match (exact reuses the thread already, by slug).
pub fn near_match<'a>(q: &str, existing: &'a [String]) -> Option<&'a String> {
    existing
        .iter()
        .filter(|e| e.as_str() != q)
        .map(|e| (similarity(q, e), e))
        .filter(|(s, _)| *s >= 0.5) // 0.5: "what am i avoiding right now" vs "What am I avoiding?" = 4∩2/4∪2 = 0.5 — the canonical rephrase must clear the bar (verified by execution in review).
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, e)| e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rephrasing_clears_the_bar_unrelated_does_not() {
        let existing = vec![
            "What am I avoiding?".to_string(),
            "Where does my anger live?".to_string(),
        ];
        assert_eq!(
            near_match("what am i avoiding right now", &existing),
            Some(&existing[0])
        );
        assert_eq!(near_match("how do I rest more deeply", &existing), None);
    }

    #[test]
    fn exact_match_is_not_offered() {
        let existing = vec!["What am I avoiding?".to_string()];
        assert_eq!(near_match("What am I avoiding?", &existing), None);
    }
}
