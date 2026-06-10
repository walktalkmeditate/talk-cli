use talk_core::questions::Pack;

const SPINE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/questions/spine.toml"));
const FUTURE_SELF: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/questions/future-self.toml"));
const PARTS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/questions/parts.toml"));
const EXAMEN: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/questions/examen.toml"));
const HELD: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/questions/held.toml"));

/// Every pack compiled into the binary, in display order.
pub fn vendored() -> Vec<Pack> {
    [SPINE, FUTURE_SELF, PARTS, EXAMEN, HELD]
        .iter()
        .map(|s| Pack::from_toml(s).expect("vendored pack TOML is valid"))
        .collect()
}

/// The pack to serve from. Unknown names fall back to the spine (never an error —
/// a stale config must not block a reflection).
pub fn by_name(name: &str) -> Pack {
    vendored()
        .into_iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| Pack::from_toml(SPINE).expect("spine TOML is valid"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_vendored_packs_load_with_unique_ids() {
        let packs = vendored();
        assert_eq!(packs.len(), 5);
        let mut ids: Vec<String> = packs.iter().flat_map(|p| p.questions.iter().map(|q| q.id.clone())).collect();
        let total = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), total, "question ids must be globally unique: two packs sharing an id would collide in one thread-file namespace");
    }

    #[test]
    fn by_name_finds_flagships_and_falls_back_to_spine() {
        assert_eq!(by_name("parts").name, "parts");
        assert_eq!(by_name("does-not-exist").name, "spine");
    }

    #[test]
    fn held_pack_is_all_held_cadence() {
        assert!(by_name("held").questions.iter().all(|q| q.cadence == "held:7"));
    }
}
