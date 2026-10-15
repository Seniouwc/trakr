//! A small OSC 1.0 receiver: parses messages and bundles and listens on a UDP socket without blocking.

use std::net::{SocketAddr, UdpSocket};

/// One OSC argument.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
}

impl Arg {
    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Arg::Int(i) => Some(*i as f32),
            Arg::Float(f) => Some(*f as f32),
            Arg::Bool(b) => Some(*b as u8 as f32),
            Arg::Str(_) => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Arg::Int(i) => Some(*i),
            Arg::Float(f) if f.is_finite() => Some(f.round() as i64),
            Arg::Bool(b) => Some(*b as i64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Arg::Str(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub address: String,
    pub args: Vec<Arg>,
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.i..self.i.checked_add(n)?)?;
        self.i += n;
        Some(s)
    }

    /// A null-terminated string padded to a multiple of four bytes.
    fn string(&mut self) -> Option<String> {
        let rest = self.b.get(self.i..)?;
        let len = rest.iter().position(|c| *c == 0)?;
        let s = String::from_utf8_lossy(&rest[..len]).into_owned();
        let padded = (len + 4) & !3;
        self.take(padded)?;
        Some(s)
    }
}

/// Parse a packet (a message or a bundle of them) into messages. Malformed data yields what could be read.
pub fn parse(packet: &[u8]) -> Vec<Message> {
    let mut out = Vec::new();
    parse_into(packet, &mut out, 0);
    out
}

fn parse_into(packet: &[u8], out: &mut Vec<Message>, depth: usize) {
    if depth > 4 || out.len() > 1024 {
        return;
    }
    if packet.starts_with(b"#bundle\0") {
        let mut r = Reader { b: packet, i: 16 }; // marker + 8 byte time tag
        while let Some(size) = r.take(4) {
            let size = u32::from_be_bytes([size[0], size[1], size[2], size[3]]) as usize;
            let Some(el) = r.take(size) else { return };
            parse_into(el, out, depth + 1);
        }
        return;
    }
    if let Some(m) = parse_message(packet) {
        out.push(m);
    }
}

fn parse_message(packet: &[u8]) -> Option<Message> {
    let mut r = Reader { b: packet, i: 0 };
    let address = r.string()?;
    if !address.starts_with('/') {
        return None;
    }
    // A message without a type tag string has no arguments.
    let tags = if r.i < packet.len() { r.string()? } else { String::new() };
    let mut args = Vec::new();
    for t in tags.strip_prefix(',').unwrap_or("").chars() {
        if args.len() >= 64 {
            break;
        }
        args.push(match t {
            'i' => {
                let b = r.take(4)?;
                Arg::Int(i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as i64)
            }
            'h' => {
                let b = r.take(8)?;
                Arg::Int(i64::from_be_bytes(b.try_into().ok()?))
            }
            'f' => {
                let b = r.take(4)?;
                Arg::Float(f32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f64)
            }
            'd' => {
                let b = r.take(8)?;
                Arg::Float(f64::from_be_bytes(b.try_into().ok()?))
            }
            's' | 'S' => Arg::Str(r.string()?),
            'T' => Arg::Bool(true),
            'F' => Arg::Bool(false),
            'N' | 'I' => continue,
            _ => return None, // unknown type: the rest cannot be located
        });
    }
    Some(Message { address, args })
}

/// Build a message (for tests and for clients written in Rust).
pub fn encode(address: &str, args: &[Arg]) -> Vec<u8> {
    fn pad_str(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(s.as_bytes());
        out.push(0);
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
    }
    let mut out = Vec::new();
    pad_str(&mut out, address);
    let mut tags = String::from(",");
    for a in args {
        tags.push(match a {
            Arg::Int(_) => 'i',
            Arg::Float(_) => 'f',
            Arg::Str(_) => 's',
            Arg::Bool(true) => 'T',
            Arg::Bool(false) => 'F',
        });
    }
    pad_str(&mut out, &tags);
    for a in args {
        match a {
            Arg::Int(i) => out.extend_from_slice(&(*i as i32).to_be_bytes()),
            Arg::Float(f) => out.extend_from_slice(&(*f as f32).to_be_bytes()),
            Arg::Str(s) => pad_str(&mut out, s),
            Arg::Bool(_) => {}
        }
    }
    out
}

/// Listens for OSC on a UDP port; poll from the UI thread.
pub struct OscServer {
    socket: UdpSocket,
    buf: Vec<u8>,
}

impl OscServer {
    /// Bind `addr` (use `127.0.0.1:port` to accept only local senders; port 0 picks a free one).
    pub fn bind(addr: SocketAddr) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        Ok(Self { socket, buf: vec![0; 65_536] })
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// Append every message received since the last call (at most 256 packets per call).
    pub fn poll(&mut self, out: &mut Vec<Message>) {
        for _ in 0..256 {
            match self.socket.recv_from(&mut self.buf) {
                Ok((n, _)) => parse_into(&self.buf[..n], out, 0),
                Err(_) => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_roundtrip() {
        let args =
            [Arg::Int(-7), Arg::Float(0.5), Arg::Str("hello".into()), Arg::Bool(true), Arg::Bool(false)];
        let m = parse(&encode("/trakr/x", &args));
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].address, "/trakr/x");
        assert_eq!(m[0].args, args);
        assert_eq!(m[0].args[1].as_f32(), Some(0.5));
        assert_eq!(m[0].args[0].as_i64(), Some(-7));
        assert_eq!(m[0].args[2].as_str(), Some("hello"));
    }

    #[test]
    fn bundles_and_oddities() {
        let a = encode("/a", &[Arg::Int(1)]);
        let b = encode("/b", &[]);
        let mut bundle = b"#bundle\0".to_vec();
        bundle.extend_from_slice(&[0; 8]);
        for el in [&a, &b] {
            bundle.extend_from_slice(&(el.len() as u32).to_be_bytes());
            bundle.extend_from_slice(el);
        }
        let m = parse(&bundle);
        assert_eq!(m.iter().map(|m| m.address.as_str()).collect::<Vec<_>>(), ["/a", "/b"]);
        // no type tags at all
        assert_eq!(parse(b"/bare\0\0\0")[0].args.len(), 0);
        // not an address
        assert!(parse(b"hello\0\0\0").is_empty());
    }

    #[test]
    fn hostile_input_never_panics() {
        let good = encode("/trakr/test", &[Arg::Int(5), Arg::Str("s".into()), Arg::Float(1.0)]);
        let mut seed = 7u32;
        let mut rnd = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            seed >> 8
        };
        for _ in 0..20_000 {
            let mut p = good.clone();
            for _ in 0..1 + rnd() % 4 {
                let i = rnd() as usize % p.len();
                p[i] = rnd() as u8;
            }
            if rnd() % 3 == 0 {
                p.truncate(rnd() as usize % p.len());
            }
            let _ = parse(&p);
            // and as a bundle element with a wild size
            let mut b = b"#bundle\0".to_vec();
            b.extend_from_slice(&[0; 8]);
            b.extend_from_slice(&rnd().to_be_bytes());
            b.extend_from_slice(&p);
            let _ = parse(&b);
        }
        let _ = parse(&[]);
        let _ = parse(b"#bundle\0");
    }

    #[test]
    fn udp_server_receives_packets() {
        let mut server = OscServer::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let to = server.local_addr().unwrap();
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client.send_to(&encode("/ping", &[Arg::Int(3)]), to).unwrap();
        let mut got = Vec::new();
        for _ in 0..100 {
            server.poll(&mut got);
            if !got.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].address, "/ping");
    }
}
