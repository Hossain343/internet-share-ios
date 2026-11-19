//! Wi-Fi share core (iPhone side):
//!  - SOCKS5 proxy on 127.0.0.1:<port>   (data, outbound forced to Wi-Fi en0)
//!  - control channel on 127.0.0.1:<port+1> (handshake/heartbeat with the PC app)
//! The PC reaches both through usbmuxd (USB cable).

mod control;
mod socks;

use std::ffi::{c_char, CStr};
use std::sync::atomic::AtomicUsize;
use std::sync::{Mutex, OnceLock};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

static RT: OnceLock<Runtime> = OnceLock::new();
static TASK: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);
pub(crate) static ACTIVE: AtomicUsize = AtomicUsize::new(0);

fn rt() -> &'static Runtime {
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("wifishare")
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

fn lock_task() -> std::sync::MutexGuard<'static, Option<JoinHandle<()>>> {
    match TASK.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

/// Start services. Returns 0 on success, -1 on bind failure.
#[no_mangle]
pub extern "C" fn wifi_share_start(port: u16) -> i32 {
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    let mut guard = lock_task();
    if let Some(h) = guard.as_ref() {
        if !h.is_finished() {
            return 0;
        }
    }
    let data = match rt().block_on(socks::bind(port)) {
        Ok(l) => l,
        Err(_) => return -1,
    };
    let ctrl = match rt().block_on(socks::bind(port.wrapping_add(1))) {
        Ok(l) => l,
        Err(_) => return -1,
    };
    *guard = Some(rt().spawn(async move {
        tokio::join!(socks::serve(data), control::serve(ctrl));
    }));
    0
}

#[no_mangle]
pub extern "C" fn wifi_share_stop() {
    if let Some(h) = lock_task().take() {
        h.abort();
    }
    control::reset();
}

#[no_mangle]
pub extern "C" fn wifi_share_running() -> i32 {
    match lock_task().as_ref() {
        Some(h) if !h.is_finished() => 1,
        _ => 0,
    }
}

#[no_mangle]
pub extern "C" fn wifi_share_active() -> u32 {
    ACTIVE.load(std::sync::atomic::Ordering::Relaxed) as u32
}

/// 1 when the Windows receiver app is linked (handshake done, heartbeat alive).
#[no_mangle]
pub extern "C" fn wifi_share_linked() -> i32 {
    control::linked() as i32
}

/// Copies the PC name (NUL-terminated) into buf. Returns the byte length.
#[no_mangle]
pub unsafe extern "C" fn wifi_share_pc_name(buf: *mut c_char, cap: usize) -> i32 {
    if buf.is_null() || cap == 0 {
        return 0;
    }
    let name = control::pc_name();
    let b = name.as_bytes();
    let k = b.len().min(cap - 1);
    std::ptr::copy_nonoverlapping(b.as_ptr(), buf as *mut u8, k);
    *buf.add(k) = 0;
    k as i32
}

#[no_mangle]
pub unsafe extern "C" fn wifi_share_set_device_name(name: *const c_char) {
    if name.is_null() {
        return;
    }
    control::set_device_name(CStr::from_ptr(name).to_string_lossy().into_owned());
}
