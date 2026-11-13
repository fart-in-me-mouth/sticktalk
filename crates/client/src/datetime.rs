pub use sticktalk_client_core::datetime::Parts;
pub fn date(ms: u64) -> String {
    sticktalk_client_core::datetime::calendar_date(local(ms))
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
    let now = crate::mock::now();
    sticktalk_client_core::datetime::label(
        local(ms),
        local(now),
        local(now.saturating_sub(crate::theme::DAY_MS)),
        short,
    )
}

pub fn member_since(d: Parts) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month = MONTHS
        .get(d.month.saturating_sub(1) as usize)
        .unwrap_or(&"Jan");
    format!("{month} {}, {}", d.day, d.year)
}
#[cfg(test)]
mod tests {
    #[test]
    fn membership_date_is_readable() {
        assert_eq!(
            super::member_since(super::Parts {
                year: 2026,
                month: 7,
                day: 15,
                hour: 0,
                minute: 0
            }),
            "Jul 15, 2026"
        );
    }
}
