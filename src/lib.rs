//! Wi-Fi share core (iPhone side):
//!  - SOCKS5 proxy on loopback:<port>       (data, outbound forced to Wi-Fi en0)
//!  - control channel on loopback:<port+1>  (handshake/heartbeat with the PC app)
//! The PC reaches both through usbmuxd (USB cable). Listeners are loopback-only
//! (IPv4 and, when available, IPv6) so nothing is exposed to the Wi-Fi network.

mod control;
mod socks;

use std::ffi::{c_char, CStr};
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener as StdListener};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::{Mutex, MutexGuard, OnceLock};

use socket2::{Domain, Socket, Type};
use tokio::net::TcpListener;
use tokio::runtime::Runtime;
use tokio::task::{JoinHandle, JoinSet};

static RT: OnceLock<Runtime> = OnceLock::new();
static TASK: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(false);
static LAST_ERR: Mutex<String> = Mutex::new(String::new());
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

fn lock_task() -> MutexGuard<'static, Option<JoinHandle<()>>> {
    match TASK.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn set_err(s: String) {
    if let Ok(mut g) = LAST_ERR.lock() {
        *g = s;
    }
}

fn bind_loopback(port: u16, v6: bool) -> io::Result<StdListener> {
    let (domain, ip): (Domain, IpAddr) = if v6 {
        (Domain::IPV6, Ipv6Addr::LOCALHOST.into())
    } else {
        (Domain::IPV4, Ipv4Addr::LOCALHOST.into())
    };
    let s = Socket::new(domain, Type::STREAM, None)?;
    s.set_reuse_address(true)?;
    if v6 {
        s.set_only_v6(true)?;
    }
    s.bind(&SocketAddr::new(ip, port).into())?;
    s.listen(1024)?;
    s.set_nonblocking(true)?;
    Ok(s.into())
}

fn bind_pair(port: u16, what: &str) -> Result<Vec<StdListener>, String> {
    let mut v = Vec::new();
    match bind_loopback(port, false) {
        Ok(l) => v.push(l),
        Err(e) => return Err(format!("{what}: cannot listen on 127.0.0.1:{port} ({e})")),
    }
    if let Ok(l) = bind_loopback(port, true) {
        v.push(l);
    }
    Ok(v)
}

/// Start services. Returns 0 on success, -1 on failure (see wifi_share_last_error).
#[no_mangle]
pub extern "C" fn wifi_share_start(port: u16) -> i32 {
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    let mut guard = lock_task();
    if RUNNING.load(Relaxed) {
        if let Some(h) = guard.as_ref() {
            if !h.is_finished() {
                return 0;
            }
        }
    }
    let data = match bind_pair(port, "proxy") {
        Ok(v) => v,
        Err(e) => {
            set_err(e);
            return -1;
        }
    };
    let ctrl = match bind_pair(port.wrapping_add(1), "control") {
        Ok(v) => v,
        Err(e) => {
            set_err(e);
            return -1;
        }
    };
    let h = rt().spawn(async move {
        let mut set: JoinSet<()> = JoinSet::new();
        for l in data {
            if let Ok(l) = TcpListener::from_std(l) {
                set.spawn(socks::serve(l));
            }
        }
        for l in ctrl {
            if let Ok(l) = TcpListener::from_std(l) {
                set.spawn(control::serve(l));
            }
        }
        while set.join_next().await.is_some() {}
    });
    *guard = Some(h);
    RUNNING.store(true, Relaxed);
    set_err(String::new());
    0
}

#[no_mangle]
pub extern "C" fn wifi_share_stop() {
    RUNNING.store(false, Relaxed);
    if let Some(h) = lock_task().take() {
        h.abort();
    }
    control::reset();
}

#[no_mangle]
pub extern "C" fn wifi_share_running() -> i32 {
    if !RUNNING.load(Relaxed) {
        return 0;
    }
    match lock_task().as_ref() {
        Some(h) if !h.is_finished() => 1,
        _ => {
            RUNNING.store(false, Relaxed);
            0
        }
    }
}

#[no_mangle]
pub extern "C" fn wifi_share_active() -> u32 {
    ACTIVE.load(Relaxed) as u32
}

/// 1 when the Windows receiver app is linked (handshake done, heartbeat alive).
#[no_mangle]
pub extern "C" fn wifi_share_linked() -> i32 {
    control::linked() as i32
}

unsafe fn copy_out(s: &str, buf: *mut c_char, cap: usize) -> i32 {
    if buf.is_null() || cap == 0 {
        return 0;
    }
    let b = s.as_bytes();
    let k = b.len().min(cap - 1);
    std::ptr::copy_nonoverlapping(b.as_ptr(), buf as *mut u8, k);
    *buf.add(k) = 0;
    k as i32
}

/// Copies the PC name (NUL-terminated) into buf. Returns the byte length.
#[no_mangle]
pub unsafe extern "C" fn wifi_share_pc_name(buf: *mut c_char, cap: usize) -> i32 {
    copy_out(&control::pc_name(), buf, cap)
}

/// Copies the last start error (empty if none) into buf.
#[no_mangle]
pub unsafe extern "C" fn wifi_share_last_error(buf: *mut c_char, cap: usize) -> i32 {
    let e = LAST_ERR.lock().map(|g| g.clone()).unwrap_or_default();
    copy_out(&e, buf, cap)
}

#[no_mangle]
pub unsafe extern "C" fn wifi_share_set_device_name(name: *const c_char) {
    if name.is_null() {
        return;
    }
    control::set_device_name(CStr::from_ptr(name).to_string_lossy().into_owned());
}
