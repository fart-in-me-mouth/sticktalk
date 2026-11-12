use crate::Result;
use windows::Win32::{
    Foundation::FILETIME,
    System::{ProcessStatus::*, Threading::*},
};

#[derive(Clone, Copy, Debug)]
pub struct ProcessUsage {
    /// Sum of kernel+user CPU time, seconds (not normalized by core count).
    pub cpu_seconds: f64,
    pub rss_bytes: u64,
    pub peak_rss_bytes: u64,
    pub logical_cpus: usize,
}

pub fn process_usage() -> Result<ProcessUsage> {
    unsafe {
        let process = GetCurrentProcess();
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user)?;
        let seconds = |t: FILETIME| {
            ((u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)) as f64 / 10_000_000.0
        };
        let mut memory = PROCESS_MEMORY_COUNTERS {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ..Default::default()
        };
        GetProcessMemoryInfo(
            process,
            &mut memory,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )?;
        Ok(ProcessUsage {
            cpu_seconds: seconds(kernel) + seconds(user),
            rss_bytes: memory.WorkingSetSize as u64,
            peak_rss_bytes: memory.PeakWorkingSetSize as u64,
            logical_cpus: std::thread::available_parallelism().map_or(1, usize::from),
        })
    }
}
