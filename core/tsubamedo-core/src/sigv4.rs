//! AWS Signature Version 4: the request is signed with its own method, path, query, headers and
//! body, so it is signed right before it goes, when all of them are final, and again whenever it
//! goes again. The path and the query are rewritten in the encoding AWS signs (everything but
//! `A-Z a-z 0-9 - _ . ~` as `%XX`), so what goes is what was signed.
//!
//! Signed: the host and every header the request has, except a few a proxy or the transport may
//! change. S3 also gets `X-Amz-Content-Sha256`; a value written by hand (`UNSIGNED-PAYLOAD`) is
//! kept. So is a hand-written `X-Amz-Date`.

use ring::{digest, hmac};

use crate::model::AwsV4;
use crate::prepare::Prepared;

/// Headers left out of the signature: the transport or a proxy may change them.
const UNSIGNED: &[&str] = &["authorization", "user-agent", "expect", "connection", "keep-alive", "proxy-connection", "transfer-encoding", "upgrade", "x-amzn-trace-id"];

/// Signs the request as of `now` (seconds since 1970): adds `X-Amz-Date`, the session token,
/// the body's hash for S3 and `Authorization`.
pub fn sign(p: &mut Prepared, aws: &AwsV4, now: i64) -> Result<(), String> {
    let missing: Vec<&str> =
        [
            (tr_static!("ключ доступа", "access key"), &aws.access_key),
            (tr_static!("секретный ключ", "secret key"), &aws.secret_key),
            (tr_static!("регион", "region"), &aws.region),
            (tr_static!("сервис", "service"), &aws.service),
        ]
            .iter()
            .filter(|(_, v)| v.trim().is_empty())
            .map(|(name, _)| *name)
            .collect();
    if !missing.is_empty() {
        return Err(tr!("AWS Signature v4: не заданы {}", "AWS Signature v4: not set: {}", missing.join(", ")));
    }
    let (region, service) = (aws.region.trim(), aws.service.trim());
    let s3 = service == "s3";

    let amz_date = match p.header("x-amz-date") {
        Some(d) => d.trim().to_string(),
        None => {
            let d = amz_date(now);
            p.headers.push(("X-Amz-Date".into(), d.clone()));
            d
        }
    };
    if amz_date.len() < 8 || !amz_date[..8].bytes().all(|b| b.is_ascii_digit()) {
        return Err(tr!("X-Amz-Date «{amz_date}»: нужно ГГГГММДДTЧЧММССZ", "X-Amz-Date \"{amz_date}\": must be YYYYMMDDTHHMMSSZ"));
    }
    if !aws.session_token.trim().is_empty() && p.header("x-amz-security-token").is_none() {
        p.headers.push(("X-Amz-Security-Token".into(), aws.session_token.trim().to_string()));
    }
    let payload = match p.header("x-amz-content-sha256") {
        Some(given) => given.trim().to_string(),
        None => {
            let hash = hex(digest::digest(&digest::SHA256, &p.body).as_ref());
            if s3 {
                p.headers.push(("X-Amz-Content-Sha256".into(), hash.clone()));
            }
            hash
        }
    };

    // What goes on the wire is what is signed.
    let path = p.url.path().split('/').map(|segment| encode(&percent_decode(segment))).collect::<Vec<_>>().join("/");
    p.url.set_path(&path);
    let mut query: Vec<(String, String)> = p.url.query_pairs().map(|(k, v)| (encode(k.as_bytes()), encode(v.as_bytes()))).collect();
    query.sort();
    let query = query.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
    p.url.set_query((!query.is_empty()).then_some(query.as_str()));
    // S3 signs the path as it goes; the other services encode it once more.
    let uri = match p.url.path() {
        "" => "/".to_string(),
        path if s3 => path.to_string(),
        path => path.split('/').map(|s| encode(s.as_bytes())).collect::<Vec<_>>().join("/"),
    };

    let host = match p.header("host") {
        Some(h) => h.to_string(),
        None => {
            let name = p.url.host_str().unwrap_or_default();
            match p.url.port() {
                Some(port) => format!("{name}:{port}"),
                None => name.to_string(),
            }
        }
    };
    let mut headers: Vec<(String, String)> = vec![("host".into(), host)];
    for (name, value) in &p.headers {
        let name = name.to_ascii_lowercase();
        if name == "host" || UNSIGNED.contains(&name.as_str()) {
            continue;
        }
        let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
        match headers.iter_mut().find(|(n, _)| *n == name) {
            Some((_, joined)) => {
                joined.push(',');
                joined.push_str(&value);
            }
            None => headers.push((name, value)),
        }
    }
    headers.sort();
    let signed = headers.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(";");
    let canonical_headers: String = headers.iter().map(|(n, v)| format!("{n}:{v}\n")).collect();

    let canonical = format!("{}\n{uri}\n{}\n{canonical_headers}\n{signed}\n{payload}", p.method, p.url.query().unwrap_or_default());
    let date = &amz_date[..8];
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", hex(digest::digest(&digest::SHA256, canonical.as_bytes()).as_ref()));
    let mut key = format!("AWS4{}", aws.secret_key.trim()).into_bytes();
    for part in [date, region, service, "aws4_request"] {
        key = mac(&key, part.as_bytes());
    }
    let signature = hex(&mac(&key, to_sign.as_bytes()));
    p.headers.retain(|(n, _)| !n.eq_ignore_ascii_case("authorization"));
    p.headers.push((
        "Authorization".into(),
        format!("AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={signature}", aws.access_key.trim()),
    ));
    Ok(())
}

fn mac(key: &[u8], data: &[u8]) -> Vec<u8> {
    hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), data).as_ref().to_vec()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Everything but `A-Z a-z 0-9 - _ . ~` as `%XX`, as AWS signs it.
fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `%XX` back to bytes; in a path `+` is itself.
fn percent_decode(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (bytes[i], bytes.get(i + 1).and_then(|&b| hex(b)), bytes.get(i + 2).and_then(|&b| hex(b))) {
            (b'%', Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

/// `20150830T123600Z`.
fn amz_date(now: i64) -> String {
    let (days, secs) = (now.div_euclid(86_400), now.rem_euclid(86_400));
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z", secs / 3600, secs % 3600 / 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RequestOptions;

    // The credentials of AWS's own examples and test suite.
    fn example(region: &str, service: &str) -> AwsV4 {
        AwsV4 {
            access_key: "AKIDEXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            session_token: String::new(),
            region: region.into(),
            service: service.into(),
        }
    }

    fn prepared(method: &str, url: &str, headers: &[(&str, &str)], body: &str) -> Prepared {
        Prepared {
            method: method.into(),
            url: url.parse().unwrap(),
            headers: headers.iter().map(|(n, v)| (n.to_string(), v.to_string())).collect(),
            body: bytes::Bytes::from(body.to_string()),
            options: RequestOptions::default(),
            oauth: None,
            digest: None,
            aws: None,
        }
    }

    fn signature(p: &Prepared) -> &str {
        p.header("authorization").unwrap().rsplit("Signature=").next().unwrap()
    }

    #[test]
    fn dates() {
        assert_eq!(amz_date(1_440_938_160), "20150830T123600Z");
        assert_eq!(amz_date(0), "19700101T000000Z");
        assert_eq!(amz_date(951_782_400), "20000229T000000Z");
    }

    #[test]
    fn aws_test_suite() {
        // get-vanilla.
        let mut p = prepared("GET", "https://example.amazonaws.com/", &[], "");
        sign(&mut p, &example("us-east-1", "service"), 1_440_938_160).unwrap();
        assert_eq!(p.header("x-amz-date"), Some("20150830T123600Z"));
        assert_eq!(
            p.header("authorization").unwrap(),
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date, Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
        // The IAM example of the documentation: a header of the user's own, signed too.
        let mut p = prepared(
            "GET",
            "https://iam.amazonaws.com/?Action=ListUsers&Version=2010-05-08",
            &[("Content-Type", "application/x-www-form-urlencoded; charset=utf-8"), ("X-Amz-Date", "20150830T123600Z")],
            "",
        );
        sign(&mut p, &example("us-east-1", "iam"), 0).unwrap();
        assert_eq!(signature(&p), "5d672d79c15b13162d9279b0855cfba6789a8edb4c82c400e06b5924a6f2b5d7");
    }

    #[test]
    fn wire_matches_the_signature() {
        let mut p = prepared("GET", "https://h.test/a%20b/c'd?b=2&a=x+y&a=1&c", &[], "");
        sign(&mut p, &example("eu-west-1", "execute-api"), 0).unwrap();
        assert_eq!(p.url.as_str(), "https://h.test/a%20b/c%27d?a=1&a=x%20y&b=2&c=");
        let mut s3 = prepared("PUT", "https://b.s3.eu-west-1.amazonaws.com/k", &[("Content-Type", "text/plain")], "кот");
        sign(&mut s3, &AwsV4 { session_token: "tok".into(), ..example("eu-west-1", "s3") }, 0).unwrap();
        assert_eq!(s3.header("x-amz-content-sha256").map(str::len), Some(64));
        assert_eq!(s3.header("x-amz-security-token"), Some("tok"));
        assert!(
            s3.header("authorization").unwrap().contains("SignedHeaders=content-type;host;x-amz-content-sha256;x-amz-date;x-amz-security-token,"),
            "{:?}",
            s3.headers
        );
        // Signing again (a retry) replaces the signature instead of adding another.
        sign(&mut s3, &example("eu-west-1", "s3"), 0).unwrap();
        assert_eq!(s3.headers.iter().filter(|(n, _)| n == "Authorization").count(), 1);
        let err = sign(&mut prepared("GET", "https://h.test/", &[], ""), &example("", ""), 0).unwrap_err();
        assert_eq!(err, "AWS Signature v4: не заданы регион, сервис");
        let err = crate::lang::in_language(true, || sign(&mut prepared("GET", "https://h.test/", &[], ""), &example("", ""), 0).unwrap_err());
        assert_eq!(err, "AWS Signature v4: not set: region, service");
    }
}
