//! Minimal usbmuxd client (Apple Mobile Device Service listens on 127.0.0.1:27015).
use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};

const ADDRS: [&str; 2] = ["127.0.0.1:27015", "[::1]:27015"];
const HEAD: &str = r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict>"#;
const COMMON: &str = "<key>ClientVersionString</key><string>wifishare</string><key>ProgName</key><string>wifishare</string><key>kLibUSBMuxVersion</key><integer>3</integer>";

async fn open() -> io::Result<TcpStream> {
    let mut first: Option<io::Error> = None;
    for a in ADDRS {
        match timeout(Duration::from_secs(2), TcpStream::connect(a)).await {
            Ok(Ok(s)) => {
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Ok(Err(e)) => {
                if first.is_none() {
                    first = Some(io::Error::new(e.kind(), format!("cannot connect to {a}: {e}")));
                }
            }
            Err(_) => {
                if first.is_none() {
                    first = Some(io::Error::new(io::ErrorKind::TimedOut, format!("connect to {a} timed out")));
                }
            }
        }
    }
    Err(first.unwrap_or_else(|| io::ErrorKind::NotFound.into()))
}

async fn request(s: &mut TcpStream, body: &str) -> io::Result<String> {
    let b = body.as_bytes();
    let len = 16 + b.len();
    let mut pkt = Vec::with_capacity(len);
    pkt.extend_from_slice(&(len as u32).to_le_bytes());
    pkt.extend_from_slice(&1u32.to_le_bytes()); // version: plist
    pkt.extend_from_slice(&8u32.to_le_bytes()); // message: plist
    pkt.extend_from_slice(&1u32.to_le_bytes()); // tag
    pkt.extend_from_slice(b);
    let io = async {
        s.write_all(&pkt).await?;
        let mut h = [0u8; 16];
        s.read_exact(&mut h).await?;
        let rl = u32::from_le_bytes([h[0], h[1], h[2], h[3]]) as usize;
        if !(16..=(1 << 20)).contains(&rl) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("bad reply header (len {rl})")));
        }
        let mut body = vec![0u8; rl - 16];
        s.read_exact(&mut body).await?;
        let text = String::from_utf8_lossy(&body).into_owned();
        if !text.contains("plist") {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "unexpected (non-plist) reply from Apple service"));
        }
        Ok(text)
    };
    match timeout(Duration::from_secs(4), io).await {
        Ok(r) => r,
        Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "Apple service did not answer in time")),
    }
}

fn int_after(xml: &str, key: &str) -> Option<u32> {
    let k = format!("<key>{key}</key>");
    let i = xml.find(&k)?;
    let rest = xml[i + k.len()..].trim_start().strip_prefix("<integer>")?;
    rest.split("</integer>").next()?.trim().parse().ok()
}

fn parse_usb_device(xml: &str) -> Option<u32> {
    let mut parts = xml.split("<key>DeviceID</key>");
    parts.next();
    for seg in parts {
        if !seg.contains("<string>USB</string>") {
            continue;
        }
        if let Some(r) = seg.trim_start().strip_prefix("<integer>") {
            if let Some(n) = r.split("</integer>").next().and_then(|v| v.trim().parse().ok()) {
                return Some(n);
            }
        }
    }
    None
}

/// Err if the Apple USB service is unreachable, Ok(None) if no iPhone is attached.
pub async fn usb_device() -> io::Result<Option<u32>> {
    let mut s = open().await?;
    let body = format!("{HEAD}<key>MessageType</key><string>ListDevices</string>{COMMON}</dict></plist>");
    let resp = request(&mut s, &body).await?;
    Ok(parse_usb_device(&resp))
}

/// Opens a raw TCP tunnel to `port` on the device.
pub async fn connect(dev: u32, port: u16) -> io::Result<TcpStream> {
    let mut s = open().await?;
    let sw = port.swap_bytes() as u32;
    let body = format!(
        "{HEAD}<key>MessageType</key><string>Connect</string><key>DeviceID</key><integer>{dev}</integer><key>PortNumber</key><integer>{sw}</integer>{COMMON}</dict></plist>"
    );
    let resp = request(&mut s, &body).await?;
    match int_after(&resp, "Number") {
        Some(0) => Ok(s),
        _ => Err(io::Error::new(io::ErrorKind::ConnectionRefused, "device refused (is the phone app open?)")),
    }
}
