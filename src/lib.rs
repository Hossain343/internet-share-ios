//! Wi-Fi share core: a very fast SOCKS5 (CONNECT) proxy bound to 127.0.0.1.
//! The PC reaches it over USB via usbmuxd port forwarding (iproxy).
//! Outbound traffic is forced onto the Wi-Fi interface (en0).

mod socks;

use std::sync::atomic::{AtomicUsize, Ordering};
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

/// Start the proxy on 127.0.0.1:port. Returns 0 on success, -1 on bind failure.
#[no_mangle]
pub extern "C" fn wifi_share_start(port: u16) -> i32 {
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    let mut guard = match TASK.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if let Some(h) = guard.as_ref() {
        if !h.is_finished() {
            return 0;
        }
    }
    let listener = match rt().block_on(socks::bind(port)) {
        Ok(l) => l,
        Err(_) => return -1,
    };
    *guard = Some(rt().spawn(socks::serve(listener)));
    0
}

/// Stop the proxy and drop every open connection.
#[no_mangle]
pub extern "C" fn wifi_share_stop() {
    let mut guard = match TASK.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if let Some(h) = guard.take() {
        h.abort();
    }
}

/// 1 if running, else 0.
#[no_mangle]
pub extern "C" fn wifi_share_running() -> i32 {
    let guard = match TASK.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    match guard.as_ref() {
        Some(h) if !h.is_finished() => 1,
        _ => 0,
    }
}

/// Number of currently open proxied connections.
#[no_mangle]
pub extern "C" fn wifi_share_active() -> u32 {
    ACTIVE.load(Ordering::Relaxed) as u32
}
