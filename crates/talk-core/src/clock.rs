/// Injectable time source so selection is deterministic in tests.
pub trait Clock {
    fn hour(&self) -> u32; // 0..=23
}

pub struct FixedClock(pub u32);
impl Clock for FixedClock {
    fn hour(&self) -> u32 { self.0 }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeOfDay { Morning, Day, Evening, Night }

pub fn time_of_day(hour: u32) -> TimeOfDay {
    match hour {
        5..=8 => TimeOfDay::Morning,
        9..=16 => TimeOfDay::Day,
        17..=20 => TimeOfDay::Evening,
        _ => TimeOfDay::Night,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_the_day() {
        assert_eq!(time_of_day(7), TimeOfDay::Morning);
        assert_eq!(time_of_day(23), TimeOfDay::Night);
    }
}
