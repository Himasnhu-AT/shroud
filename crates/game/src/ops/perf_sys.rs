//! OS probes for the perf monitor: process memory/CPU, per-thread CPU, GPU. All `unsafe` FFI
//! for perf lives here. Called only from the sampler thread (1 Hz), never on the frame.
//!
//! macOS:
//! - memory: `proc_pid_rusage(RUSAGE_INFO_V2)` → resident size and `phys_footprint`
//!   (what Activity Monitor calls "Memory");
//! - CPU: `getrusage(RUSAGE_SELF)`; threads: `task_threads` + `thread_info(THREAD_EXTENDED_INFO)`
//!   (CPU ns + name) and `THREAD_IDENTIFIER_INFO` (stable id);
//! - GPU: IOKit `IOAccelerator` → `PerformanceStatistics` (device-wide utilisation and memory)
//!   and its `AGXDeviceUserClient` children's `AppUsage.accumulatedGPUTime` for *this* pid
//!   (process GPU time, as asitop/macmon read it); Metal `currentAllocatedSize` of the default
//!   device (this process's GPU allocations).
//!
//! Linux: `/proc/self` for RSS and threads, no GPU. Elsewhere: process CPU only (or nothing).

/// CPU time used so far by one thread.
#[derive(Clone, Debug)]
pub struct ThreadCpu {
    pub id: u64,
    pub name: String,
    pub cpu_ns: u64,
}

#[derive(Clone, Debug, Default)]
pub struct GpuSys {
    /// Device-wide utilisation (all processes), %.
    pub device_util: Option<f64>,
    /// GPU time used by this process so far, ns.
    pub process_gpu_ns: Option<u64>,
    /// Device-wide memory in use, bytes.
    pub in_use_system_mem: Option<u64>,
    /// This process's Metal allocations, bytes.
    pub process_alloc: Option<u64>,
}

/// Process CPU time (user + system) in ns.
pub fn process_cpu_ns() -> Option<u64> {
    #[cfg(unix)]
    {
        let mut ru = std::mem::MaybeUninit::<libc::rusage>::zeroed();
        // SAFETY: `ru` is a valid, writable `rusage`; getrusage only writes into it.
        let ok = unsafe { libc::getrusage(libc::RUSAGE_SELF, ru.as_mut_ptr()) } == 0;
        if !ok {
            return None;
        }
        // SAFETY: zero-initialised and filled in by a successful getrusage.
        let ru = unsafe { ru.assume_init() };
        let tv = |t: libc::timeval| t.tv_sec as u64 * 1_000_000_000 + t.tv_usec as u64 * 1000;
        Some(tv(ru.ru_utime) + tv(ru.ru_stime))
    }
    #[cfg(not(unix))]
    None
}

/// Id of the calling thread, comparable with [`ThreadCpu::id`].
pub fn current_thread_id() -> u64 {
    #[cfg(target_os = "macos")]
    {
        let mut id = 0u64;
        // SAFETY: a null pthread_t means "the calling thread"; `id` is a valid out pointer.
        unsafe { libc::pthread_threadid_np(0 as libc::pthread_t, &mut id) };
        id
    }
    #[cfg(target_os = "linux")]
    {
        // SAFETY: gettid has no preconditions.
        unsafe { libc::gettid() as u64 }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    0
}

pub use imp::*;

#[cfg(target_os = "macos")]
mod imp {
    use super::{GpuSys, ThreadCpu};
    use std::ffi::{CStr, c_char, c_void};

    type CFTypeRef = *const c_void;
    type IoObject = u32;

    const KCF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const KCF_NUMBER_SINT64: isize = 4;

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOServiceMatching(name: *const c_char) -> *mut c_void;
        fn IOServiceGetMatchingServices(main_port: u32, matching: *mut c_void, it: *mut IoObject) -> i32;
        fn IOIteratorNext(it: IoObject) -> IoObject;
        fn IOObjectRelease(o: IoObject) -> i32;
        fn IORegistryEntryCreateCFProperty(entry: IoObject, key: CFTypeRef, alloc: CFTypeRef, options: u32) -> CFTypeRef;
        fn IORegistryEntryGetChildIterator(entry: IoObject, plane: *const c_char, it: *mut IoObject) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(alloc: CFTypeRef, s: *const c_char, enc: u32) -> CFTypeRef;
        fn CFStringGetCString(s: CFTypeRef, buf: *mut c_char, size: isize, enc: u32) -> u8;
        fn CFDictionaryGetValue(d: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
        fn CFNumberGetValue(n: CFTypeRef, ty: isize, out: *mut c_void) -> u8;
        fn CFArrayGetCount(a: CFTypeRef) -> isize;
        fn CFArrayGetValueAtIndex(a: CFTypeRef, i: isize) -> CFTypeRef;
        fn CFGetTypeID(cf: CFTypeRef) -> usize;
        fn CFStringGetTypeID() -> usize;
        fn CFNumberGetTypeID() -> usize;
        fn CFArrayGetTypeID() -> usize;
        fn CFDictionaryGetTypeID() -> usize;
        fn CFRelease(cf: CFTypeRef);
    }

    #[link(name = "Metal", kind = "framework")]
    unsafe extern "C" {
        fn MTLCreateSystemDefaultDevice() -> *mut c_void;
    }

    #[link(name = "objc")]
    unsafe extern "C" {
        fn sel_registerName(name: *const c_char) -> *const c_void;
        fn objc_msgSend();
    }

    unsafe extern "C" {
        fn mach_port_deallocate(task: libc::mach_port_t, name: libc::mach_port_t) -> i32;
        static mach_task_self_: libc::mach_port_t;
    }

    fn task_self() -> libc::mach_port_t {
        // SAFETY: `mach_task_self_` is set by libSystem before main and never changes.
        unsafe { mach_task_self_ }
    }

    /// `(resident bytes, phys_footprint bytes)`.
    pub fn process_mem() -> Option<(u64, u64)> {
        let mut info = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
        // SAFETY: `info` is a writable rusage_info_v2, matching the RUSAGE_INFO_V2 flavor.
        let rc = unsafe {
            libc::proc_pid_rusage(
                libc::getpid(),
                libc::RUSAGE_INFO_V2,
                info.as_mut_ptr() as *mut libc::rusage_info_t,
            )
        };
        if rc != 0 {
            return None;
        }
        // SAFETY: zero-initialised and filled in by a successful proc_pid_rusage.
        let info = unsafe { info.assume_init() };
        Some((info.ri_resident_size, info.ri_phys_footprint))
    }

    /// CPU time and name of every thread in this process.
    pub fn threads() -> Vec<ThreadCpu> {
        let task = task_self();
        let mut list: libc::thread_act_array_t = std::ptr::null_mut();
        let mut count: libc::mach_msg_type_number_t = 0;
        // SAFETY: valid out pointers; on success the kernel gives us `count` send rights in a
        // vm-allocated array, which we release below.
        if unsafe { libc::task_threads(task, &mut list, &mut count) } != 0 || list.is_null() {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(count as usize);
        for i in 0..count as usize {
            // SAFETY: `i < count`, the array holds `count` thread ports.
            let th = unsafe { *list.add(i) };
            let mut ext = std::mem::MaybeUninit::<libc::thread_extended_info>::zeroed();
            let mut n = libc::THREAD_EXTENDED_INFO_COUNT;
            // SAFETY: `ext` has room for THREAD_EXTENDED_INFO_COUNT integers, as `n` says.
            let ok = unsafe {
                libc::thread_info(th, libc::THREAD_EXTENDED_INFO as u32, ext.as_mut_ptr() as libc::thread_info_t, &mut n)
            } == 0;
            let mut ident = std::mem::MaybeUninit::<libc::thread_identifier_info>::zeroed();
            let mut n2 = libc::THREAD_IDENTIFIER_INFO_COUNT;
            // SAFETY: as above, for thread_identifier_info.
            let id_ok = unsafe {
                libc::thread_info(th, libc::THREAD_IDENTIFIER_INFO as u32, ident.as_mut_ptr() as libc::thread_info_t, &mut n2)
            } == 0;
            // SAFETY: `th` is a send right task_threads gave us; we drop our reference.
            unsafe { mach_port_deallocate(task, th) };
            if !ok {
                continue;
            }
            // SAFETY: filled in by a successful thread_info (zeroed otherwise).
            let (ext, ident) = unsafe { (ext.assume_init(), ident.assume_init()) };
            // SAFETY: pth_name is a NUL-terminated C string inside a fixed array (zeroed tail).
            let name = unsafe { CStr::from_ptr(ext.pth_name.as_ptr()) }.to_string_lossy().into_owned();
            out.push(ThreadCpu {
                id: if id_ok { ident.thread_id } else { th as u64 },
                name,
                cpu_ns: ext.pth_user_time + ext.pth_system_time,
            });
        }
        // SAFETY: frees the port array task_threads allocated in our address space.
        unsafe {
            libc::vm_deallocate(
                task,
                list as libc::vm_address_t,
                count as usize * std::mem::size_of::<libc::thread_act_t>(),
            )
        };
        out
    }

    /// GPU probe. Holds CF strings and the Metal device; lives on the sampler thread.
    pub struct GpuProbe {
        k_perf: CFTypeRef,
        k_util: CFTypeRef,
        k_inuse: CFTypeRef,
        k_creator: CFTypeRef,
        k_usage: CFTypeRef,
        k_gputime: CFTypeRef,
        mtl: *mut c_void,
        sel_alloc: *const c_void,
        pid_prefix: String,
    }

    // SAFETY: CF strings are immutable and thread-safe; MTLDevice is thread-safe. The probe is
    // only used by one thread at a time anyway.
    unsafe impl Send for GpuProbe {}

    fn cfstr(s: &CStr) -> CFTypeRef {
        // SAFETY: `s` is a valid NUL-terminated string; the result is owned (+1) by the probe.
        unsafe { CFStringCreateWithCString(std::ptr::null(), s.as_ptr(), KCF_STRING_ENCODING_UTF8) }
    }

    /// `CFNumber` → i64, if `v` is one.
    fn num(v: CFTypeRef) -> Option<i64> {
        // SAFETY: `v` is null or a live CF object; we check its type before reading it.
        unsafe {
            if v.is_null() || CFGetTypeID(v) != CFNumberGetTypeID() {
                return None;
            }
            let mut x = 0i64;
            (CFNumberGetValue(v, KCF_NUMBER_SINT64, &mut x as *mut i64 as *mut c_void) != 0).then_some(x)
        }
    }

    /// `CFString` → String, if `v` is one.
    fn string(v: CFTypeRef) -> Option<String> {
        // SAFETY: `v` is null or a live CF object; type-checked; `buf` is writable for its length.
        unsafe {
            if v.is_null() || CFGetTypeID(v) != CFStringGetTypeID() {
                return None;
            }
            let mut buf = [0 as c_char; 256];
            if CFStringGetCString(v, buf.as_mut_ptr(), buf.len() as isize, KCF_STRING_ENCODING_UTF8) == 0 {
                return None;
            }
            Some(CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
        }
    }

    impl GpuProbe {
        pub fn new() -> Self {
            // SAFETY: MTLCreateSystemDefaultDevice returns a retained device (or null); we keep it
            // for the process lifetime. Devices are per-GPU singletons, so this is the device
            // wgpu renders with. sel_registerName takes a valid C string.
            let (mtl, sel_alloc) =
                unsafe { (MTLCreateSystemDefaultDevice(), sel_registerName(c"currentAllocatedSize".as_ptr())) };
            Self {
                k_perf: cfstr(c"PerformanceStatistics"),
                k_util: cfstr(c"Device Utilization %"),
                k_inuse: cfstr(c"In use system memory"),
                k_creator: cfstr(c"IOUserClientCreator"),
                k_usage: cfstr(c"AppUsage"),
                k_gputime: cfstr(c"accumulatedGPUTime"),
                mtl,
                sel_alloc,
                pid_prefix: format!("pid {},", std::process::id()),
            }
        }

        pub fn sample(&mut self) -> GpuSys {
            let mut g = GpuSys::default();
            if !self.mtl.is_null() {
                // SAFETY: `-[MTLDevice currentAllocatedSize]` takes no arguments and returns
                // NSUInteger; objc_msgSend is called through the matching function type.
                g.process_alloc = Some(unsafe {
                    let f: unsafe extern "C" fn(*mut c_void, *const c_void) -> usize =
                        std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
                    f(self.mtl, self.sel_alloc) as u64
                });
            }
            let mut it: IoObject = 0;
            // SAFETY: IOServiceMatching returns a dictionary that IOServiceGetMatchingServices
            // consumes; `it` is a valid out pointer and released below.
            if unsafe { IOServiceGetMatchingServices(0, IOServiceMatching(c"IOAccelerator".as_ptr()), &mut it) } != 0 {
                return g;
            }
            loop {
                // SAFETY: `it` is a live iterator; returned objects are released below.
                let accel = unsafe { IOIteratorNext(it) };
                if accel == 0 {
                    break;
                }
                // SAFETY: valid entry and key; the returned property (+1) is released here.
                unsafe {
                    let stats = IORegistryEntryCreateCFProperty(accel, self.k_perf, std::ptr::null(), 0);
                    if !stats.is_null() {
                        if CFGetTypeID(stats) == CFDictionaryGetTypeID() {
                            g.device_util = num(CFDictionaryGetValue(stats, self.k_util)).map(|v| v as f64).or(g.device_util);
                            g.in_use_system_mem =
                                num(CFDictionaryGetValue(stats, self.k_inuse)).map(|v| v as u64).or(g.in_use_system_mem);
                        }
                        CFRelease(stats);
                    }
                }
                if let Some(ns) = self.process_gpu_time(accel) {
                    *g.process_gpu_ns.get_or_insert(0) += ns;
                }
                // SAFETY: we own this reference from IOIteratorNext.
                unsafe { IOObjectRelease(accel) };
            }
            // SAFETY: we own the iterator.
            unsafe { IOObjectRelease(it) };
            g
        }

        /// Sum of `accumulatedGPUTime` over this process's user clients of `accel`.
        fn process_gpu_time(&self, accel: IoObject) -> Option<u64> {
            let mut it: IoObject = 0;
            // SAFETY: valid entry, plane name and out pointer; iterator released below.
            if unsafe { IORegistryEntryGetChildIterator(accel, c"IOService".as_ptr(), &mut it) } != 0 {
                return None;
            }
            let mut total: Option<u64> = None;
            loop {
                // SAFETY: live iterator.
                let child = unsafe { IOIteratorNext(it) };
                if child == 0 {
                    break;
                }
                // SAFETY: properties are +1 references we release; CF accessors are given
                // type-checked live objects.
                unsafe {
                    let creator = IORegistryEntryCreateCFProperty(child, self.k_creator, std::ptr::null(), 0);
                    let mine = string(creator).is_some_and(|s| s.starts_with(&self.pid_prefix));
                    if !creator.is_null() {
                        CFRelease(creator);
                    }
                    if mine {
                        let usage = IORegistryEntryCreateCFProperty(child, self.k_usage, std::ptr::null(), 0);
                        if !usage.is_null() {
                            if CFGetTypeID(usage) == CFArrayGetTypeID() {
                                for i in 0..CFArrayGetCount(usage) {
                                    let d = CFArrayGetValueAtIndex(usage, i);
                                    if !d.is_null()
                                        && CFGetTypeID(d) == CFDictionaryGetTypeID()
                                        && let Some(ns) = num(CFDictionaryGetValue(d, self.k_gputime))
                                    {
                                        *total.get_or_insert(0) += ns.max(0) as u64;
                                    }
                                }
                            }
                            CFRelease(usage);
                        }
                    }
                    IOObjectRelease(child);
                }
            }
            // SAFETY: we own the iterator.
            unsafe { IOObjectRelease(it) };
            total
        }
    }

    impl Drop for GpuProbe {
        fn drop(&mut self) {
            for k in [self.k_perf, self.k_util, self.k_inuse, self.k_creator, self.k_usage, self.k_gputime] {
                if !k.is_null() {
                    // SAFETY: each key is a +1 CFString created in `new`.
                    unsafe { CFRelease(k) };
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{GpuSys, ThreadCpu};

    /// `(resident bytes, resident bytes)`: Linux has no footprint, RSS stands in.
    pub fn process_mem() -> Option<(u64, u64)> {
        let s = std::fs::read_to_string("/proc/self/statm").ok()?;
        let pages: u64 = s.split_whitespace().nth(1)?.parse().ok()?;
        let rss = pages * 4096;
        Some((rss, rss))
    }

    pub fn threads() -> Vec<ThreadCpu> {
        let Ok(dir) = std::fs::read_dir("/proc/self/task") else { return Vec::new() };
        dir.flatten()
            .filter_map(|e| {
                let id: u64 = e.file_name().to_str()?.parse().ok()?;
                let name = std::fs::read_to_string(e.path().join("comm")).ok()?.trim().to_string();
                let stat = std::fs::read_to_string(e.path().join("stat")).ok()?;
                let rest = &stat[stat.rfind(')')? + 2..];
                let f: Vec<&str> = rest.split_whitespace().collect();
                // utime, stime (fields 14, 15) in clock ticks, assumed 100 Hz.
                let ticks: u64 = f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?;
                Some(ThreadCpu { id, name, cpu_ns: ticks * 10_000_000 })
            })
            .collect()
    }

    pub struct GpuProbe;
    impl GpuProbe {
        pub fn new() -> Self {
            GpuProbe
        }
        pub fn sample(&mut self) -> GpuSys {
            GpuSys::default()
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod imp {
    use super::{GpuSys, ThreadCpu};

    pub fn process_mem() -> Option<(u64, u64)> {
        None
    }
    pub fn threads() -> Vec<ThreadCpu> {
        Vec::new()
    }
    pub struct GpuProbe;
    impl GpuProbe {
        pub fn new() -> Self {
            GpuProbe
        }
        pub fn sample(&mut self) -> GpuSys {
            GpuSys::default()
        }
    }
}
