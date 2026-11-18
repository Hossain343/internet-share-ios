use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::os::fd::AsRawFd;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::io::{copy_bidirectional_with_sizes, AsyncReadExt, AsyncWriteExt};
use tokio::net::{lookup_host, TcpListener, TcpSocket, TcpStream};
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::ACTIVE;

const BUF: usize = 512 * 1024; // relay buffer per direction
const SOCK_BUF: usize = 2 * 1024 * 1024; // kernel socket buffers
const CONNECT_TIMEOUT: Duration = Duration::from_secs(6);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

// Darwin socket options (not always exported by libc).
const IP_BOUND_IF: libc::c_int = 25;
const IPV6_BOUND_IF: libc::c_int = 125;

enum Target {
    Addr(SocketAddr),
    Host(String, u16),
}

struct Guard;
impl Guard {
    fn new() -> Self {
        ACTIVE.fetch_add(1, Ordering::Relaxed);
        Guard
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::Relaxed);
    }
}

pub async fn bind(port: u16) -> io::Result<TcpListener> {
    let s = TcpSocket::new_v4()?;
    s.set_reuseaddr(true)?;
    s.bind(SocketAddr::from(([127, 0, 0, 1], port)))?;
    s.listen(1024)
}

fn wifi_index() -> u32 {
    unsafe { libc::if_nametoindex(b"en0\0".as_ptr() as *const libc::c_char) }
}

fn bind_to_wifi(s: &TcpSocket, v4: bool, idx: u32) {
    if idx == 0 {
        return;
    }
    let (level, opt) = if v4 {
        (libc::IPPROTO_IP, IP_BOUND_IF)
    } else {
        (libc::IPPROTO_IPV6, IPV6_BOUND_IF)
    };
    unsafe {
        libc::setsockopt(
            s.as_raw_fd(),
            level,
            opt,
            &idx as *const u32 as *const libc::c_void,
            std::mem::size_of::<u32>() as libc::socklen_t,
        );
    }
}

pub async fn serve(listener: TcpListener) {
    let idx = wifi_index();
    let mut set: JoinSet<()> = JoinSet::new();
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                while set.try_join_next().is_some() {}
                set.spawn(async move {
                    let _g = Guard::new();
                    let _ = handle(stream, idx).await;
                });
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
}

async fn handshake(c: &mut TcpStream) -> io::Result<Option<Target>> {
    let mut h = [0u8; 2];
    c.read_exact(&mut h).await?;
    if h[0] != 5 {
        return Ok(None);
    }
    let mut methods = vec![0u8; h[1] as usize];
    c.read_exact(&mut methods).await?;
    c.write_all(&[5, 0]).await?; // no auth (loopback only)

    let mut r = [0u8; 4];
    c.read_exact(&mut r).await?;
    if r[0] != 5 || r[1] != 1 {
        c.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await?; // only CONNECT
        return Ok(None);
    }
    let target = match r[3] {
        1 => {
            let mut a = [0u8; 4];
            c.read_exact(&mut a).await?;
            let port = read_port(c).await?;
            Target::Addr(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(a)), port))
        }
        3 => {
            let mut l = [0u8; 1];
            c.read_exact(&mut l).await?;
            let mut d = vec![0u8; l[0] as usize];
            c.read_exact(&mut d).await?;
            let port = read_port(c).await?;
            let host = String::from_utf8(d)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad host"))?;
            Target::Host(host, port)
        }
        4 => {
            let mut a = [0u8; 16];
            c.read_exact(&mut a).await?;
            let port = read_port(c).await?;
            Target::Addr(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(a)), port))
        }
        _ => {
            c.write_all(&[5, 8, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            return Ok(None);
        }
    };
    Ok(Some(target))
}

async fn read_port(c: &mut TcpStream) -> io::Result<u16> {
    let mut p = [0u8; 2];
    c.read_exact(&mut p).await?;
    Ok(u16::from_be_bytes(p))
}

async fn connect(t: Target, idx: u32) -> io::Result<TcpStream> {
    let addrs: Vec<SocketAddr> = match t {
        Target::Addr(a) => vec![a],
        Target::Host(h, p) => lookup_host((h.as_str(), p)).await?.collect(),
    };
    let mut last = io::Error::new(io::ErrorKind::NotFound, "no address");
    for a in addrs {
        let s = if a.is_ipv4() { TcpSocket::new_v4()? } else { TcpSocket::new_v6()? };
        bind_to_wifi(&s, a.is_ipv4(), idx);
        let _ = s.set_recv_buffer_size(SOCK_BUF as u32);
        let _ = s.set_send_buffer_size(SOCK_BUF as u32);
        match timeout(CONNECT_TIMEOUT, s.connect(a)).await {
            Ok(Ok(st)) => {
                let _ = st.set_nodelay(true);
                return Ok(st);
            }
            Ok(Err(e)) => last = e,
            Err(_) => last = io::Error::new(io::ErrorKind::TimedOut, "connect timeout"),
        }
    }
    Err(last)
}

async fn handle(mut c: TcpStream, idx: u32) -> io::Result<()> {
    let _ = c.set_nodelay(true);
    let sr = socket2::SockRef::from(&c);
    let _ = sr.set_recv_buffer_size(SOCK_BUF);
    let _ = sr.set_send_buffer_size(SOCK_BUF);

    let target = match timeout(HANDSHAKE_TIMEOUT, handshake(&mut c)).await {
        Ok(r) => match r? {
            Some(t) => t,
            None => return Ok(()),
        },
        Err(_) => return Ok(()),
    };
    let mut up = match connect(target, idx).await {
        Ok(s) => s,
        Err(_) => {
            c.write_all(&[5, 4, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            return Ok(());
        }
    };
    c.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
    let _ = copy_bidirectional_with_sizes(&mut c, &mut up, BUF, BUF).await;
    Ok(())
}
