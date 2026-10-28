//! `cairn health`: the container healthcheck. `scratch` has no shell, wget or curl, so the binary asks itself.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

/// True when `GET /api/health` on `addr` answers 200. An unspecified address (0.0.0.0, ::) is asked on 127.0.0.1.
pub fn check(addr: SocketAddr) -> bool {
    let target = if addr.ip().is_unspecified() {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), addr.port())
    } else {
        addr
    };
    let timeout = Duration::from_secs(2);
    let Ok(mut stream) = TcpStream::connect_timeout(&target, timeout) else {
        return false;
    };
    if stream.set_read_timeout(Some(timeout)).is_err()
        || stream
            .write_all(b"GET /api/health HTTP/1.0\r\nHost: localhost\r\n\r\n")
            .is_err()
    {
        return false;
    }
    let mut head = [0u8; 16];
    let mut read = 0;
    while read < head.len() {
        match stream.read(&mut head[read..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => read += n,
        }
    }
    let line = &head[..read];
    line.starts_with(b"HTTP/1.") && line.get(8..13) == Some(b" 200 ")
}
