//! Read-only per-process GPU engine counters, benchmark only. No admin/UI needed.
use std::collections::BTreeMap;
use windows_sys::Win32::System::Performance::*;
pub struct GpuCounters {
    query: PDH_HQUERY,
    counter: PDH_HCOUNTER,
}
impl GpuCounters {
    pub fn new() -> Option<Self> {
        unsafe {
            let mut query = std::mem::zeroed();
            if PdhOpenQueryW(std::ptr::null(), 0, &mut query) != 0 {
                return None;
            }
            let mut counter = std::mem::zeroed();
            let path: Vec<u16> = "\\GPU Engine(*)\\Utilization Percentage\0"
                .encode_utf16()
                .collect();
            if PdhAddEnglishCounterW(query, path.as_ptr(), 0, &mut counter) != 0 {
                PdhCloseQuery(query);
                return None;
            }
            PdhCollectQueryData(query);
            Some(Self { query, counter })
        }
    }
    pub fn sample(&mut self, pids: &[u32]) -> Option<BTreeMap<(u32, String), f64>> {
        unsafe {
            if PdhCollectQueryData(self.query) != 0 {
                return None;
            }
            let mut bytes = 0;
            let mut count = 0;
            PdhGetFormattedCounterArrayW(
                self.counter,
                PDH_FMT_DOUBLE,
                &mut bytes,
                &mut count,
                std::ptr::null_mut(),
            );
            if bytes == 0 || bytes > 256 * 1024 {
                return None;
            }
            let mut storage = vec![0usize; (bytes as usize).div_ceil(std::mem::size_of::<usize>())];
            if PdhGetFormattedCounterArrayW(
                self.counter,
                PDH_FMT_DOUBLE,
                &mut bytes,
                &mut count,
                storage.as_mut_ptr().cast(),
            ) != 0
            {
                return None;
            }
            if count as usize
                > storage.len() * std::mem::size_of::<usize>()
                    / std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>()
            {
                return None;
            }
            let items = std::slice::from_raw_parts(
                storage.as_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>(),
                count as usize,
            );
            let mut result = BTreeMap::new();
            for item in items {
                if item.szName.is_null() || item.FmtValue.CStatus > 1 {
                    continue;
                }
                let mut len = 0;
                while len < 1024 && *item.szName.add(len) != 0 {
                    len += 1;
                }
                let name = String::from_utf16_lossy(std::slice::from_raw_parts(item.szName, len));
                let Some(pid) = name
                    .strip_prefix("pid_")
                    .and_then(|s| s.split('_').next())
                    .and_then(|s| s.parse::<u32>().ok())
                else {
                    continue;
                };
                if !pids.contains(&pid) {
                    continue;
                }
                let kind = name
                    .split("_engtype_")
                    .nth(1)
                    .unwrap_or("Other")
                    .to_string();
                let value = item.FmtValue.Anonymous.doubleValue;
                if value.is_finite() && value >= 0. {
                    *result.entry((pid, kind)).or_insert(0.) += value;
                }
            }
            if result.is_empty() {
                None
            } else {
                Some(result)
            }
        }
    }
}
impl Drop for GpuCounters {
    fn drop(&mut self) {
        unsafe {
            PdhCloseQuery(self.query);
        }
    }
}
