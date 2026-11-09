use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const HOUR: i64 = 60 * 60;
const DAY: i64 = 24 * HOUR;
const KEPT_HOURS: u32 = 400 * 24;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct Usage {
    hours: BTreeMap<u32, u32>,
}

impl Usage {
    pub(super) fn record(&mut self, now: i64) {
        let hour = hour_of(now);
        *self.hours.entry(hour).or_default() += 1;
        let oldest = hour.saturating_sub(KEPT_HOURS);
        self.hours = self.hours.split_off(&oldest);
    }

    pub(super) fn clear(&mut self) {
        self.hours.clear();
    }

    pub(crate) fn days(&self, now: i64, offset: i64, count: usize) -> Vec<u32> {
        let today = (now + offset).div_euclid(DAY);
        let first = today - count as i64 + 1;
        let mut days = vec![0; count];
        for (&hour, &opens) in &self.hours {
            let day = (i64::from(hour) * HOUR + offset).div_euclid(DAY);
            if let Some(slot) = usize::try_from(day - first).ok().and_then(|slot| days.get_mut(slot)) {
                *slot += opens;
            }
        }
        days
    }
}

fn hour_of(seconds: i64) -> u32 {
    u32::try_from(seconds.div_euclid(HOUR)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_784_116_800;

    #[test]
    fn opens_land_on_local_days() {
        let mut usage = Usage::default();
        usage.record(NOW - DAY);
        usage.record(NOW - 13 * HOUR);
        usage.record(NOW);
        usage.record(NOW - 40 * DAY);
        assert_eq!(usage.days(NOW, 0, 3), [0, 2, 1]);
        assert_eq!(usage.days(NOW, 2 * HOUR, 3), [0, 1, 2]);
    }

    #[test]
    fn old_hours_are_dropped() {
        let mut usage = Usage::default();
        usage.record(NOW - 500 * DAY);
        usage.record(NOW);
        assert_eq!(usage.hours.len(), 1);
    }
}
