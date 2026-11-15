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

/// `bytes` as a PEM block with `label` (RFC 7468): base64 in lines of 64 characters.
pub fn pem_armor(label: &str, bytes: &[u8]) -> String {
    let encoded = base64_encode(bytes);
    let mut out = format!("-----BEGIN {label}-----\n");
    let mut rest = encoded.as_str();
    while !rest.is_empty() {
        let (line, tail) = rest.split_at(rest.len().min(64));
        out.push_str(line);
        out.push('\n');
        rest = tail;
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
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

/// The PEM blocks of `text` (RFC 7468): label and decoded bytes. Explanatory text around them is ignored, and so are
/// RFC 1421 headers (`Proc-Type: ...`) inside a block.
pub fn pem_blocks(text: &str) -> Vec<(String, Option<Vec<u8>>)> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let line = line.trim();
        let Some(label) = line
            .strip_prefix("-----BEGIN ")
            .and_then(|rest| rest.strip_suffix("-----"))
        else {
            continue;
        };
        let end = format!("-----END {label}-----");
        let mut body = String::new();
        let mut closed = false;
        for line in lines.by_ref() {
            let line = line.trim();
            if line == end {
                closed = true;
                break;
            }
            if line.contains(':') {
                continue;
            }
            body.push_str(line);
        }
        let bytes = if closed {
            base64(body.as_bytes())
        } else {
            None
        };
        out.push((label.to_owned(), bytes));
    }
    out
}

/// Reads `bytes` as PEM, then base64, then DER, and returns every encoding found in it.
pub fn sniff(bytes: &[u8]) -> Vec<Input> {
    if let Ok(text) = std::str::from_utf8(bytes)
        && text.contains("-----BEGIN ")
    {
        let blocks: Vec<Input> = pem_blocks(text)
            .into_iter()
            .filter_map(|(label, bytes)| {
                Some(Input {
                    encoding: Encoding::Pem,
                    label: Some(label),
                    bytes: bytes?,
                })
            })
            .collect();
        if !blocks.is_empty() {
            return blocks;
        }
    }
    let looks_like_text = !bytes.is_empty()
        && bytes
            .iter()
            .all(|b| b.is_ascii_whitespace() || base64_value(*b).is_some() || *b == b'=');
    if looks_like_text && let Some(decoded) = base64(bytes) {
        return vec![Input {
            encoding: Encoding::Base64,
            label: None,
            bytes: decoded,
        }];
    }
    vec![Input {
        encoding: Encoding::Der,
        label: None,
        bytes: bytes.to_vec(),
    }]
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
            let text = std::str::from_utf8(bytes).map_err(|_| "the input is not text")?;
            let blocks: Vec<Input> = pem_blocks(text)
                .into_iter()
                .filter_map(|(label, bytes)| {
                    Some(Input {
                        encoding,
                        label: Some(label),
                        bytes: bytes?,
                    })
                })
                .collect();
            if blocks.is_empty() {
                Err("the input has no PEM block".to_owned())
            } else {
                Ok(blocks)
            }
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
        assert_eq!(sniff(pem.as_bytes())[0].bytes, [0x30; 50]);
    }

    #[test]
    fn pem_then_base64_then_der() {
        let pem = "intro\n-----BEGIN FOO-----\nMAA=\n-----END FOO-----\ntext\n-----BEGIN BAR-----\nBQA=\n-----END BAR-----\n";
        let inputs = sniff(pem.as_bytes());
        assert_eq!(inputs.len(), 2);
        assert_eq!(inputs[0].label.as_deref(), Some("FOO"));
        assert_eq!(inputs[0].bytes, [0x30, 0x00]);
        assert_eq!(sniff(b"MAA=")[0].encoding, Encoding::Base64);
        assert_eq!(sniff(&[0x30, 0x00])[0].encoding, Encoding::Der);
    }
}
