//! Certificate/key file type plugin: core and presentation halves.
//!
//! Covers PEM-encoded `.pem`/`.crt`/`.cer`/`.key` files: X.509 certificates,
//! certificate signing requests, and private/public keys. `sniff` only
//! recognises PEM's own `-----BEGIN <label>-----` text marker; a bare DER
//! file has no comparably unambiguous signal, since its leading `SEQUENCE`
//! tag byte overlaps too much other binary content to serve as one.

use pkcs8::PrivateKeyInfoRef;
use plugin_api::{Icon, PluginCore, PluginPresentation, Span};
use serde::{Deserialize, Serialize};
use spki::SubjectPublicKeyInfoRef;
use std::io;
use std::path::Path;
use syntax::Language;
use x509_parser::prelude::{FromDer, X509Certificate, X509CertificationRequest};

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
pub const EXTENSIONS: &[&str] = &["pem", "crt", "cer", "der"];

/// One parsed PEM block from a certificate/key file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum PemEntry {
    /// An X.509 certificate.
    Certificate {
        /// The certificate subject's distinguished name.
        subject: String,
        /// The issuing certificate authority's distinguished name.
        issuer: String,
        /// The certificate's serial number, as colon-separated hex.
        serial: String,
        /// Start of the certificate's validity period.
        not_before: String,
        /// End of the certificate's validity period.
        not_after: String,
    },
    /// A certificate signing request.
    CertificateRequest {
        /// The requested subject's distinguished name.
        subject: String,
    },
    /// A private key.
    PrivateKey {
        /// The key's algorithm, e.g. `"RSA"`, `"EC (P-256)"`.
        algorithm: String,
    },
    /// A public key.
    PublicKey {
        /// The key's algorithm, e.g. `"RSA"`, `"EC (P-256)"`.
        algorithm: String,
    },
    /// A PEM block whose label this plugin doesn't parse further.
    Unrecognized {
        /// The PEM block's `-----BEGIN <label>-----` label.
        label: String,
    },
}

/// View data produced by [`CertificateCore::view`]: one entry per PEM block
/// in the file, in file order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateKeyView {
    /// The file's PEM blocks, in order.
    pub entries: Vec<PemEntry>,
}

/// Maps a SEC1/PKCS8 EC named-curve OID to its common name.
fn curve_name(oid: &der::oid::ObjectIdentifier) -> Option<&'static str> {
    match oid.to_string().as_str() {
        "1.2.840.10045.3.1.7" => Some("P-256"),
        "1.3.132.0.34" => Some("P-384"),
        "1.3.132.0.35" => Some("P-521"),
        "1.3.132.0.10" => Some("secp256k1"),
        _ => None,
    }
}

/// Maps a key algorithm OID (and its optional parameters) to a
/// human-readable name.
fn algorithm_name(
    oid: &der::oid::ObjectIdentifier,
    parameters: Option<der::asn1::AnyRef<'_>>,
) -> String {
    match oid.to_string().as_str() {
        "1.2.840.113549.1.1.1" => "RSA".to_owned(),
        "1.2.840.10045.2.1" => {
            let curve = parameters
                .and_then(|params| params.decode_as::<der::oid::ObjectIdentifier>().ok())
                .and_then(|curve_oid| curve_name(&curve_oid));
            curve.map_or_else(|| "EC".to_owned(), |name| format!("EC ({name})"))
        }
        "1.3.101.112" => "Ed25519".to_owned(),
        "1.3.101.113" => "Ed448".to_owned(),
        "1.3.101.110" => "X25519".to_owned(),
        "1.3.101.111" => "X448".to_owned(),
        "1.2.840.10040.4.1" => "DSA".to_owned(),
        other => format!("unknown algorithm ({other})"),
    }
}

/// Reads a certificate's fields into a [`PemEntry::Certificate`], falling
/// back to [`PemEntry::Unrecognized`] if `der` doesn't decode as an X.509
/// certificate.
fn certificate_entry(der: &[u8]) -> PemEntry {
    X509Certificate::from_der(der).map_or_else(
        |_| PemEntry::Unrecognized {
            label: "CERTIFICATE".to_owned(),
        },
        |(_, cert)| PemEntry::Certificate {
            subject: cert.subject().to_string(),
            issuer: cert.issuer().to_string(),
            serial: cert.raw_serial_as_string(),
            not_before: cert.validity().not_before.to_string(),
            not_after: cert.validity().not_after.to_string(),
        },
    )
}

/// Reads a certificate signing request's subject into a
/// [`PemEntry::CertificateRequest`], falling back to
/// [`PemEntry::Unrecognized`] if `der` doesn't decode as a PKCS#10 request.
fn certificate_request_entry(der: &[u8]) -> PemEntry {
    X509CertificationRequest::from_der(der).map_or_else(
        |_| PemEntry::Unrecognized {
            label: "CERTIFICATE REQUEST".to_owned(),
        },
        |(_, csr)| PemEntry::CertificateRequest {
            subject: csr.certification_request_info.subject.to_string(),
        },
    )
}

/// Reads a PKCS#8 `PrivateKeyInfo`'s algorithm into a
/// [`PemEntry::PrivateKey`], falling back to [`PemEntry::Unrecognized`] if
/// `der` doesn't decode.
fn pkcs8_private_key_entry(der: &[u8]) -> PemEntry {
    PrivateKeyInfoRef::try_from(der).map_or_else(
        |_| PemEntry::Unrecognized {
            label: "PRIVATE KEY".to_owned(),
        },
        |info| PemEntry::PrivateKey {
            algorithm: algorithm_name(&info.algorithm.oid, info.algorithm.parameters),
        },
    )
}

/// Reads an SPKI `SubjectPublicKeyInfo`'s algorithm into a
/// [`PemEntry::PublicKey`], falling back to [`PemEntry::Unrecognized`] if
/// `der` doesn't decode.
fn public_key_entry(der: &[u8]) -> PemEntry {
    SubjectPublicKeyInfoRef::try_from(der).map_or_else(
        |_| PemEntry::Unrecognized {
            label: "PUBLIC KEY".to_owned(),
        },
        |info| PemEntry::PublicKey {
            algorithm: algorithm_name(&info.algorithm.oid, info.algorithm.parameters),
        },
    )
}

/// Turns one decoded PEM block into a [`PemEntry`], dispatching on its
/// `-----BEGIN <label>-----` label. The three legacy, non-PKCS#8 private key
/// labels (`RSA`/`EC`/`DSA PRIVATE KEY`) carry their algorithm in the label
/// itself, so those are named directly rather than parsed.
fn entry_for(label: &str, der: &[u8]) -> PemEntry {
    match label {
        _ if label.contains("CERTIFICATE REQUEST") => certificate_request_entry(der),
        _ if label.contains("CERTIFICATE") => certificate_entry(der),
        "RSA PRIVATE KEY" => PemEntry::PrivateKey {
            algorithm: "RSA".to_owned(),
        },
        "EC PRIVATE KEY" => PemEntry::PrivateKey {
            algorithm: "EC".to_owned(),
        },
        "DSA PRIVATE KEY" => PemEntry::PrivateKey {
            algorithm: "DSA".to_owned(),
        },
        "PRIVATE KEY" => pkcs8_private_key_entry(der),
        "RSA PUBLIC KEY" => PemEntry::PublicKey {
            algorithm: "RSA".to_owned(),
        },
        "PUBLIC KEY" => public_key_entry(der),
        other => PemEntry::Unrecognized {
            label: other.to_owned(),
        },
    }
}

/// How many days before a certificate's `not_after` counts as "expiring",
/// for [`certificate_status`] (#621). The one place this is decided.
pub const EXPIRING_WITHIN_DAYS: i64 = 30;

/// Seconds in a day, for comparing [`EXPIRING_WITHIN_DAYS`] against a
/// difference of two Unix timestamps.
const SECONDS_PER_DAY: i64 = 24 * 60 * 60;

/// A certificate's standing as of `now`, decided in this one place (#621)
/// so a front end never re-derives the rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CertificateStatus {
    /// `not_after` has already passed.
    Expired,
    /// `not_after` is within [`EXPIRING_WITHIN_DAYS`] days from now.
    Expiring,
    /// `not_before` has not yet arrived.
    NotYetValid,
    /// Neither of the above.
    Valid,
}

/// Where a certificate valid from `not_before` to `not_after` (Unix
/// seconds) stands as of `now` (also Unix seconds).
///
/// A certificate whose validity period has not started yet is
/// [`CertificateStatus::NotYetValid`], checked before expiry: a sane
/// certificate never has `not_before` in the future and `not_after` in the
/// past at once, so the order only matters for malformed input, where
/// "not yet valid" is the more useful thing to say.
#[must_use]
pub fn certificate_status(not_before: i64, not_after: i64, now: i64) -> CertificateStatus {
    if now < not_before {
        CertificateStatus::NotYetValid
    } else if now >= not_after {
        CertificateStatus::Expired
    } else if not_after - now <= EXPIRING_WITHIN_DAYS * SECONDS_PER_DAY {
        CertificateStatus::Expiring
    } else {
        CertificateStatus::Valid
    }
}

/// One certificate's fields, as found by [`scan`] (#621) - distinct from
/// [`PemEntry::Certificate`]: validity as Unix seconds rather than a
/// formatted display string, ready for [`certificate_status`], and a
/// self-signed flag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateSummary {
    /// The certificate subject's distinguished name.
    pub subject: String,
    /// The issuing certificate authority's distinguished name.
    pub issuer: String,
    /// The certificate's serial number, as colon-separated hex.
    pub serial: String,
    /// Start of the certificate's validity period, Unix seconds.
    pub not_before: i64,
    /// End of the certificate's validity period, Unix seconds.
    pub not_after: i64,
    /// Whether the certificate's subject and issuer distinguished names are
    /// identical - a description read off the certificate, not a
    /// cryptographic signature check (D10, rule 8: detect, do not drive).
    pub self_signed: bool,
}

/// One PEM block found by [`scan`] (#621). A certificate carries full
/// detail; a private key or certificate signing request is only ever
/// counted, never detailed, so no key material reaches the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScannedBlock {
    /// A certificate that parsed.
    Certificate(CertificateSummary),
    /// A private key: present, and nothing more.
    PrivateKey,
    /// A certificate signing request: present, and nothing more.
    CertificateRequest,
    /// A `CERTIFICATE`-labelled block whose content did not decode as
    /// X.509, reported rather than dropped.
    Unreadable,
}

/// Reads a certificate's fields into a [`ScannedBlock::Certificate`], or
/// `None` if `der` doesn't decode as an X.509 certificate.
fn certificate_summary(der: &[u8]) -> Option<CertificateSummary> {
    let (_, cert) = X509Certificate::from_der(der).ok()?;
    let subject = cert.subject().to_string();
    let issuer = cert.issuer().to_string();
    Some(CertificateSummary {
        self_signed: subject == issuer,
        subject,
        issuer,
        serial: cert.raw_serial_as_string(),
        not_before: cert.validity().not_before.timestamp(),
        not_after: cert.validity().not_after.timestamp(),
    })
}

/// The [`ScannedBlock`] for one PEM block, or `None` for a block a
/// certificate scan has no use for - a public key, or a label it does not
/// recognise at all - which is not "unreadable": nothing tried and failed
/// to parse it.
fn scanned_block(label: &str, der: &[u8]) -> Option<ScannedBlock> {
    match label {
        _ if label.contains("CERTIFICATE REQUEST") => Some(ScannedBlock::CertificateRequest),
        _ if label.contains("CERTIFICATE") => Some(
            certificate_summary(der).map_or(ScannedBlock::Unreadable, ScannedBlock::Certificate),
        ),
        "RSA PRIVATE KEY" | "EC PRIVATE KEY" | "DSA PRIVATE KEY" | "PRIVATE KEY" => {
            Some(ScannedBlock::PrivateKey)
        }
        _ => None,
    }
}

/// Scans `bytes` - the (possibly truncated) content of a candidate
/// certificate/key file - into one [`ScannedBlock`] per certificate,
/// private key or certificate signing request found, for
/// `Request::FindCertificates` (#621).
///
/// Distinct from [`CertificateCore::view`], which serves the file's own
/// preview tab and keeps every block, including ones this scan has no use
/// for.
///
/// # Errors
/// Returns an error if `bytes` cannot be split into PEM blocks at all - not
/// PEM-encoded, rather than one block within it failing to decode as
/// X.509, which is instead a [`ScannedBlock::Unreadable`] entry.
pub fn scan(bytes: &[u8]) -> io::Result<Vec<ScannedBlock>> {
    let blocks =
        pem::parse_many(bytes).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    Ok(blocks
        .iter()
        .filter_map(|block| scanned_block(block.tag(), block.contents()))
        .collect())
}

/// The certificate/key plugin's core half.
#[derive(Debug, Default)]
pub struct CertificateCore;

/// How this language is coloured, for the shared tokeniser. GUIDANCE.md
/// §3.6: the plugin describes its own format, the pane paints what it is
/// told.
const CERTIFICATE: Language = Language {
    line_comment: &["%"],
    block_comment: &[],
    quotes: &[],
    keywords: &[
        "BEGIN",
        "CERTIFICATE",
        "END",
        "KEY",
        "PRIVATE",
        "PUBLIC",
        "REQUEST",
    ],
    types: &[],
    calls: false,
    ignore_case: false,
};

impl PluginCore for CertificateCore {
    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn name(&self) -> &'static str {
        "certificate"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let text = String::from_utf8_lossy(prefix);
        text.contains("-----BEGIN CERTIFICATE")
            || text.contains("PRIVATE KEY-----")
            || text.contains("PUBLIC KEY-----")
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let raw = std::fs::read(path)?;
        let blocks =
            pem::parse_many(&raw).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let entries = blocks
            .iter()
            .map(|block| entry_for(block.tag(), block.contents()))
            .collect();
        let view = CertificateKeyView { entries };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The certificate/key plugin's presentation half.
#[derive(Debug, Default)]
pub struct CertificatePresentation;

impl PluginPresentation for CertificatePresentation {
    fn classify(&self, text: &str) -> Vec<Span> {
        syntax::classify(text, &CERTIFICATE)
    }

    fn name(&self) -> &'static str {
        "certificate"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "CRT",
            tint: 0x0005_9669,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: CertificateKeyView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        if view.entries.is_empty() {
            return vec!["No PEM blocks found".to_owned()];
        }
        let mut lines = Vec::new();
        for (index, entry) in view.entries.iter().enumerate() {
            if index > 0 {
                lines.push(String::new());
            }
            match entry {
                PemEntry::Certificate {
                    subject,
                    issuer,
                    serial,
                    not_before,
                    not_after,
                } => {
                    lines.push("Certificate".to_owned());
                    lines.push(format!("Subject: {subject}"));
                    lines.push(format!("Issuer: {issuer}"));
                    lines.push(format!("Serial: {serial}"));
                    lines.push(format!("Valid from {not_before} to {not_after}"));
                }
                PemEntry::CertificateRequest { subject } => {
                    lines.push("Certificate signing request".to_owned());
                    lines.push(format!("Subject: {subject}"));
                }
                PemEntry::PrivateKey { algorithm } => {
                    lines.push(format!("{algorithm} private key"));
                }
                PemEntry::PublicKey { algorithm } => {
                    lines.push(format!("{algorithm} public key"));
                }
                PemEntry::Unrecognized { label } => {
                    lines.push(format!("Unrecognized PEM block: {label}"));
                }
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CertificateCore, CertificateKeyView, CertificatePresentation, CertificateStatus,
        EXPIRING_WITHIN_DAYS, PemEntry, ScannedBlock, certificate_status, scan,
    };
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-certificate-test-{}-{name}",
            std::process::id()
        ))
    }

    /// A real, `rcgen`-generated self-signed EC certificate, subject
    /// `CN=Test Root CA, O=RepoSphereExplorer Test`, valid 1975-01-01 to
    /// 4096-01-01.
    const CERT_PEM: &str = "-----BEGIN CERTIFICATE-----
MIIBkTCCATagAwIBAgIUf1zOrArsGiN2arJZNkQIT3HL6w4wCgYIKoZIzj0EAwIw
OTEVMBMGA1UEAwwMVGVzdCBSb290IENBMSAwHgYDVQQKDBdSZXBvU3BoZXJlRXhw
bG9yZXIgVGVzdDAgFw03NTAxMDEwMDAwMDBaGA80MDk2MDEwMTAwMDAwMFowOTEV
MBMGA1UEAwwMVGVzdCBSb290IENBMSAwHgYDVQQKDBdSZXBvU3BoZXJlRXhwbG9y
ZXIgVGVzdDBZMBMGByqGSM49AgEGCCqGSM49AwEHA0IABMMAKU4Arv7N+K5Xl/uo
GONeVXtrOhCcAUOf4StBpmlkgDo6hUfFTRj7IV5Txom86+qU5Jd6ADvPTzKeedWo
kuOjGjAYMBYGA1UdEQQPMA2CC2V4YW1wbGUuY29tMAoGCCqGSM49BAMCA0kAMEYC
IQCLgSlLPiOqHmY6oBKfdbCFqLHqgoZPgOGIdxzkiio+4AIhAJUvavI81fz1qqiW
Q8c1CP8QZQZVYgnOSYqC2s/Wyr6i
-----END CERTIFICATE-----
";

    /// A real, `rcgen`-generated PKCS#10 certificate signing request,
    /// subject `CN=Test CSR`.
    const CSR_PEM: &str = "-----BEGIN CERTIFICATE REQUEST-----
MIH2MIGeAgEAMBMxETAPBgNVBAMMCFRlc3QgQ1NSMFkwEwYHKoZIzj0CAQYIKoZI
zj0DAQcDQgAED5ApS6x06yoqOs9tdfPrQKmKIEjEBPxpIMcVVJEOrLgYgkRkSRnz
OU14BBTRRetJ9Za5+gcaoDjzPULqQvK5BaApMCcGCSqGSIb3DQEJDjEaMBgwFgYD
VR0RBA8wDYILZXhhbXBsZS5jb20wCgYIKoZIzj0EAwIDRwAwRAIgasIrLBfUnMZ9
ZTSqHT3d9XK6fxOrk2+VM0rkE/pTI0UCIEIeW89hTIQKTePmuLqhTXd3oxGWuqas
nkwqjy1SFrZN
-----END CERTIFICATE REQUEST-----
";

    /// A real, `rsa`-crate-generated 512-bit PKCS#8 RSA private key (small
    /// only so this fixture is short; never used for anything but a parse
    /// test).
    const PKCS8_RSA_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIIBVQIBADANBgkqhkiG9w0BAQEFAASCAT8wggE7AgEAAkEA3RNJt9hafRyQ7kep
vIo+NOPMCH06/hDiNlSx9U5B8qzmUpy8O6JDeUaL6Zmuc0MYs3jGKqjlRS4jUbJv
s28Y8QIDAQABAkACPyHuplo1D0dBxKSq79S2AOKf63XgAxfpaW7tiUAOUUKn3O/N
UZgxrOOCKZKNAARiqZTZqqq8L6TZt3eVcFgBAiEA8t19Xc77fVrRqnp3SHj/hWve
RwuHl1pU8eY16gEddUECIQDpCB0w+AfTd/x/ZX5gYEq6/pVNcM4fdp5eQRUB44xH
sQIhANm20HHN4QkI5zfKPTBMt9NlVYeewFhf9BI960rw4PWBAiEAyGEjyNHe2OZa
Bqoda34hhH4ZoEeZ1tBHCcFo8QDbxWECIDNwkmvUEJEkNDKc+kxZj3fVUXbUhbze
Mo8hvqlfr/IR
-----END PRIVATE KEY-----
";

    /// A real, `rsa`-crate-generated 512-bit PKCS#1 (legacy) RSA private
    /// key.
    const PKCS1_RSA_KEY_PEM: &str = "-----BEGIN RSA PRIVATE KEY-----
MIIBOwIBAAJBALH0mw+bfbccIPnR1/i+iDfuDsi/s5SJ4ye1p1j2fe8SzBWylqLy
DTi8Dn/pLXCg/AFPRy97B/yoQ7QvYJZDsXkCAwEAAQJAGOARYNAidZsn/OPZZbr0
faT4ShWJ+8R+jUl2OBhUqDtjseKCRcVB74decTCKoFe2VNe+oGTYKV9X84tpdp1W
AQIhAMvmwiRX5gds6TLzeYrqtn40B/IJIyV6gdVd/QINiEY5AiEA32y4DMpubHFG
Wm3dfXGlj7LlmPB7IqKUYj1mWI2gxUECIDEXnxim/SA+jasRyeqzdjrOhjc1Efw9
EbNwjLEI1w2pAiEAw7K605lMd3gQo4ywAPzWg7OzH+8kLAYz6ojVaKNFOwECIQC3
/Ad4bxTkMgEFF+t2qDq6hqsbaer2Ed8UDD2bMmUmxQ==
-----END RSA PRIVATE KEY-----
";

    /// A real, `p256`-crate-generated SEC1 (legacy) EC private key.
    const SEC1_EC_KEY_PEM: &str = "-----BEGIN EC PRIVATE KEY-----
MGsCAQEEIPzwcIZaq7X2XuZyZEwYWG26KEIzsXjdRjcuU/TZ/ZALoUQDQgAERVLg
GfYFaLOw5R54POkusOQBbHALMfM+od+lNwDmZGo6ng0AxzKPQ5CkBZJo/JdC/2Jk
3geXbP+Ka/7lQ2DCbA==
-----END EC PRIVATE KEY-----
";

    /// The PKCS#8 RSA private key's matching SPKI public key.
    const SPKI_PUBLIC_KEY_PEM: &str = "-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEOVGPrvwamRzUiwmQ1rE2SHRdlY/L
Qy/5obPgbof0CvPfw/Y1Y2/FyFMlFE7IqzbsiYS3OPJ5/fT+bccozfpAMA==
-----END PUBLIC KEY-----
";

    #[test]
    fn sniffs_pem_labels() {
        assert!(CertificateCore.sniff(b"-----BEGIN CERTIFICATE-----"));
        assert!(CertificateCore.sniff(b"-----BEGIN RSA PRIVATE KEY-----"));
        assert!(CertificateCore.sniff(b"-----BEGIN PUBLIC KEY-----"));
        assert!(!CertificateCore.sniff(b"not a pem file"));
        assert!(!CertificateCore.sniff(&[0xFF, 0xD8, 0xFF]));
    }

    #[test]
    fn views_a_real_certificate() {
        let path = unique_temp_file("cert.pem");
        std::fs::write(&path, CERT_PEM).unwrap();

        let data = CertificateCore.view(&path).unwrap();
        let view: CertificateKeyView = serde_json::from_value(data).unwrap();

        assert_eq!(view.entries.len(), 1);
        match &view.entries[0] {
            PemEntry::Certificate {
                subject,
                issuer,
                not_before,
                not_after,
                ..
            } => {
                assert_eq!(subject, "CN=Test Root CA, O=RepoSphereExplorer Test");
                assert_eq!(issuer, subject);
                assert!(not_before.contains("1975"));
                assert!(not_after.contains("4096"));
            }
            other => panic!("expected a Certificate entry, got {other:?}"),
        }

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_real_certificate_signing_request() {
        let path = unique_temp_file("request.csr");
        std::fs::write(&path, CSR_PEM).unwrap();

        let data = CertificateCore.view(&path).unwrap();
        let view: CertificateKeyView = serde_json::from_value(data).unwrap();

        assert_eq!(
            view.entries,
            vec![PemEntry::CertificateRequest {
                subject: "CN=Test CSR".to_owned(),
            }]
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_pkcs8_rsa_private_key() {
        let path = unique_temp_file("pkcs8_rsa.key");
        std::fs::write(&path, PKCS8_RSA_KEY_PEM).unwrap();

        let data = CertificateCore.view(&path).unwrap();
        let view: CertificateKeyView = serde_json::from_value(data).unwrap();

        assert_eq!(
            view.entries,
            vec![PemEntry::PrivateKey {
                algorithm: "RSA".to_owned(),
            }]
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_legacy_pkcs1_rsa_private_key() {
        let path = unique_temp_file("pkcs1_rsa.key");
        std::fs::write(&path, PKCS1_RSA_KEY_PEM).unwrap();

        let data = CertificateCore.view(&path).unwrap();
        let view: CertificateKeyView = serde_json::from_value(data).unwrap();

        assert_eq!(
            view.entries,
            vec![PemEntry::PrivateKey {
                algorithm: "RSA".to_owned(),
            }]
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_legacy_sec1_ec_private_key() {
        let path = unique_temp_file("sec1_ec.key");
        std::fs::write(&path, SEC1_EC_KEY_PEM).unwrap();

        let data = CertificateCore.view(&path).unwrap();
        let view: CertificateKeyView = serde_json::from_value(data).unwrap();

        assert_eq!(
            view.entries,
            vec![PemEntry::PrivateKey {
                algorithm: "EC".to_owned(),
            }]
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_pkcs8_ec_private_key_with_curve_name() {
        let path = unique_temp_file("pkcs8_ec.key");
        // Reuses the EC certificate's own key material shape: a PKCS#8
        // wrapper around a P-256 key, generated by `rcgen`.
        std::fs::write(
            &path,
            "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQglrhp3Dw4C/7ygCd+
kZvyTb8ELA79WuxH1aSuDM1XE9OhRANCAATDAClOAK7+zfiuV5f7qBjjXlV7azoQ
nAFDn+ErQaZpZIA6OoVHxU0Y+yFeU8aJvOvqlOSXegA7z08ynnnVqJLj
-----END PRIVATE KEY-----
",
        )
        .unwrap();

        let data = CertificateCore.view(&path).unwrap();
        let view: CertificateKeyView = serde_json::from_value(data).unwrap();

        assert_eq!(
            view.entries,
            vec![PemEntry::PrivateKey {
                algorithm: "EC (P-256)".to_owned(),
            }]
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_public_key() {
        let path = unique_temp_file("public.key");
        std::fs::write(&path, SPKI_PUBLIC_KEY_PEM).unwrap();

        let data = CertificateCore.view(&path).unwrap();
        let view: CertificateKeyView = serde_json::from_value(data).unwrap();

        assert_eq!(
            view.entries,
            vec![PemEntry::PublicKey {
                algorithm: "EC (P-256)".to_owned(),
            }]
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_multiple_blocks_in_one_file() {
        let path = unique_temp_file("chain_and_key.pem");
        let combined = format!("{CERT_PEM}{PKCS8_RSA_KEY_PEM}");
        std::fs::write(&path, combined).unwrap();

        let data = CertificateCore.view(&path).unwrap();
        let view: CertificateKeyView = serde_json::from_value(data).unwrap();

        assert_eq!(view.entries.len(), 2);
        assert!(matches!(view.entries[0], PemEntry::Certificate { .. }));
        assert!(matches!(view.entries[1], PemEntry::PrivateKey { .. }));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_a_certificate() {
        let data = serde_json::to_value(CertificateKeyView {
            entries: vec![PemEntry::Certificate {
                subject: "CN=example.com".to_owned(),
                issuer: "CN=Test Root CA".to_owned(),
                serial: "01:02:03".to_owned(),
                not_before: "Jan  1 00:00:00 2024 +00:00".to_owned(),
                not_after: "Jan  1 00:00:00 2025 +00:00".to_owned(),
            }],
        })
        .unwrap();

        let lines = CertificatePresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "Certificate".to_owned(),
                "Subject: CN=example.com".to_owned(),
                "Issuer: CN=Test Root CA".to_owned(),
                "Serial: 01:02:03".to_owned(),
                "Valid from Jan  1 00:00:00 2024 +00:00 to Jan  1 00:00:00 2025 +00:00".to_owned(),
            ]
        );
    }

    #[test]
    fn presents_a_private_key() {
        let data = serde_json::to_value(CertificateKeyView {
            entries: vec![PemEntry::PrivateKey {
                algorithm: "RSA".to_owned(),
            }],
        })
        .unwrap();

        let lines = CertificatePresentation.present(&data);

        assert_eq!(lines, vec!["RSA private key".to_owned()]);
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::CertificateCore),
            plugin_api::PluginPresentation::extensions(&crate::CertificatePresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn a_certificate_that_expires_now_is_expired() {
        assert_eq!(
            certificate_status(0, 1_000, 1_000),
            CertificateStatus::Expired
        );
    }

    #[test]
    fn a_certificate_expiring_in_exactly_thirty_days_is_expiring() {
        let now = 1_700_000_000;
        let not_after = now + EXPIRING_WITHIN_DAYS * 24 * 60 * 60;

        assert_eq!(
            certificate_status(0, not_after, now),
            CertificateStatus::Expiring
        );
    }

    #[test]
    fn a_certificate_expiring_in_thirty_one_days_is_valid() {
        let now = 1_700_000_000;
        let not_after = now + (EXPIRING_WITHIN_DAYS + 1) * 24 * 60 * 60;

        assert_eq!(
            certificate_status(0, not_after, now),
            CertificateStatus::Valid
        );
    }

    #[test]
    fn a_certificate_whose_validity_starts_now_is_valid() {
        let now = 1_700_000_000;
        let not_after = now + 365 * 24 * 60 * 60;

        assert_eq!(
            certificate_status(now, not_after, now),
            CertificateStatus::Valid
        );
    }

    #[test]
    fn scans_a_real_certificate_with_unix_seconds_and_self_signed() {
        let blocks = scan(CERT_PEM.as_bytes()).unwrap();

        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            ScannedBlock::Certificate(summary) => {
                assert_eq!(
                    summary.subject,
                    "CN=Test Root CA, O=RepoSphereExplorer Test"
                );
                assert_eq!(summary.issuer, summary.subject);
                assert!(summary.self_signed);
                assert!(summary.not_before < summary.not_after);
            }
            other => panic!("expected a certificate, got {other:?}"),
        }
    }

    #[test]
    fn scans_a_private_key_as_present_only() {
        let blocks = scan(PKCS8_RSA_KEY_PEM.as_bytes()).unwrap();

        assert_eq!(blocks, vec![ScannedBlock::PrivateKey]);
    }

    #[test]
    fn scans_a_certificate_signing_request_as_present_only() {
        let blocks = scan(CSR_PEM.as_bytes()).unwrap();

        assert_eq!(blocks, vec![ScannedBlock::CertificateRequest]);
    }

    #[test]
    fn scans_a_public_key_as_nothing_a_certificate_scan_wants() {
        let blocks = scan(SPKI_PUBLIC_KEY_PEM.as_bytes()).unwrap();

        assert!(blocks.is_empty());
    }

    #[test]
    fn a_file_with_no_pem_markers_at_all_scans_as_nothing_found() {
        let blocks = scan(b"not a pem file at all").unwrap();

        assert!(blocks.is_empty());
    }

    #[test]
    fn mismatched_begin_and_end_tags_are_an_error() {
        let malformed = "-----BEGIN CERTIFICATE-----\nAAAA\n-----END RSA PRIVATE KEY-----\n";

        let err = scan(malformed.as_bytes()).unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_certificate_block_with_bad_der_scans_as_unreadable_rather_than_dropped() {
        let bad = "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n";

        let blocks = scan(bad.as_bytes()).unwrap();

        assert_eq!(blocks, vec![ScannedBlock::Unreadable]);
    }

    #[test]
    fn scans_every_block_in_a_file_that_mixes_a_certificate_and_a_key() {
        let combined = format!("{CERT_PEM}{PKCS8_RSA_KEY_PEM}");

        let blocks = scan(combined.as_bytes()).unwrap();

        assert_eq!(blocks.len(), 2);
        assert!(matches!(blocks[0], ScannedBlock::Certificate(_)));
        assert_eq!(blocks[1], ScannedBlock::PrivateKey);
    }
}
