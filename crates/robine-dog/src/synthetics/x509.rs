//! What the SSL tests read from a certificate (DER): subject, issuer,
//! validity and DNS names. Only enough DER to walk to them.

#[derive(Debug, Clone, PartialEq)]
pub struct Cert {
    pub subject: String,
    pub issuer: String,
    /// Epoch milliseconds.
    pub not_before: i64,
    pub not_after: i64,
    pub dns_names: Vec<String>,
}

/// A TLV: its tag, its contents, and what follows it.
fn tlv(b: &[u8]) -> Result<(u8, &[u8], &[u8]), String> {
    let tag = *b.first().ok_or("truncated DER")?;
    let first = *b.get(1).ok_or("truncated DER")? as usize;
    let (len, head) = if first < 0x80 {
        (first, 2)
    } else {
        let n = first & 0x7f;
        if n == 0 || n > 4 {
            return Err("unsupported DER length".into());
        }
        let bytes = b.get(2..2 + n).ok_or("truncated DER")?;
        (
            bytes.iter().fold(0usize, |a, x| (a << 8) | *x as usize),
            2 + n,
        )
    };
    let body = b.get(head..head + len).ok_or("truncated DER")?;
    Ok((tag, body, &b[head + len..]))
}

fn children(mut b: &[u8]) -> Result<Vec<(u8, &[u8])>, String> {
    let mut out = Vec::new();
    while !b.is_empty() {
        let (t, body, rest) = tlv(b)?;
        out.push((t, body));
        b = rest;
    }
    Ok(out)
}

/// `CN=example.com, O=Example`.
fn name(b: &[u8]) -> Result<String, String> {
    let mut parts = Vec::new();
    for (_, set) in children(b)? {
        for (_, atv) in children(set)? {
            let kids = children(atv)?;
            let (Some((_, oid)), Some((_, value))) = (kids.first(), kids.get(1)) else {
                continue;
            };
            let key = match oid {
                [0x55, 0x04, 0x03] => "CN",
                [0x55, 0x04, 0x06] => "C",
                [0x55, 0x04, 0x07] => "L",
                [0x55, 0x04, 0x08] => "ST",
                [0x55, 0x04, 0x0a] => "O",
                [0x55, 0x04, 0x0b] => "OU",
                _ => continue,
            };
            parts.push(format!("{key}={}", String::from_utf8_lossy(value)));
        }
    }
    Ok(parts.join(", "))
}

fn time(tag: u8, b: &[u8]) -> Result<i64, String> {
    let s = std::str::from_utf8(b).map_err(|_| "bad time")?;
    let s = s.trim_end_matches('Z');
    let full = match tag {
        0x17 => {
            let yy: i32 = s.get(..2).and_then(|y| y.parse().ok()).ok_or("bad time")?;
            format!("{}{s}", if yy >= 50 { "19" } else { "20" })
        }
        0x18 => s.to_string(),
        _ => return Err("bad time".into()),
    };
    let dt =
        chrono::NaiveDateTime::parse_from_str(full.get(..14).ok_or("bad time")?, "%Y%m%d%H%M%S")
            .map_err(|e| e.to_string())?;
    Ok(dt.and_utc().timestamp_millis())
}

pub fn parse(der: &[u8]) -> Result<Cert, String> {
    let (_, cert, _) = tlv(der)?;
    let (_, tbs, _) = tlv(cert)?;
    let mut f = children(tbs)?.into_iter().peekable();
    // The version is optional and explicitly tagged [0].
    if f.peek().is_some_and(|(t, _)| *t == 0xa0) {
        f.next();
    }
    let _serial = f.next().ok_or("no serial")?;
    let _alg = f.next().ok_or("no algorithm")?;
    let issuer = name(f.next().ok_or("no issuer")?.1)?;
    let validity = children(f.next().ok_or("no validity")?.1)?;
    let (Some(nb), Some(na)) = (validity.first(), validity.get(1)) else {
        return Err("bad validity".into());
    };
    let subject = name(f.next().ok_or("no subject")?.1)?;
    let _spki = f.next();
    let mut dns_names = Vec::new();
    for (t, body) in f {
        if t != 0xa3 {
            continue;
        }
        let (_, exts, _) = tlv(body)?;
        for (_, ext) in children(exts)? {
            let parts = children(ext)?;
            if parts.first().map(|p| p.1) != Some(&[0x55, 0x1d, 0x11][..]) {
                continue;
            }
            // extnValue is the last part: an OCTET STRING around the names.
            let Some((_, value)) = parts.last() else {
                continue;
            };
            let (_, names, _) = tlv(value)?;
            for (tag, n) in children(names)? {
                if tag == 0x82 {
                    dns_names.push(String::from_utf8_lossy(n).into_owned());
                }
            }
        }
    }
    Ok(Cert {
        subject,
        issuer,
        not_before: time(nb.0, nb.1)?,
        not_after: time(na.0, na.1)?,
        dns_names,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(tag: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        if body.len() < 0x80 {
            out.push(body.len() as u8);
        } else {
            out.push(0x82);
            out.extend_from_slice(&(body.len() as u16).to_be_bytes());
        }
        out.extend_from_slice(body);
        out
    }

    fn rdn(oid: &[u8], v: &str) -> Vec<u8> {
        let atv = [enc(0x06, oid), enc(0x0c, v.as_bytes())].concat();
        enc(0x31, &enc(0x30, &atv))
    }

    #[test]
    fn reads_a_certificate() {
        let issuer = enc(
            0x30,
            &[rdn(&[0x55, 4, 6], "US"), rdn(&[0x55, 4, 3], "Test CA")].concat(),
        );
        let subject = enc(0x30, &rdn(&[0x55, 4, 3], "example.com"));
        let validity = enc(
            0x30,
            &[enc(0x17, b"250101000000Z"), enc(0x18, b"20260301120000Z")].concat(),
        );
        let san = enc(
            0x30,
            &[enc(0x82, b"example.com"), enc(0x82, b"www.example.com")].concat(),
        );
        let ext = enc(
            0x30,
            &[enc(0x06, &[0x55, 0x1d, 0x11]), enc(0x04, &san)].concat(),
        );
        let tbs = enc(
            0x30,
            &[
                enc(0xa0, &enc(0x02, &[2])),
                enc(0x02, &[1, 2, 3]),
                enc(0x30, &enc(0x06, &[1, 2])),
                issuer,
                validity,
                subject,
                enc(0x30, &[0x05, 0x00]),
                enc(0xa3, &enc(0x30, &ext)),
            ]
            .concat(),
        );
        let der = enc(0x30, &[tbs, enc(0x30, &[]), enc(0x03, &[0])].concat());
        let c = parse(&der).unwrap();
        assert_eq!(c.issuer, "C=US, CN=Test CA");
        assert_eq!(c.subject, "CN=example.com");
        assert_eq!(c.not_before, 1_735_689_600_000);
        assert_eq!(c.not_after, 1_772_366_400_000);
        assert_eq!(c.dns_names, vec!["example.com", "www.example.com"]);
        assert!(parse(&der[..10]).is_err());
    }
}
