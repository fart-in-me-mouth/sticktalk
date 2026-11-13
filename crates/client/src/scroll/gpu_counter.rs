//! Optional Windows per-process 3D engine utilization; no polling thread.
#[cfg(windows)]
pub struct GpuCounter {
    query: windows_sys::Win32::System::Performance::PDH_HQUERY,
    counter: windows_sys::Win32::System::Performance::PDH_HCOUNTER,
}
#[cfg(windows)]
impl GpuCounter {
    pub fn new() -> Option<Self> {
        use windows_sys::Win32::System::Performance::*;
        let mut query = std::ptr::null_mut();
        let mut counter = std::ptr::null_mut();
        let path: Vec<u16> = "\\GPU Engine(*)\\Utilization Percentage\0"
            .encode_utf16()
            .collect();
        // SAFETY: null data source selects real-time local counters; output handles are live.
        unsafe {
            if PdhOpenQueryW(std::ptr::null(), 0, &mut query) != 0 {
                return None;
            }
            if PdhAddEnglishCounterW(query, path.as_ptr(), 0, &mut counter) != 0 {
                PdhCloseQuery(query);
                return None;
            }
            let _ = PdhCollectQueryData(query);
        }
        Some(Self { query, counter })
    }
    pub fn sample(&self) -> Option<f64> {
        use windows_sys::Win32::System::Performance::*;
        // SAFETY: handles belong to this query. The size-negotiated u64 buffer has native
        // alignment; PDH returns item_count initialized records and strings within it.
        unsafe {
            if PdhCollectQueryData(self.query) != 0 {
                return None;
            }
            let mut bytes = 0;
            let mut count = 0;
            if PdhGetFormattedCounterArrayW(
                self.counter,
                PDH_FMT_DOUBLE,
                &mut bytes,
                &mut count,
                std::ptr::null_mut(),
            ) != PDH_MORE_DATA
                || bytes > 16 * 1024 * 1024
            {
                return None;
            }
            let mut data = vec![0u64; (bytes as usize).div_ceil(8)];
            if PdhGetFormattedCounterArrayW(
                self.counter,
                PDH_FMT_DOUBLE,
                &mut bytes,
                &mut count,
                data.as_mut_ptr().cast(),
            ) != 0
            {
                return None;
            }
            let items = std::slice::from_raw_parts(
                data.as_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>(),
                count as usize,
            );
            let prefix = format!("pid_{}_", std::process::id());
            let mut found = false;
            let mut sum = 0.;
            for item in items {
                if item.szName.is_null() || item.FmtValue.CStatus > 1 {
                    continue;
                }
                let mut len = 0;
                while len < 1024 && *item.szName.add(len) != 0 {
                    len += 1;
                }
                let name = String::from_utf16_lossy(std::slice::from_raw_parts(item.szName, len))
                    .to_ascii_lowercase();
                if name.starts_with(&prefix) && name.contains("engtype_3d") {
                    found = true;
                    sum += item.FmtValue.Anonymous.doubleValue;
                }
            }
            found.then_some(sum)
        }
    }
}
#[cfg(windows)]
impl Drop for GpuCounter {
    fn drop(&mut self) {
        // SAFETY: this object exclusively owns the open query.
        unsafe {
            windows_sys::Win32::System::Performance::PdhCloseQuery(self.query);
        }
    }
}
#[cfg(not(windows))]
pub struct GpuCounter;
#[cfg(not(windows))]
impl GpuCounter {
    pub fn new() -> Option<Self> {
        None
    }
    pub fn sample(&self) -> Option<f64> {
        None
    }
}
