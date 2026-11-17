//! What the user gave: PEM, base64 or DER (Codegen and Decoder Design, Decoder: Input).

use serde::Serialize;

/// How the input is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    Pem,
    Base64,
    Der,
}

/// One encoding to decode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Input {
    pub encoding: Encoding,
    /// The PEM label (`CERTIFICATE`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip)]
    pub bytes: Vec<u8>,
}

fn base64_value(c: u8) -> Option<u8> {
    match c {
        b'A'..=b'Z' => Some(c.wrapping_sub(b'A')),
        b'a'..=b'z' => Some(c.wrapping_sub(b'a').wrapping_add(26)),
        b'0'..=b'9' => Some(c.wrapping_sub(b'0').wrapping_add(52)),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    }
}

/// Decodes base64 (the standard and the URL-safe alphabets), ignoring whitespace; `None` on any other character.
pub fn base64(text: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len().saturating_mul(3) / 4);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut padding = false;
    for &c in text {
        if c.is_ascii_whitespace() {
            continue;
        }
        if c == b'=' {
            padding = true;
            continue;
        }
        if padding {
            return None;
        }
        acc = (acc << 6) | u32::from(base64_value(c)?);
        bits = bits.saturating_add(6);
        if bits >= 8 {
            bits = bits.saturating_sub(8);
            out.push(u8::try_from((acc >> bits) & 0xff).unwrap_or(0));
        }
    }
    Some(out)
}

/// Encodes `bytes` as base64 (the standard alphabet, padded), on one line.
pub fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let symbol =
        |index: u32| char::from(ALPHABET.get((index & 63) as usize).copied().unwrap_or(b'A'));
    let mut out = String::with_capacity(bytes.len().div_ceil(3).saturating_mul(4));
    for chunk in bytes.chunks(3) {
        let mut word = 0u32;
        for (shift, byte) in [16u32, 8, 0].into_iter().zip(chunk) {
            word |= u32::from(*byte) << shift;
        }
        out.push(symbol(word >> 18));
        out.push(symbol(word >> 12));
        out.push(if chunk.len() > 1 {
            symbol(word >> 6)
        } else {
            '='
        });
        out.push(if chunk.len() > 2 { symbol(word) } else { '=' });
    }
    out
}

/// `bytes` in base64, in lines of 64 characters, each ended by a newline.
pub fn base64_lines(bytes: &[u8]) -> String {
    let encoded = base64_encode(bytes);
    let mut out = String::new();
    let mut rest = encoded.as_str();
    while !rest.is_empty() {
        let (line, tail) = rest.split_at(rest.len().min(64));
        out.push_str(line);
        out.push('\n');
        rest = tail;
    }
    out
}

/// `bytes` as a PEM block with `label` (RFC 7468): base64 in lines of 64 characters.
pub fn pem_armor(label: &str, bytes: &[u8]) -> String {
    format!(
        "-----BEGIN {label}-----\n{}-----END {label}-----\n",
        base64_lines(bytes)
    )
}

/// The label a PEM block is served with: the canonical form of a label the label map knows (`X509 CERTIFICATE` →
/// `CERTIFICATE`, RFC 7468 Section 5.3), or `None` for a label it does not know.
pub fn canonical_label(label: &str) -> Option<&'static str> {
    Some(match label {
        "CERTIFICATE" | "X509 CERTIFICATE" => "CERTIFICATE",
        "X509 CRL" | "CRL" => "X509 CRL",
        "CERTIFICATE REQUEST" | "NEW CERTIFICATE REQUEST" => "CERTIFICATE REQUEST",
        "ATTRIBUTE CERTIFICATE" => "ATTRIBUTE CERTIFICATE",
        "PKCS7" | "CMS" => "CMS",
        "PRIVATE KEY" => "PRIVATE KEY",
        "ENCRYPTED PRIVATE KEY" => "ENCRYPTED PRIVATE KEY",
        "PUBLIC KEY" => "PUBLIC KEY",
        "RSA PRIVATE KEY" => "RSA PRIVATE KEY",
        "RSA PUBLIC KEY" => "RSA PUBLIC KEY",
        "EC PRIVATE KEY" => "EC PRIVATE KEY",
        "OCSP REQUEST" => "OCSP REQUEST",
        "OCSP RESPONSE" => "OCSP RESPONSE",
        "PKCS12" => "PKCS12",
        _ => return None,
    })
}

/// The base64 of a PEM block's body, read as RFC 7468's lax grammar (Section 3, Figure 2): whitespace and the
/// standard alphabet (RFC 4648 Section 4), with at most two `=` at the end. Everything else is refused rather than
/// skipped, so the octets decoded are exactly the octets written: a header line (Section 2: textual encoding does not
/// permit headers), a character of another alphabet, characters that do not make a whole octet, and pad bits that are
/// not zero (RFC 4648 Section 3.5).
fn pem_base64(body: &str) -> Result<Vec<u8>, String> {
    let is_space = |c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c');
    if body.lines().any(|line| line.contains(':')) {
        return Err("a header line; RFC 7468 permits none".to_owned());
    }
    let mut symbols = 0usize;
    let mut padding = 0usize;
    for c in body.chars() {
        if is_space(c) {
            continue;
        }
        if c == '=' {
            padding = padding.saturating_add(1);
            if padding > 2 {
                return Err("more than two `=`".to_owned());
            }
            continue;
        }
        if padding > 0 {
            return Err(format!("`{c}` after `=`"));
        }
        if !(c.is_ascii_alphanumeric() || c == '+' || c == '/') {
            return Err(format!(
                "`{}` is not a base64 character",
                c.escape_default()
            ));
        }
        symbols = symbols.saturating_add(1);
    }
    let complete = match symbols % 4 {
        0 => padding == 0,
        2 => padding == 0 || padding == 2,
        3 => padding <= 1,
        _ => false,
    };
    if !complete {
        return Err("the base64 does not end on a whole octet".to_owned());
    }
    let compact: String = body.chars().filter(|&c| !is_space(c) && c != '=').collect();
    let last = compact.bytes().last().and_then(base64_value).unwrap_or(0);
    let pad_bits = match symbols % 4 {
        2 => last & 0x0f,
        3 => last & 0x03,
        _ => 0,
    };
    if pad_bits != 0 {
        return Err("the pad bits of the last base64 character are not zero".to_owned());
    }
    base64(compact.as_bytes()).ok_or_else(|| "the input is not base64".to_owned())
}

/// The PEM blocks of `text` (RFC 7468): label, and the decoded octets or what is wrong with the block. Text before,
/// between and after the blocks is ignored, as Section 2 asks of parsers; inside a block, the body is read strictly
/// (`pem_base64`), and the post-encapsulation boundary must name the label of the pre-encapsulation one.
pub fn pem_blocks(text: &str) -> Vec<(String, Result<Vec<u8>, String>)> {
    let mut out = Vec::new();
    // RFC 7468 Section 2: lines end with CRLF, CR or LF
    let mut lines = text.split(['\r', '\n']);
    while let Some(line) = lines.next() {
        let Some(label) = line
            .trim()
            .strip_prefix("-----BEGIN ")
            .and_then(|rest| rest.strip_suffix("-----"))
        else {
            continue;
        };
        let mut body = String::new();
        let mut result = Err(format!("no `-----END {label}-----` line"));
        for line in lines.by_ref() {
            let line = line.trim();
            if let Some(end) = line
                .strip_prefix("-----END ")
                .and_then(|rest| rest.strip_suffix("-----"))
            {
                result = if end == label {
                    pem_base64(&body)
                } else {
                    Err(format!("its END line names `{end}`"))
                };
                break;
            }
            if line.starts_with("-----BEGIN ") {
                break;
            }
            body.push_str(line);
            body.push('\n');
        }
        out.push((label.to_owned(), result));
    }
    out
}

/// The blocks of `text` as inputs, or the first block that cannot be read.
fn pem_inputs(text: &str) -> Result<Vec<Input>, String> {
    let mut inputs = Vec::new();
    for (index, (label, bytes)) in pem_blocks(text).into_iter().enumerate() {
        let bytes = bytes.map_err(|problem| {
            format!("PEM block {} ({label}): {problem}", index.saturating_add(1))
        })?;
        inputs.push(Input {
            encoding: Encoding::Pem,
            label: Some(label),
            bytes,
        });
    }
    if inputs.is_empty() {
        return Err("the input has no PEM block".to_owned());
    }
    Ok(inputs)
}

/// Reads `bytes` as PEM, then base64, then DER, and returns every encoding found in it. Text with a
/// pre-encapsulation boundary is PEM, and a block that cannot be read is an error.
pub fn sniff(bytes: &[u8]) -> Result<Vec<Input>, String> {
    if let Ok(text) = std::str::from_utf8(bytes)
        && text.contains("-----BEGIN ")
    {
        return pem_inputs(text);
    }
    let looks_like_text = !bytes.is_empty()
        && bytes
            .iter()
            .all(|b| b.is_ascii_whitespace() || base64_value(*b).is_some() || *b == b'=');
    if looks_like_text && let Some(decoded) = base64(bytes) {
        return Ok(vec![Input {
            encoding: Encoding::Base64,
            label: None,
            bytes: decoded,
        }]);
    }
    Ok(vec![Input {
        encoding: Encoding::Der,
        label: None,
        bytes: bytes.to_vec(),
    }])
}

/// Reads `bytes` as `encoding`, without sniffing.
pub fn read_as(bytes: &[u8], encoding: Encoding) -> Result<Vec<Input>, String> {
    match encoding {
        Encoding::Der => Ok(vec![Input {
            encoding,
            label: None,
            bytes: bytes.to_vec(),
        }]),
        Encoding::Base64 => base64(bytes)
            .map(|decoded| {
                vec![Input {
                    encoding,
                    label: None,
                    bytes: decoded,
                }]
            })
            .ok_or_else(|| "the input is not base64".to_owned()),
        Encoding::Pem => {
            pem_inputs(std::str::from_utf8(bytes).map_err(|_| "the input is not text")?)
        }
    }
}

/// The corpus root types a PEM label suggests, in preference order (`DOC#Module.Type`): the labels of RFC 7468 and
/// the common de-facto ones.
pub fn label_roots(label: &str) -> &'static [&'static str] {
    match label {
        "CERTIFICATE" | "X509 CERTIFICATE" => &[
            "rfc5912#PKIX1Explicit-2009.Certificate",
            "rfc5280#PKIX1Explicit88.Certificate",
        ],
        "X509 CRL" | "CRL" => &[
            "rfc5912#PKIX1Explicit-2009.CertificateList",
            "rfc5280#PKIX1Explicit88.CertificateList",
        ],
        "CERTIFICATE REQUEST" | "NEW CERTIFICATE REQUEST" => &[
            "rfc5912#PKCS-10.CertificationRequest",
            "rfc2986#PKCS-10.CertificationRequest",
        ],
        "ATTRIBUTE CERTIFICATE" => &[
            "rfc5912#PKIXAttributeCertificate-2009.AttributeCertificate",
            "rfc5755#PKIXAttributeCertificate.AttributeCertificate",
        ],
        "PKCS7" | "CMS" => &[
            "rfc6268#CryptographicMessageSyntax-2010.ContentInfo",
            "rfc5652#CryptographicMessageSyntax2004.ContentInfo",
        ],
        "PRIVATE KEY" => &["rfc5958#AsymmetricKeyPackageModuleV1.OneAsymmetricKey"],
        "ENCRYPTED PRIVATE KEY" => {
            &["rfc5958#AsymmetricKeyPackageModuleV1.EncryptedPrivateKeyInfo"]
        }
        "PUBLIC KEY" => &[
            "rfc5912#PKIX1Explicit-2009.SubjectPublicKeyInfo",
            "rfc5280#PKIX1Explicit88.SubjectPublicKeyInfo",
        ],
        "RSA PRIVATE KEY" => &["rfc8017#PKCS-1.RSAPrivateKey"],
        "RSA PUBLIC KEY" => &["rfc8017#PKCS-1.RSAPublicKey"],
        "EC PRIVATE KEY" => &["rfc5915#ECPrivateKey.ECPrivateKey"],
        "OCSP REQUEST" => &["rfc6960#OCSP-2013-08.OCSPRequest"],
        "OCSP RESPONSE" => &["rfc6960#OCSP-2013-08.OCSPResponse"],
        "PKCS12" => &["rfc7292#PKCS-12.PFX"],
        _ => &[],
    }
}

/// The corpus roots tried for an example whose label suggests none that fits (Codegen and Decoder Design, Example
/// artifacts: Identification), most common first.
pub fn common_roots() -> &'static [&'static str] {
    &[
        "rfc5912#PKIX1Explicit-2009.Certificate",
        "rfc5912#PKIX1Explicit-2009.CertificateList",
        "rfc5912#PKCS-10.CertificationRequest",
        "rfc6268#CryptographicMessageSyntax-2010.ContentInfo",
        "rfc5958#AsymmetricKeyPackageModuleV1.OneAsymmetricKey",
        "rfc5958#AsymmetricKeyPackageModuleV1.EncryptedPrivateKeyInfo",
        "rfc5912#PKIX1Explicit-2009.SubjectPublicKeyInfo",
        "rfc6960#OCSP-2013-08.OCSPResponse",
        "rfc6960#OCSP-2013-08.OCSPRequest",
        "rfc3161#PKIXTSP.TimeStampResp",
        "rfc3161#PKIXTSP.TimeStampReq",
        "rfc5912#PKIXAttributeCertificate-2009.AttributeCertificate",
        "rfc9810#PKIXCMP-2023.PKIMessage",
        "rfc7292#PKCS-12.PFX",
        "rfc5915#ECPrivateKey.ECPrivateKey",
        "rfc8017#PKCS-1.RSAPrivateKey",
        "rfc8017#PKCS-1.RSAPublicKey",
    ]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn base64_with_whitespace_and_padding() {
        assert_eq!(base64(b"TWFu").unwrap(), b"Man");
        assert_eq!(base64(b"TW\nE=").unwrap(), b"Ma");
        assert!(base64(b"TW=E").is_none());
    }

    #[test]
    fn base64_round_trips() {
        for bytes in [
            &b""[..],
            b"M",
            b"Ma",
            b"Man",
            b"Many bytes, more than one line of PEM: 0123456789abcdef",
        ] {
            assert_eq!(base64(base64_encode(bytes).as_bytes()).unwrap(), bytes);
        }
        let pem = pem_armor("FOO", &[0x30; 50]);
        assert_eq!(pem.lines().nth(1).unwrap().len(), 64);
        assert_eq!(sniff(pem.as_bytes()).unwrap()[0].bytes, [0x30; 50]);
    }

    #[test]
    fn pem_then_base64_then_der() {
        let pem = "intro\n-----BEGIN FOO-----\nMAA=\n-----END FOO-----\ntext\n-----BEGIN BAR-----\nBQA=\n-----END BAR-----\n";
        let inputs = sniff(pem.as_bytes()).unwrap();
        assert_eq!(inputs.len(), 2);
        assert_eq!(inputs[0].label.as_deref(), Some("FOO"));
        assert_eq!(inputs[0].bytes, [0x30, 0x00]);
        assert_eq!(sniff(b"MAA=").unwrap()[0].encoding, Encoding::Base64);
        assert_eq!(sniff(&[0x30, 0x00]).unwrap()[0].encoding, Encoding::Der);
    }

    fn block(body: &str) -> Result<Vec<u8>, String> {
        let text = format!("-----BEGIN FOO-----\n{body}\n-----END FOO-----\n");
        pem_blocks(&text).remove(0).1
    }

    #[test]
    fn pem_bodies_in_the_lax_grammar_are_read() {
        assert_eq!(block("MAA=").unwrap(), [0x30, 0x00]);
        assert_eq!(block("MAA").unwrap(), [0x30, 0x00]);
        assert_eq!(block(" MA\tA= ").unwrap(), [0x30, 0x00]);
        assert_eq!(block("").unwrap(), b"");
        let crlf = "-----BEGIN FOO-----\r\nMAA=\r\n-----END FOO-----\r\n";
        assert_eq!(pem_blocks(crlf).remove(0).1.unwrap(), [0x30, 0x00]);
        let cr = "-----BEGIN FOO-----\rMAA=\r-----END FOO-----\r";
        assert_eq!(pem_blocks(cr).remove(0).1.unwrap(), [0x30, 0x00]);
    }

    // RFC 7468 Section 2 (no headers, the standard alphabet) and Figure 2; RFC 4648 Section 3.5 (pad bits).
    #[test]
    fn pem_bodies_outside_the_grammar_are_refused() {
        for (body, problem) in [
            ("Proc-Type: 4,ENCRYPTED\nMAA=", "header"),
            ("MA_A", "not a base64 character"),
            ("MA-A", "not a base64 character"),
            ("MA!A", "not a base64 character"),
            ("MA=A", "after `=`"),
            ("MA===", "more than two"),
            ("MAA=A", "after `=`"),
            ("MAAAB", "whole octet"),
            ("MA=", "whole octet"),
            ("MAB=", "pad bits"),
            ("MB==", "pad bits"),
        ] {
            let error = block(body).unwrap_err();
            assert!(error.contains(problem), "{body:?}: {error}");
        }
    }

    #[test]
    fn pem_boundaries_must_close_and_match() {
        let unclosed = "-----BEGIN FOO-----\nMAA=\n";
        assert!(
            pem_blocks(unclosed)
                .remove(0)
                .1
                .unwrap_err()
                .contains("no `-----END FOO-----`")
        );
        let other = "-----BEGIN FOO-----\nMAA=\n-----END BAR-----\n";
        assert!(
            pem_blocks(other)
                .remove(0)
                .1
                .unwrap_err()
                .contains("names `BAR`")
        );
        let nested = "-----BEGIN FOO-----\nMAA=\n-----BEGIN BAR-----\nMAA=\n-----END BAR-----\n";
        assert!(pem_blocks(nested).remove(0).1.is_err());
    }

    #[test]
    fn one_bad_block_fails_the_whole_input() {
        let text = "-----BEGIN FOO-----\nMAA=\n-----END FOO-----\n-----BEGIN FOO-----\nM!A=\n-----END FOO-----\n";
        let error = read_as(text.as_bytes(), Encoding::Pem).unwrap_err();
        assert!(error.starts_with("PEM block 2 (FOO)"), "{error}");
        assert!(sniff(text.as_bytes()).is_err());
        let text_around = "intro\n-----BEGIN FOO-----\nMAA=\n-----END FOO-----\ntrailing text\n";
        assert_eq!(
            read_as(text_around.as_bytes(), Encoding::Pem)
                .unwrap()
                .len(),
            1
        );
    }
}
