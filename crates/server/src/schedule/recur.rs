//! Occurrences of a recurring event, computed in the creator's time zone so
//! that "every Monday at 19:00 in Berlin" stays at 19:00 across daylight
//! saving changes. Each occurrence is stepped from the first one (never from
//! the previous), so monthly events on the 31st do not drift to the 28th.
use super::*;
use jiff::{civil::DateTime, tz::TimeZone, Timestamp, ToSpan};

const DAY_MS: u64 = 86_400_000;

#[derive(Clone)]
pub(crate) struct Rule {
    start: DateTime,
    tz: TimeZone,
    recurrence: EventRecurrence,
    /// UTC milliseconds of occurrence 0.
    pub first_at: u64,
    /// Occurrences at or after this belong to a later series.
    pub until: Option<u64>,
}

/// An IANA zone the server knows (from the bundled time zone database).
pub(crate) fn zone(name: &str) -> Result<TimeZone> {
    require(
        !name.is_empty() && name.len() <= 64 && name.is_ascii(),
        ErrorCode::InvalidRequest,
    )?;
    let tz = TimeZone::get(name).map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    require(tz.iana_name().is_some(), ErrorCode::InvalidRequest)?;
    Ok(tz)
}

/// The civil date and time `at` (UTC milliseconds) has in `tz`.
pub(crate) fn local(at: u64, tz: &TimeZone) -> Result<DateTime> {
    let ts = Timestamp::from_millisecond(at.min(i64::MAX as u64) as i64)
        .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    Ok(ts.to_zoned_datetime(tz.clone()).datetime())
}

fn utc(dt: DateTime, tz: &TimeZone) -> Option<u64> {
    let z = dt.to_zoned_datetime(tz.clone()).ok()?;
    u64::try_from(z.timestamp().as_millisecond()).ok()
}

impl Rule {
    pub(crate) fn new(
        start_local: &str,
        time_zone: &str,
        recurrence: EventRecurrence,
        until: Option<u64>,
    ) -> Result<Self> {
        let tz = zone(time_zone)?;
        let start: DateTime = start_local
            .parse()
            .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
        let first_at = utc(start, &tz).ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
        Ok(Self {
            start,
            tz,
            recurrence,
            first_at,
            until,
        })
    }
    /// A rule whose first occurrence is at `at` (UTC) as seen in `time_zone`.
    pub(crate) fn starting(
        at: u64,
        time_zone: &str,
        recurrence: EventRecurrence,
    ) -> Result<(Self, String)> {
        let tz = zone(time_zone)?;
        let start = local(at, &tz)?;
        let text = start.to_string();
        Ok((Self::new(&text, time_zone, recurrence, None)?, text))
    }
    pub(crate) fn recurrence(&self) -> EventRecurrence {
        self.recurrence
    }
    /// Occurrence `n` (0 is the first), in UTC milliseconds.
    pub(crate) fn nth(&self, n: u64) -> Option<u64> {
        let n = i64::try_from(n).ok()?;
        let dt = match self.recurrence {
            EventRecurrence::None if n == 0 => self.start,
            EventRecurrence::None => return None,
            EventRecurrence::Daily => self.start.checked_add(n.days()).ok()?,
            EventRecurrence::Weekly => self.start.checked_add((n.checked_mul(7)?).days()).ok()?,
            EventRecurrence::EveryTwoWeeks => {
                self.start.checked_add((n.checked_mul(14)?).days()).ok()?
            }
            EventRecurrence::Monthly => self.start.checked_add(n.months()).ok()?,
            EventRecurrence::Yearly => self.start.checked_add(n.years()).ok()?,
            EventRecurrence::Weekdays => {
                // Monday to Friday, starting with the first one on or after the start.
                let w = i64::from(self.start.weekday().to_monday_zero_offset());
                let base = if w >= 5 { 7 - w } else { 0 };
                let w = if w >= 5 { 0 } else { w };
                let (weeks, rem) = (n / 5, n % 5);
                let extra = if w + rem >= 5 { 2 } else { 0 };
                let days = base + weeks.checked_mul(7)? + rem + extra;
                self.start.checked_add(days.days()).ok()?
            }
        };
        let at = utc(dt, &self.tz)?;
        if self.until.is_some_and(|u| at >= u) {
            return None;
        }
        Some(at)
    }
    /// An index at or below the first occurrence at or after `t`.
    fn estimate(&self, t: u64) -> u64 {
        let gap = t.saturating_sub(self.first_at);
        let n = match self.recurrence {
            EventRecurrence::None => 0,
            EventRecurrence::Daily => gap / DAY_MS,
            EventRecurrence::Weekly => gap / (7 * DAY_MS),
            EventRecurrence::EveryTwoWeeks => gap / (14 * DAY_MS),
            EventRecurrence::Monthly => gap / (31 * DAY_MS),
            EventRecurrence::Yearly => gap / (366 * DAY_MS),
            EventRecurrence::Weekdays => gap / DAY_MS * 5 / 7,
        };
        n.saturating_sub(2)
    }
    /// The first occurrence at or after `t`, and its index.
    pub(crate) fn next_from(&self, t: u64) -> Option<(u64, u64)> {
        for n in (self.estimate(t)..).take(64) {
            let at = self.nth(n)?;
            if at >= t {
                return Some((n, at));
            }
        }
        None
    }
    /// Whether `at` is one of this rule's occurrences.
    pub(crate) fn contains(&self, at: u64) -> bool {
        self.next_from(at).is_some_and(|(_, o)| o == at)
    }
    /// Up to `count` occurrences after `at`.
    pub(crate) fn after(&self, at: u64, count: usize) -> Vec<u64> {
        let mut out = vec![];
        let Some((mut n, first)) = self.next_from(at.saturating_add(1)) else {
            return out;
        };
        out.push(first);
        while out.len() < count {
            n += 1;
            match self.nth(n) {
                Some(o) => out.push(o),
                None => break,
            }
        }
        out
    }
    /// The civil start, for the calendar file.
    pub(crate) fn start_local(&self) -> DateTime {
        self.start
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ms(s: &str, tz: &str) -> u64 {
        utc(s.parse().unwrap(), &zone(tz).unwrap()).unwrap()
    }
    #[test]
    fn weekly_keeps_local_time_across_daylight_saving() {
        // 19:00 in Berlin each Monday from October into November 2026 (DST
        // ends on October 25): the UTC time moves by an hour, the local not.
        let rule = Rule::new(
            "2026-10-19T19:00:00",
            "Europe/Berlin",
            EventRecurrence::Weekly,
            None,
        )
        .unwrap();
        assert_eq!(
            rule.nth(0),
            Some(ms("2026-10-19T19:00:00", "Europe/Berlin"))
        );
        assert_eq!(rule.nth(0), Some(ms("2026-10-19T17:00:00", "UTC")));
        assert_eq!(rule.nth(1), Some(ms("2026-10-26T18:00:00", "UTC")));
        for n in 0..10 {
            let at = rule.nth(n).unwrap();
            assert_eq!(
                local(at, &zone("Europe/Berlin").unwrap()).unwrap().hour(),
                19
            );
        }
    }
    #[test]
    fn monthly_clamps_without_drifting_and_weekdays_skip_weekends() {
        let rule = Rule::new("2026-01-31T09:00:00", "UTC", EventRecurrence::Monthly, None).unwrap();
        let tz = zone("UTC").unwrap();
        let days: Vec<i8> = (0..4)
            .map(|n| local(rule.nth(n).unwrap(), &tz).unwrap().day())
            .collect();
        assert_eq!(days, vec![31, 28, 31, 30]);
        // Friday 2026-11-27: then Monday, Tuesday...
        let rule = Rule::new(
            "2026-11-27T08:00:00",
            "UTC",
            EventRecurrence::Weekdays,
            None,
        )
        .unwrap();
        let days: Vec<i8> = (0..7)
            .map(|n| local(rule.nth(n).unwrap(), &tz).unwrap().day())
            .collect();
        assert_eq!(days, vec![27, 30, 1, 2, 3, 4, 7]);
        // Starting on a Saturday: the first occurrence is the Monday after.
        let rule = Rule::new(
            "2026-11-28T08:00:00",
            "UTC",
            EventRecurrence::Weekdays,
            None,
        )
        .unwrap();
        assert_eq!(local(rule.nth(0).unwrap(), &tz).unwrap().day(), 30);
    }
    #[test]
    fn next_from_until_and_unknown_zones() {
        let rule = Rule::new(
            "2026-01-01T12:00:00",
            "America/New_York",
            EventRecurrence::Daily,
            Some(ms("2026-03-01T00:00:00", "UTC")),
        )
        .unwrap();
        let t = ms("2026-02-10T18:00:00", "UTC");
        let (_, next) = rule.next_from(t).unwrap();
        assert!(next >= t && next - t < DAY_MS);
        assert!(rule.contains(next));
        assert!(!rule.contains(next + 1));
        assert!(rule.next_from(ms("2026-03-02T00:00:00", "UTC")).is_none());
        assert_eq!(rule.after(next, 3).len(), 3);
        assert!(zone("Mars/Olympus_Mons").is_err());
        assert!(zone("").is_err());
        let once = Rule::new(
            "2026-05-05T05:05:00",
            "Asia/Tokyo",
            EventRecurrence::None,
            None,
        )
        .unwrap();
        assert_eq!(once.nth(1), None);
        assert_eq!(once.after(once.first_at, 5), Vec::<u64>::new());
    }
}
