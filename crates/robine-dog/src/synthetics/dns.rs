//! A small DNS client for the DNS tests: one question per query, over UDP,
//! and over TCP when the answer is truncated.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const TYPES: &[(&str, u16)] = &[
    ("A", 1),
    ("NS", 2),
    ("CNAME", 5),
    ("MX", 15),
    ("TXT", 16),
    ("AAAA", 28),
];

pub fn query(id: u16, name: &str, qtype: u16) -> Result<Vec<u8>, String> {
    let mut q = Vec::with_capacity(64);
    q.extend_from_slice(&id.to_be_bytes());
    // Recursion desired; one question.
    q.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in name.trim_end_matches('.').split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err(format!("bad domain name {name:?}"));
        }
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0);
    q.extend_from_slice(&qtype.to_be_bytes());
    q.extend_from_slice(&1u16.to_be_bytes());
    Ok(q)
}

/// A name at `at`, following compression pointers; and where it ends.
fn name(msg: &[u8], mut at: usize) -> Result<(String, usize), String> {
    let mut labels = Vec::new();
    let mut end = None;
    for _ in 0..64 {
        let len = *msg.get(at).ok_or("truncated name")? as usize;
        if len & 0xc0 == 0xc0 {
            let ptr = ((len & 0x3f) << 8) | *msg.get(at + 1).ok_or("truncated name")? as usize;
            end.get_or_insert(at + 2);
            at = ptr;
            continue;
        }
        if len == 0 {
            return Ok((labels.join("."), end.unwrap_or(at + 1)));
        }
        let label = msg.get(at + 1..at + 1 + len).ok_or("truncated name")?;
        labels.push(String::from_utf8_lossy(label).into_owned());
        at += 1 + len;
    }
    Err("name loop".into())
}

/// The records of an answer, as text, by type: `A` → `93.184.215.14`,
/// `MX` → `10 mail.example.com`.
pub struct Answer {
    pub truncated: bool,
    pub rcode: u8,
    pub records: Vec<(String, String)>,
}

pub fn parse(msg: &[u8], id: u16) -> Result<Answer, String> {
    if msg.len() < 12 {
        return Err("short DNS answer".into());
    }
    if u16::from_be_bytes([msg[0], msg[1]]) != id {
        return Err("DNS answer to another query".into());
    }
    let truncated = msg[2] & 0x02 != 0;
    let rcode = msg[3] & 0x0f;
    let qd = u16::from_be_bytes([msg[4], msg[5]]);
    let an = u16::from_be_bytes([msg[6], msg[7]]);
    let mut at = 12;
    for _ in 0..qd {
        at = name(msg, at)?.1 + 4;
    }
    let mut records = Vec::new();
    for _ in 0..an {
        let (_, after) = name(msg, at)?;
        let head = msg.get(after..after + 10).ok_or("truncated record")?;
        let typ = u16::from_be_bytes([head[0], head[1]]);
        let len = u16::from_be_bytes([head[8], head[9]]) as usize;
        let start = after + 10;
        let data = msg.get(start..start + len).ok_or("truncated record")?;
        let text = match typ {
            1 if len == 4 => Some(Ipv4Addr::new(data[0], data[1], data[2], data[3]).to_string()),
            28 if len == 16 => {
                let b: [u8; 16] = data.try_into().map_err(|_| "bad AAAA")?;
                Some(Ipv6Addr::from(b).to_string())
            }
            2 | 5 => Some(name(msg, start)?.0),
            15 if len > 2 => Some(format!(
                "{} {}",
                u16::from_be_bytes([data[0], data[1]]),
                name(msg, start + 2)?.0
            )),
            16 => {
                let mut out = String::new();
                let mut i = 0;
                while i < data.len() {
                    let l = data[i] as usize;
                    out.push_str(&String::from_utf8_lossy(
                        data.get(i + 1..i + 1 + l).unwrap_or_default(),
                    ));
                    i += 1 + l;
                }
                Some(out)
            }
            _ => None,
        };
        if let (Some(t), Some((n, _))) = (text, TYPES.iter().find(|(_, c)| *c == typ)) {
            records.push((n.to_string(), t));
        }
        at = start + len;
    }
    Ok(Answer {
        truncated,
        rcode,
        records,
    })
}

/// The first name server of `/etc/resolv.conf`.
pub fn system_server() -> Option<String> {
    let conf = std::fs::read_to_string("/etc/resolv.conf").ok()?;
    conf.lines().find_map(|l| {
        let mut parts = l.split_whitespace();
        (parts.next() == Some("nameserver"))
            .then(|| parts.next().map(String::from))
            .flatten()
    })
}

/// One question to a server: over UDP, again over TCP when truncated.
pub async fn ask(
    server: SocketAddr,
    domain: &str,
    qtype: u16,
    timeout: Duration,
) -> Result<Answer, String> {
    let id: u16 = rand::random();
    let q = query(id, domain, qtype)?;
    let udp = async {
        let bind: SocketAddr = if server.is_ipv4() {
            "0.0.0.0:0".parse().unwrap()
        } else {
            "[::]:0".parse().unwrap()
        };
        let sock = tokio::net::UdpSocket::bind(bind)
            .await
            .map_err(|e| e.to_string())?;
        sock.send_to(&q, server).await.map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 4096];
        loop {
            let (n, from) = sock.recv_from(&mut buf).await.map_err(|e| e.to_string())?;
            if from.ip() == server.ip() && n >= 2 && buf[..2] == id.to_be_bytes() {
                return parse(&buf[..n], id);
            }
        }
    };
    let a = tokio::time::timeout(timeout, udp)
        .await
        .map_err(|_| "no DNS answer in time".to_string())??;
    if !a.truncated {
        return Ok(a);
    }
    let tcp = async {
        let mut s = tokio::net::TcpStream::connect(server)
            .await
            .map_err(|e| e.to_string())?;
        let mut framed = (q.len() as u16).to_be_bytes().to_vec();
        framed.extend_from_slice(&q);
        s.write_all(&framed).await.map_err(|e| e.to_string())?;
        let len = s.read_u16().await.map_err(|e| e.to_string())? as usize;
        let mut buf = vec![0u8; len];
        s.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
        parse(&buf, id)
    };
    tokio::time::timeout(timeout, tcp)
        .await
        .map_err(|_| "no DNS answer in time over TCP".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let q = query(0x1234, "example.com", 1).unwrap();
        assert_eq!(&q[12..25], b"\x07example\x03com\x00");
        // An answer: the question, then an A, a compressed CNAME, an MX and a TXT.
        let mut a = q.clone();
        a[2] = 0x81;
        a[3] = 0x80;
        a[7] = 4;
        let rr = |a: &mut Vec<u8>, typ: u16, data: &[u8]| {
            a.extend_from_slice(&[0xc0, 12]);
            a.extend_from_slice(&typ.to_be_bytes());
            a.extend_from_slice(&[0, 1, 0, 0, 0, 60]);
            a.extend_from_slice(&(data.len() as u16).to_be_bytes());
            a.extend_from_slice(data);
        };
        rr(&mut a, 1, &[93, 184, 215, 14]);
        rr(&mut a, 5, b"\x03www\xc0\x0c");
        rr(&mut a, 15, b"\x00\x0a\x04mail\xc0\x0c");
        rr(&mut a, 16, b"\x05hello\x06 world");
        let ans = parse(&a, 0x1234).unwrap();
        assert!(!ans.truncated);
        assert_eq!(
            ans.records,
            vec![
                ("A".into(), "93.184.215.14".into()),
                ("CNAME".into(), "www.example.com".into()),
                ("MX".into(), "10 mail.example.com".into()),
                ("TXT".into(), "hello world".into()),
            ]
        );
        assert!(parse(&a, 1).is_err());
        assert!(query(1, "a..b", 1).is_err());
    }
}
