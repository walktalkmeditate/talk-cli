use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const STREAK_FILE: &str = ".streak.toml";

/// Local, account-free reflection record. A day is credited when an entry is
/// saved that civil day (ported from meditate-cli; talk credits entries, not
/// session length — writing is the practice).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Streak {
    pub entries: u64,
    pub current_streak: u32,
    pub longest_streak: u32,
    pub last_day: Option<i64>,
}

impl Streak {
    /// Fold one saved entry into the record. `today` is the civil day number
    /// (days since the Unix epoch).
    pub fn record(&mut self, today: i64) {
        self.entries += 1;
        match self.last_day {
            Some(day) if day == today => {}
            Some(day) if today == day + 1 => self.current_streak += 1,
            // Clock moved backward (NTP correction, travel) — leave the streak intact.
            Some(day) if today < day => {}
            _ => self.current_streak = 1,
        }
        self.last_day = Some(today);
        self.longest_streak = self.longest_streak.max(self.current_streak);
    }

    pub fn path_in(dir: &Path) -> PathBuf {
        dir.join(STREAK_FILE)
    }

    /// Missing or corrupt files are no history — a bad file never blocks a launch.
    pub fn load_from(dir: &Path) -> Streak {
        std::fs::read_to_string(Self::path_in(dir))
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }
}

/// Civil day number for an ISO `YYYY-MM-DD` date (Hinnant's days_from_civil).
pub fn civil_day(date: &str) -> Option<i64> {
    let mut parts = date.splitn(3, '-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// Record one saved entry under an exclusive file lock (read-modify-write, not
/// last-writer-wins), creating the file 0600.
pub fn record_entry(dir: &Path, today: i64) -> std::io::Result<Streak> {
    std::fs::create_dir_all(dir)?;
    let mut opts = OpenOptions::new();
    opts.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(Streak::path_in(dir))?;
    file.lock_exclusive()?;

    let result = (|| {
        let mut text = String::new();
        file.read_to_string(&mut text)?;
        let mut streak: Streak = toml::from_str(&text).unwrap_or_default();
        streak.record(today);
        let serialized = toml::to_string_pretty(&streak).expect("streak serializes to TOML");
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(serialized.as_bytes())?;
        Ok(streak)
    })();

    // Fully-qualified trait call: the inherent `File::unlock` is only stable since
    // Rust 1.89, above this crate's 1.82 MSRV — `fs2::FileExt::unlock` is portable.
    let _ = FileExt::unlock(&file);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_days_grow_the_streak() {
        let mut s = Streak::default();
        s.record(100);
        s.record(100); // same day: counted as an entry, not a new streak day
        s.record(101);
        assert_eq!(s.entries, 3);
        assert_eq!(s.current_streak, 2);
        s.record(105); // gap resets
        assert_eq!(s.current_streak, 1);
        assert_eq!(s.longest_streak, 2);
        s.record(103); // clock moved backward: streak intact
        assert_eq!(s.current_streak, 1);
    }

    #[test]
    fn civil_day_matches_known_dates() {
        assert_eq!(civil_day("1970-01-01"), Some(0));
        assert_eq!(civil_day("1970-01-02"), Some(1));
        assert_eq!(civil_day("2026-06-09"), Some(20_613));
        assert_eq!(civil_day("not-a-date"), None);
    }

    #[test]
    fn record_entry_locks_and_persists_0600() {
        let dir = tempfile::tempdir().unwrap();
        record_entry(dir.path(), 50).unwrap();
        let s = Streak::load_from(dir.path());
        assert_eq!(s.current_streak, 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(Streak::path_in(dir.path())).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
