//! Control channel (port 1081): handshake + heartbeat with the Windows receiver.
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};

static LINKS: AtomicUsize = AtomicUsize::new(0);
static PC_NAME: Mutex<String> = Mutex::new(String::new());
static DEV_NAME: Mutex<String> = Mutex::new(String::new());

pub fn linked() -> bool {
    LINKS.load(Relaxed) > 0
}

pub fn pc_name() -> String {
    PC_NAME.lock().map(|g| g.clone()).unwrap_or_default()
}

pub fn set_device_name(s: String) {
    if let Ok(mut g) = DEV_NAME.lock() {
        *g = s;
    }
}

pub fn reset() {
    LINKS.store(0, Relaxed);
    if let Ok(mut g) = PC_NAME.lock() {
        g.clear();
    }
}

struct LinkGuard;
impl LinkGuard {
    fn new() -> Self {
        LINKS.fetch_add(1, Relaxed);
        LinkGuard
    }
}
impl Drop for LinkGuard {
    fn drop(&mut self) {
        let _ = LINKS.fetch_update(Relaxed, Relaxed, |v| Some(v.saturating_sub(1)));
        if LINKS.load(Relaxed) == 0 {
            if let Ok(mut g) = PC_NAME.lock() {
                g.clear();
            }
        }
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

async fn session(s: TcpStream) -> io::Result<()> {
    let _ = s.set_nodelay(true);
    let (r, mut w) = s.into_split();
    let mut r = BufReader::new(r);
    let mut line = String::new();
    read_line(&mut r, &mut line, Duration::from_secs(5)).await?;
    let name: String = match line.strip_prefix("HELLO ") {
        Some(rest) => rest.trim().chars().take(64).collect(),
        None => return Ok(()),
    };
    let _g = LinkGuard::new();
    if let Ok(mut g) = PC_NAME.lock() {
        *g = name;
    }
    let dev = DEV_NAME.lock().map(|g| g.clone()).unwrap_or_default();
    w.write_all(format!("OK {dev}\n").as_bytes()).await?;
    loop {
        read_line(&mut r, &mut line, Duration::from_secs(6)).await?;
        if line.starts_with("PING") {
            w.write_all(b"PONG\n").await?;
        }
    }
}

pub async fn serve(listener: TcpListener) {
    let mut set: JoinSet<()> = JoinSet::new();
    loop {
        match listener.accept().await {
            Ok((s, _)) => {
                while set.try_join_next().is_some() {}
                set.spawn(async move {
                    let _ = session(s).await;
                });
            }
            Err(_) => sleep(Duration::from_millis(50)).await,
        }
    }
}
