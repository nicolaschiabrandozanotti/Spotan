//! Keep transient network/decoder allocations from becoming permanent idle RAM.
//!
//! This only tunes glibc's allocator; live audio and library data are untouched.
pub fn configure() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        libc::mallopt(libc::M_ARENA_MAX, 2);
        libc::mallopt(libc::M_TRIM_THRESHOLD, 128 * 1024);
        libc::mallopt(libc::M_TOP_PAD, 32 * 1024);
    }
}

pub fn reclaim_unused() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        // glibc synchronizes its arenas and only returns completely free pages.
        libc::malloc_trim(0);
    }
}
