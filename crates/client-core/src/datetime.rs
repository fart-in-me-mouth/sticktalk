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
        format!("{:02}/{:02}/{}", value.month, value.day, value.year)
    };
    format!("{date} at {time}")
}
