//! Digest authentication (RFC 7616, and the older RFC 2069 form): the request goes without it,
//! the server answers 401 with a challenge, the request goes again with the answer to it.
//! MD5, SHA-256 and their `-sess` forms; `qop` auth and auth-int.

/// The Digest challenge among `WWW-Authenticate` headers; SHA-256 when the server offers both.
pub fn challenge(headers: &[(String, String)]) -> Option<&str> {
    let offered: Vec<&str> = headers
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case("www-authenticate"))
        .map(|(_, v)| v.trim())
        .filter(|v| v.len() > 7 && v[..7].eq_ignore_ascii_case("digest "))
        .collect();
    offered.iter().find(|c| params(c).iter().any(|(k, v)| k == "algorithm" && v.to_ascii_uppercase().starts_with("SHA-256"))).or(offered.first()).copied()
}

/// The `Authorization` header answering the challenge for this request; `uri` is the path
/// with the query, as it goes in the request line.
pub fn answer(challenge: &str, method: &str, uri: &str, body: &[u8], username: &str, password: &str, cnonce: &str) -> Result<String, String> {
    let p = params(challenge);
    let get = |key: &str| p.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
    let realm = get("realm").unwrap_or_default();
    let nonce = get("nonce").ok_or(tr_static!("в запросе Digest от сервера нет nonce", "the server's Digest challenge has no nonce"))?;
    let algorithm = get("algorithm").unwrap_or("MD5");
    let (algo, sess) = match algorithm.to_ascii_uppercase().as_str() {
        "MD5" => (Algo::Md5, false),
        "MD5-SESS" => (Algo::Md5, true),
        "SHA-256" => (Algo::Sha256, false),
        "SHA-256-SESS" => (Algo::Sha256, true),
        other => return Err(tr!("Digest: алгоритм {other} не поддерживается (есть MD5 и SHA-256)", "Digest: algorithm {other} is not supported (MD5 and SHA-256 are)")),
    };
    let qops: Vec<&str> = get("qop").map(|q| q.split(',').map(str::trim).collect()).unwrap_or_default();
    let qop = if qops.contains(&"auth") {
        Some("auth")
    } else if qops.contains(&"auth-int") {
        Some("auth-int")
    } else if qops.is_empty() {
        None
    } else {
        return Err(tr!("Digest: qop {} не поддерживается", "Digest: qop {} is not supported", qops.join(", ")));
    };
    let hash = |s: &str| algo.hex(s.as_bytes());
    let nc = "00000001";
    let mut ha1 = hash(&format!("{username}:{realm}:{password}"));
    if sess {
        ha1 = hash(&format!("{ha1}:{nonce}:{cnonce}"));
    }
    let ha2 = match qop {
        Some("auth-int") => hash(&format!("{method}:{uri}:{}", algo.hex(body))),
        _ => hash(&format!("{method}:{uri}")),
    };
    let response = match qop {
        Some(qop) => hash(&format!("{ha1}:{nonce}:{nc}:{cnonce}:{qop}:{ha2}")),
        None => hash(&format!("{ha1}:{nonce}:{ha2}")),
    };
    let quote = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let mut header = format!(
        "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", algorithm={algorithm}, response=\"{response}\"",
        quote(username),
        quote(realm),
        quote(nonce),
        quote(uri)
    );
    if let Some(qop) = qop {
        header.push_str(&format!(", qop={qop}, nc={nc}, cnonce=\"{cnonce}\""));
    }
    if let Some(opaque) = get("opaque") {
        header.push_str(&format!(", opaque=\"{}\"", quote(opaque)));
    }
    Ok(header)
}

/// `Digest realm="x", qop="auth,auth-int", nonce=abc` → pairs, keys lowercase.
fn params(challenge: &str) -> Vec<(String, String)> {
    let rest = challenge.get(7..).unwrap_or_default();
    let mut out = Vec::new();
    let mut chars = rest.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| *c == ',' || c.is_whitespace()) {
            chars.next();
        }
        let key: String = chars.by_ref().take_while(|c| *c != '=').collect::<String>().trim().to_ascii_lowercase();
        if key.is_empty() {
            break;
        }
        let mut value = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => value.extend(chars.next()),
                    '"' => break,
                    c => value.push(c),
                }
            }
        } else {
            while let Some(c) = chars.peek().copied().filter(|c| *c != ',') {
                value.push(c);
                chars.next();
            }
            value = value.trim().to_string();
        }
        out.push((key, value));
    }
    out
}

#[derive(Clone, Copy)]
enum Algo {
    Md5,
    Sha256,
}

impl Algo {
    fn hex(self, bytes: &[u8]) -> String {
        match self {
            Algo::Md5 => hex(&md5(bytes)),
            Algo::Sha256 => hex(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref()),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// MD5 (RFC 1321): Digest still needs it, and it is not worth a crate.
fn md5(input: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16,
        23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64).map(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32).collect();
    let mut state: [u32; 4] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];
    let mut data = input.to_vec();
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&((input.len() as u64).wrapping_mul(8)).to_le_bytes());
    for block in data.chunks(64) {
        let m: Vec<u32> = block.chunks(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect();
        let [mut a, mut b, mut c, mut d] = state;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        for (s, v) in state.iter_mut().zip([a, b, c, d]) {
            *s = s.wrapping_add(v);
        }
    }
    let mut out = [0u8; 16];
    for (i, s) in state.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&s.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md5_hex(s: &str) -> String {
        Algo::Md5.hex(s.as_bytes())
    }

    #[test]
    fn md5_vectors() {
        assert_eq!(md5_hex(""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex("The quick brown fox jumps over the lazy dog"), "9e107d9d372bb6826bd81d3542a419d6");
        assert_eq!(md5_hex(&"a".repeat(1000)), "cabe45dcc9ae5b66ba86600cca6b8ba8");
    }

    #[test]
    fn rfc_7616_examples() {
        // RFC 7616 §3.9.1, the same request with MD5 and with SHA-256.
        let md5 = r#"Digest realm="http-auth@example.org", qop="auth, auth-int", algorithm=MD5, nonce="7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v", opaque="FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS""#;
        let cnonce = "f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ";
        let got = answer(md5, "GET", "/dir/index.html", b"", "Mufasa", "Circle of Life", cnonce).unwrap();
        assert!(got.contains(r#"response="8ca523f5e9506fed4657c9700eebdbec""#), "{got}");
        assert!(got.contains("qop=auth, nc=00000001") && got.contains(r#"opaque="FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS""#), "{got}");

        let sha = md5.replace("MD5", "SHA-256");
        let got = answer(&sha, "GET", "/dir/index.html", b"", "Mufasa", "Circle of Life", cnonce).unwrap();
        assert!(got.contains(r#"response="753927fa0e85d155564e2e272a28d1802ca10daf4496794697cf8db5856cb6c1""#), "{got}");

        let headers = vec![("WWW-Authenticate".to_string(), md5.to_string()), ("www-authenticate".to_string(), sha.clone())];
        assert_eq!(challenge(&headers), Some(sha.as_str()), "SHA-256 when both are offered");
        assert!(answer("Digest realm=\"x\", nonce=\"n\", algorithm=SHA-512-256", "GET", "/", b"", "u", "p", "c").is_err());
    }

    #[test]
    fn rfc_2069_form() {
        // Without qop, as httpbin's older form and some cameras answer.
        let got = answer(r#"Digest realm="testrealm@host.com", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093""#, "GET", "/dir/index.html", b"", "Mufasa", "CircleOfLife", "x").unwrap();
        assert!(got.contains(r#"response="1949323746fe6a43ef61f9606e7febea""#), "{got}");
        assert!(!got.contains("qop"));
    }
}
