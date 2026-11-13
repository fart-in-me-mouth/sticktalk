//! Calendar-label formatting; hosts supply their timezone conversion and clock.
#[derive(Clone, Copy)]
pub struct Parts {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
}
impl Parts {
    pub fn date(self) -> (u16, u16, u16) {
        (self.year, self.month, self.day)
    }
}
pub fn calendar_date(value: Parts) -> String {
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][(value.month as usize).saturating_sub(1).min(11)];
    format!("{month} {}, {}", value.day, value.year)
}
pub fn label(value: Parts, today: Parts, yesterday: Parts, short: bool) -> String {
    let time = format!(
        "{}:{:02} {}",
        if value.hour.is_multiple_of(12) {
            12
        } else {
            value.hour % 12
        },
        value.minute,
        if value.hour < 12 { "AM" } else { "PM" }
    );
    if short {
        return time;
    }
    let date = if value.date() == today.date() {
        "Today".into()
    } else if value.date() == yesterday.date() {
        "Yesterday".into()
    } else {
        calendar_date(value)
    };
    format!("{date} at {time}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calendar_dates_use_month_names_without_zero_padding() {
        let day = Parts {
            year: 2025,
            month: 4,
            day: 18,
            hour: 12,
            minute: 0,
        };
        assert_eq!(calendar_date(day), "Apr 18, 2025");
        assert_eq!(calendar_date(Parts { day: 3, ..day }), "Apr 3, 2025");
        assert_eq!(label(day, day, day, false), "Today at 12:00 PM");
        assert_eq!(
            label(day, Parts { year: 2026, ..day }, day, false),
            "Yesterday at 12:00 PM"
        );
        assert_eq!(
            label(
                day,
                Parts { year: 2026, ..day },
                Parts { day: 17, ..day },
                false
            ),
            "Apr 18, 2025 at 12:00 PM"
        );
    }
}
