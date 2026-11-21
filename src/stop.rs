use std::sync::atomic::{AtomicBool, Ordering};

static ASKED: AtomicBool = AtomicBool::new(false);

extern "C" fn note(_signal: libc::c_int) {
    ASKED.store(true, Ordering::Relaxed);
}

pub fn watch() {
    let handler = note as extern "C" fn(libc::c_int);
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handler as libc::sighandler_t;
        action.sa_flags = libc::SA_RESETHAND | libc::SA_RESTART;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut());
    }
}

pub fn asked() -> bool {
    ASKED.load(Ordering::Relaxed)
}
