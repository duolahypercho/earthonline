//! Process CPU time and memory (Windows; zeros elsewhere), for the bench.

#[derive(Clone, Copy, Debug, Default)]
pub struct Memory {
    pub working_set: u64,
    pub peak_working_set: u64,
    pub private: u64,
}

#[cfg(windows)]
mod win {
    #[repr(C)]
    #[derive(Default)]
    pub struct Counters {
        pub cb: u32,
        pub page_fault_count: u32,
        pub peak_working_set: usize,
        pub working_set: usize,
        pub quota: [usize; 4],
        pub pagefile_usage: usize,
        pub peak_pagefile_usage: usize,
    }
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    pub struct FileTime {
        pub lo: u32,
        pub hi: u32,
    }
    #[link(name = "psapi")]
    unsafe extern "system" {
        pub fn GetProcessMemoryInfo(process: isize, counters: *mut Counters, cb: u32) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn GetCurrentProcess() -> isize;
        pub fn GetProcessTimes(p: isize, c: *mut FileTime, e: *mut FileTime, k: *mut FileTime, u: *mut FileTime) -> i32;
    }
}

/// User + kernel CPU seconds of this process so far.
pub fn process_cpu_s() -> f64 {
    #[cfg(windows)]
    {
        let (mut c, mut e, mut k, mut u) = Default::default();
        // SAFETY: plain Win32 call with writable out-parameters.
        if unsafe { win::GetProcessTimes(win::GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) } != 0 {
            let t = |f: win::FileTime| ((f.hi as u64) << 32 | f.lo as u64) as f64 * 1e-7;
            return t(k) + t(u);
        }
    }
    0.0
}

pub fn memory() -> Memory {
    #[cfg(windows)]
    {
        let mut c = win::Counters { cb: std::mem::size_of::<win::Counters>() as u32, ..Default::default() };
        // SAFETY: plain Win32 call with a correctly sized, writable struct.
        if unsafe { win::GetProcessMemoryInfo(win::GetCurrentProcess(), &mut c, c.cb) } != 0 {
            return Memory { working_set: c.working_set as u64, peak_working_set: c.peak_working_set as u64, private: c.pagefile_usage as u64 };
        }
    }
    Memory::default()
}
