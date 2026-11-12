//! Local calendar labels through Win32, without a timezone database or a timer.
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
#[cfg(windows)]
pub fn local(ms: u64) -> Parts {
    use windows_sys::Win32::{
        Foundation::{FILETIME, SYSTEMTIME},
        System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime},
    };
    let ticks = ms.saturating_add(11_644_473_600_000).saturating_mul(10_000);
    let file = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    let mut out = SYSTEMTIME::default();
    // Pointers reference initialized, stack-owned structures; null selects the OS timezone.
    let ok = unsafe {
        FileTimeToSystemTime(&file, &mut utc) != 0
            && SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut out) != 0
    };
    if !ok {
        out = utc;
    }
    Parts {
        year: out.wYear,
        month: out.wMonth,
        day: out.wDay,
        hour: out.wHour,
        minute: out.wMinute,
    }
}
#[cfg(not(windows))]
pub fn local(ms: u64) -> Parts {
    Parts {
        year: 1970,
        month: 1,
        day: (ms / 86_400_000) as u16,
        hour: ((ms / 3_600_000) % 24) as u16,
        minute: ((ms / 60_000) % 60) as u16,
    }
}
pub fn label(ms: u64, short: bool) -> String {
    let value = local(ms);
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
    let now = crate::mock::now();
    let date = if value.date() == local(now).date() {
        "Today".into()
    } else if value.date() == local(now.saturating_sub(crate::theme::DAY_MS)).date() {
        "Yesterday".into()
    } else {
        format!("{:02}/{:02}/{}", value.month, value.day, value.year)
    };
    format!("{date} at {time}")
}
