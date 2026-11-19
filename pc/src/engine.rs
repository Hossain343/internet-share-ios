use std::io;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{copy_bidirectional_with_sizes, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpSocket, TcpStream};
use tokio::process::Command;
use tokio::runtime::Runtime;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{sleep, timeout};

use crate::{dns, muxd};

pub const PROXY_PORT: u16 = 10808; // local SOCKS5 address handed to tun2socks
pub const PHONE_SOCKS_PORT: u16 = 1080;
pub const PHONE_CTRL_PORT: u16 = 1081;
pub const ADAPTER: &str = "WiFiShare";
pub const ADAPTER_IP: &str = "10.77.0.2";
const GATEWAY_IP: &str = "10.77.0.1";
const BUF: usize = 512 * 1024;
const SOCK_BUF: usize = 2 * 1024 * 1024;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

static TUN2SOCKS: &[u8] = include_bytes!("../assets/tun2socks.exe");
static WINTUN: &[u8] = include_bytes!("../assets/wintun.dll");

#[derive(Default)]
pub struct Shared {
    pub device_id: AtomicU32,
    pub linked: AtomicBool,
    pub tun_up: AtomicBool,
    pub muxd_ok: AtomicBool,
    /// 0 unknown, 1 service running, 2 stopped/starting, 3 not installed
    pub apple: AtomicU8,
    pub muxd_err: Mutex<String>,
    pub active: AtomicUsize,
    pub phone: Mutex<String>,
    pub note: Mutex<String>,
}

pub enum Level {
    Off,
    Wait,
    Bad,
    Ok,
}

pub struct Snapshot {
    pub on: bool,
    pub level: Level,
    pub status: String,
    pub phone: String,
    pub proxy: String,
    pub adapter: String,
    pub active: usize,
    pub note: String,
    pub detail: String,
    pub need_install: bool,
}

pub struct Engine {
    rt: Runtime,
    sh: Arc<Shared>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl Engine {
    pub fn new() -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("tokio runtime");
        Engine { rt, sh: Arc::new(Shared::default()), task: Mutex::new(None) }
    }

    pub fn is_on(&self) -> bool {
        self.task.lock().map(|g| g.as_ref().map_or(false, |h| !h.is_finished())).unwrap_or(false)
    }

    pub fn start(&self) {
        let mut g = match self.task.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if g.as_ref().map_or(false, |h| !h.is_finished()) {
            return;
        }
        if let Ok(mut n) = self.sh.note.lock() {
            n.clear();
        }
        *g = Some(self.rt.spawn(run(self.sh.clone())));
    }

    pub fn stop(&self) {
        let h = match self.task.lock() {
            Ok(mut g) => g.take(),
            Err(p) => p.into_inner().take(),
        };
        if let Some(h) = h {
            h.abort();
        }
        self.sh.device_id.store(0, Relaxed);
        self.sh.linked.store(false, Relaxed);
        self.sh.tun_up.store(false, Relaxed);
        if let Ok(mut p) = self.sh.phone.lock() {
            p.clear();
        }
    }

    pub fn toggle(&self) {
        if self.is_on() {
            self.stop()
        } else {
            self.start()
        }
    }

    /// Opens a visible console that installs iTunes (includes Apple USB service + driver) via winget.
    pub fn install_apple(&self) {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("cmd")
            .args([
                "/c", "start", "Apple components", "cmd", "/k", "winget", "install", "-e", "--id", "Apple.iTunes",
                "--accept-package-agreements", "--accept-source-agreements",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }

    /// Opens the Apple Devices page of the Microsoft Store.
    pub fn open_store(&self) {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", "", "ms-windows-store://pdp/?productid=9NP83LWLPZ9K"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }

    pub fn snapshot(&self) -> Snapshot {
        let sh = &self.sh;
        let on = self.is_on();
        let dev = sh.device_id.load(Relaxed);
        let linked = sh.linked.load(Relaxed);
        let up = sh.tun_up.load(Relaxed);
        let apple = sh.apple.load(Relaxed);
        let muxd_ok = sh.muxd_ok.load(Relaxed);
        let (level, status) = if !on {
            (Level::Off, "Stopped")
        } else if !muxd_ok {
            match apple {
                3 => (Level::Bad, "Apple USB support is NOT installed - press Install below"),
                2 => (Level::Wait, "Apple USB service is stopped - starting it..."),
                1 => (Level::Bad, "Apple service is running but not answering - see details"),
                _ => (Level::Wait, "Checking Apple USB service..."),
            }
        } else if dev == 0 {
            (Level::Wait, "Waiting for iPhone - plug in the cable, unlock it, tap Trust")
        } else if !linked {
            (Level::Wait, "iPhone detected - open the WiFi Share app on the phone")
        } else if !up {
            (Level::Wait, "Linked to phone - creating network adapter...")
        } else {
            (Level::Ok, "Connected - internet is coming from the iPhone")
        };
        let detail = if on && !muxd_ok { sh.muxd_err.lock().map(|e| e.clone()).unwrap_or_default() } else { String::new() };
        let phone = sh.phone.lock().map(|p| p.clone()).unwrap_or_default();
        Snapshot {
            on,
            level,
            status: status.to_string(),
            phone: if phone.is_empty() { "-".into() } else { phone },
            proxy: format!("socks5://127.0.0.1:{PROXY_PORT}"),
            adapter: if up { format!("{ADAPTER} ({ADAPTER_IP})") } else { "not active".into() },
            active: sh.active.load(Relaxed),
            note: sh.note.lock().map(|n| n.clone()).unwrap_or_default(),
            detail,
            need_install: on && !linked,
        }
    }
}

struct Guard(Arc<Shared>);
impl Guard {
    fn new(sh: &Arc<Shared>) -> Self {
        sh.active.fetch_add(1, Relaxed);
        Guard(sh.clone())
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Relaxed);
    }
}

fn set_note(sh: &Shared, s: String) {
    if let Ok(mut n) = sh.note.lock() {
        *n = s;
    }
}

async fn run(sh: Arc<Shared>) {
    let dir = match extract_assets() {
        Ok(d) => d,
        Err(e) => return set_note(&sh, format!("Cannot extract components: {e}")),
    };
    let listener = match bind_proxy().await {
        Ok(l) => l,
        Err(e) => return set_note(&sh, format!("Cannot open proxy port {PROXY_PORT}: {e}")),
    };
    let mut set: JoinSet<()> = JoinSet::new();
    set.spawn(device_monitor(sh.clone()));
    set.spawn(control_loop(sh.clone()));
    set.spawn(forwarder(sh.clone(), listener));
    set.spawn(dns::run(sh.clone()));
    set.spawn(tunnel_manager(sh.clone(), dir));
    while set.join_next().await.is_some() {}
}

fn extract_assets() -> io::Result<PathBuf> {
    let base = std::env::var("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
    let dir = base.join("WiFiShare");
    std::fs::create_dir_all(&dir)?;
    let files: [(&str, &[u8]); 2] = [("tun2socks.exe", TUN2SOCKS), ("wintun.dll", WINTUN)];
    for (name, data) in files {
        let p = dir.join(name);
        let same = std::fs::metadata(&p).map(|m| m.len() == data.len() as u64).unwrap_or(false);
        if !same {
            std::fs::write(&p, data)?;
        }
    }
    Ok(dir)
}

async fn bind_proxy() -> io::Result<TcpListener> {
    let s = TcpSocket::new_v4()?;
    s.set_reuseaddr(true)?;
    s.bind(([127, 0, 0, 1], PROXY_PORT).into())?;
    s.listen(1024)
}

async fn sc(args: &[&str]) -> (Option<i32>, String) {
    match Command::new("sc")
        .args(args)
        .stdin(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await
    {
        Ok(o) => {
            let mut t = String::from_utf8_lossy(&o.stdout).to_string();
            t.push_str(&String::from_utf8_lossy(&o.stderr));
            (o.status.code(), t)
        }
        Err(e) => (None, e.to_string()),
    }
}

const APPLE_SERVICE: &str = "Apple Mobile Device Service";

/// Finds out why the Apple USB service is unreachable and tries to start it.
async fn diagnose(sh: &Shared) {
    let (code, text) = sc(&["query", APPLE_SERVICE]).await;
    if code == Some(1060) || text.contains("1060") {
        sh.apple.store(3, Relaxed);
    } else if text.contains("RUNNING") {
        sh.apple.store(1, Relaxed);
    } else if text.contains("STOPPED") || text.contains("PAUSED") || text.contains("PENDING") {
        sh.apple.store(2, Relaxed);
        let _ = sc(&["config", APPLE_SERVICE, "start=", "auto"]).await;
        let _ = sc(&["start", APPLE_SERVICE]).await;
    } else {
        sh.apple.store(0, Relaxed);
    }
}

async fn device_monitor(sh: Arc<Shared>) {
    let mut last_diag: Option<Instant> = None;
    loop {
        match muxd::usb_device().await {
            Ok(id) => {
                sh.muxd_ok.store(true, Relaxed);
                sh.apple.store(1, Relaxed);
                sh.device_id.store(id.unwrap_or(0), Relaxed);
                if let Ok(mut e) = sh.muxd_err.lock() {
                    e.clear();
                }
            }
            Err(e) => {
                sh.muxd_ok.store(false, Relaxed);
                sh.device_id.store(0, Relaxed);
                if let Ok(mut m) = sh.muxd_err.lock() {
                    *m = e.to_string();
                }
                if last_diag.map_or(true, |t| t.elapsed() > Duration::from_secs(5)) {
                    last_diag = Some(Instant::now());
                    diagnose(&sh).await;
                }
            }
        }
        sleep(Duration::from_millis(1500)).await;
    }
}

async fn read_line<R: AsyncBufReadExt + Unpin>(r: &mut R, line: &mut String, d: Duration) -> io::Result<()> {
    line.clear();
    match timeout(d, r.read_line(line)).await {
        Ok(Ok(0)) => Err(io::ErrorKind::UnexpectedEof.into()),
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(io::ErrorKind::TimedOut.into()),
    }
}

async fn control_loop(sh: Arc<Shared>) {
    let pc = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Windows-PC".into());
    loop {
        let dev = sh.device_id.load(Relaxed);
        if dev == 0 {
            sleep(Duration::from_millis(500)).await;
            continue;
        }
        let _ = session(&sh, dev, &pc).await;
        sh.linked.store(false, Relaxed);
        if let Ok(mut p) = sh.phone.lock() {
            p.clear();
        }
        sleep(Duration::from_secs(1)).await;
    }
}

async fn session(sh: &Arc<Shared>, dev: u32, pc: &str) -> io::Result<()> {
    let s = muxd::connect(dev, PHONE_CTRL_PORT).await?;
    let (r, mut w) = s.into_split();
    let mut r = BufReader::new(r);
    let mut line = String::new();
    w.write_all(format!("HELLO {pc}\n").as_bytes()).await?;
    read_line(&mut r, &mut line, Duration::from_secs(4)).await?;
    if !line.starts_with("OK") {
        return Err(io::ErrorKind::InvalidData.into());
    }
    if let Ok(mut p) = sh.phone.lock() {
        *p = line[2..].trim().to_string();
    }
    sh.linked.store(true, Relaxed);
    loop {
        sleep(Duration::from_millis(1500)).await;
        w.write_all(b"PING\n").await?;
        read_line(&mut r, &mut line, Duration::from_secs(4)).await?;
        if sh.device_id.load(Relaxed) != dev {
            return Err(io::ErrorKind::ConnectionAborted.into());
        }
    }
}

async fn forwarder(sh: Arc<Shared>, l: TcpListener) {
    let mut set: JoinSet<()> = JoinSet::new();
    loop {
        match l.accept().await {
            Ok((c, _)) => {
                while set.try_join_next().is_some() {}
                let sh = sh.clone();
                set.spawn(async move {
                    let _g = Guard::new(&sh);
                    let _ = pipe(c, &sh).await;
                });
            }
            Err(_) => sleep(Duration::from_millis(50)).await,
        }
    }
}

async fn pipe(mut c: TcpStream, sh: &Shared) -> io::Result<()> {
    let dev = sh.device_id.load(Relaxed);
    if dev == 0 || !sh.linked.load(Relaxed) {
        return Ok(());
    }
    let _ = c.set_nodelay(true);
    let sr = socket2::SockRef::from(&c);
    let _ = sr.set_recv_buffer_size(SOCK_BUF);
    let _ = sr.set_send_buffer_size(SOCK_BUF);
    let mut up = muxd::connect(dev, PHONE_SOCKS_PORT).await?;
    let sr = socket2::SockRef::from(&up);
    let _ = sr.set_recv_buffer_size(SOCK_BUF);
    let _ = sr.set_send_buffer_size(SOCK_BUF);
    copy_bidirectional_with_sizes(&mut c, &mut up, BUF, BUF).await?;
    Ok(())
}

async fn netsh(args: &[&str]) -> bool {
    Command::new("netsh")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn configure() -> bool {
    let name = format!("name={ADAPTER}");
    let iface = format!("interface={ADAPTER}");
    let addr = format!("addr={ADAPTER_IP}");
    let hop = format!("nexthop={GATEWAY_IP}");
    let ok = netsh(&["interface", "ip", "set", "address", &name, "source=static", &addr, "mask=255.255.255.0"]).await;
    netsh(&["interface", "ip", "set", "dns", &name, "source=static", "address=127.0.0.1", "validate=no"]).await;
    netsh(&["interface", "ipv4", "set", "interface", &iface, "metric=1"]).await;
    let r1 = netsh(&["interface", "ipv4", "add", "route", "prefix=0.0.0.0/1", &iface, &hop, "metric=1", "store=active"]).await;
    let r2 = netsh(&["interface", "ipv4", "add", "route", "prefix=128.0.0.0/1", &iface, &hop, "metric=1", "store=active"]).await;
    ok && r1 && r2
}

async fn tunnel_manager(sh: Arc<Shared>, dir: PathBuf) {
    loop {
        while !sh.linked.load(Relaxed) {
            sleep(Duration::from_millis(300)).await;
        }
        let mut cmd = Command::new(dir.join("tun2socks.exe"));
        cmd.arg("-device")
            .arg(format!("tun://{ADAPTER}"))
            .arg("-proxy")
            .arg(format!("socks5://127.0.0.1:{PROXY_PORT}"))
            .arg("-loglevel")
            .arg("error")
            .current_dir(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .creation_flags(CREATE_NO_WINDOW);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                set_note(&sh, format!("Cannot start tun2socks: {e}"));
                sleep(Duration::from_secs(3)).await;
                continue;
            }
        };
        let probe = format!("name={ADAPTER}");
        let mut ready = false;
        for _ in 0..40 {
            if let Ok(Some(_)) = child.try_wait() {
                break;
            }
            if netsh(&["interface", "show", "interface", &probe]).await {
                ready = true;
                break;
            }
            sleep(Duration::from_millis(250)).await;
        }
        if ready {
            if configure().await {
                sh.tun_up.store(true, Relaxed);
                set_note(&sh, String::new());
            } else {
                set_note(&sh, "Adapter setup failed - run the app as Administrator".into());
            }
        } else {
            set_note(&sh, "Network adapter was not created (administrator rights / wintun.dll)".into());
        }
        loop {
            if !sh.linked.load(Relaxed) {
                break;
            }
            if let Ok(Some(_)) = child.try_wait() {
                break;
            }
            sleep(Duration::from_millis(500)).await;
        }
        sh.tun_up.store(false, Relaxed);
        let _ = child.kill().await;
        sleep(Duration::from_secs(1)).await;
    }
}
