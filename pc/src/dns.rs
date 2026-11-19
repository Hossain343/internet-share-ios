//! Local DNS stub on 127.0.0.1:53. Queries are sent over TCP through the phone (UDP is not tunneled).
use std::collections::HashMap;
use std::io;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UdpSocket;
use tokio::time::{sleep, timeout};

use crate::engine::{Shared, PHONE_SOCKS_PORT};
use crate::muxd;

type Cache = Arc<Mutex<HashMap<Vec<u8>, (Instant, Vec<u8>)>>>;

pub async fn run(sh: Arc<Shared>) {
    let sock = match UdpSocket::bind("127.0.0.1:53").await {
        Ok(s) => Arc::new(s),
        Err(e) => {
            if let Ok(mut n) = sh.note.lock() {
                *n = format!("DNS port 53 unavailable ({e}); websites by name may not load");
            }
            return;
        }
    };
    let cache: Cache = Arc::new(Mutex::new(HashMap::new()));
    let mut buf = vec![0u8; 4096];
    loop {
        let (n, from) = match sock.recv_from(&mut buf).await {
            Ok(x) => x,
            Err(_) => {
                sleep(Duration::from_millis(10)).await;
                continue;
            }
        };
        if n < 12 {
            continue;
        }
        let q = buf[..n].to_vec();
        let (sock, sh, cache) = (sock.clone(), sh.clone(), cache.clone());
        tokio::spawn(async move {
            if let Some(resp) = resolve(&sh, &cache, &q).await {
                let _ = sock.send_to(&resp, from).await;
            }
        });
    }
}

async fn resolve(sh: &Shared, cache: &Cache, q: &[u8]) -> Option<Vec<u8>> {
    let key = q[2..].to_vec();
    {
        let c = cache.lock().ok()?;
        if let Some((t, r)) = c.get(&key) {
            if t.elapsed() < Duration::from_secs(30) {
                let mut r = r.clone();
                r[0] = q[0];
                r[1] = q[1];
                return Some(r);
            }
        }
    }
    let dev = sh.device_id.load(Relaxed);
    if dev == 0 {
        return None;
    }
    let resp = timeout(Duration::from_secs(5), upstream(dev, q)).await.ok()?.ok()?;
    if resp.len() > 12 && resp[3] & 0x0f == 0 {
        if let Ok(mut c) = cache.lock() {
            if c.len() > 2048 {
                c.clear();
            }
            c.insert(key, (Instant::now(), resp.clone()));
        }
    }
    Some(resp)
}

async fn upstream(dev: u32, q: &[u8]) -> io::Result<Vec<u8>> {
    let mut s = muxd::connect(dev, PHONE_SOCKS_PORT).await?;
    s.write_all(&[5, 1, 0]).await?;
    let mut r = [0u8; 2];
    s.read_exact(&mut r).await?;
    s.write_all(&[5, 1, 0, 1, 1, 1, 1, 1, 0, 53]).await?; // CONNECT 1.1.1.1:53
    let mut rep = [0u8; 10];
    s.read_exact(&mut rep).await?;
    if rep[1] != 0 {
        return Err(io::ErrorKind::ConnectionRefused.into());
    }
    let mut m = Vec::with_capacity(q.len() + 2);
    m.extend_from_slice(&(q.len() as u16).to_be_bytes());
    m.extend_from_slice(q);
    s.write_all(&m).await?;
    let mut l = [0u8; 2];
    s.read_exact(&mut l).await?;
    let mut resp = vec![0u8; u16::from_be_bytes(l) as usize];
    s.read_exact(&mut resp).await?;
    Ok(resp)
}
