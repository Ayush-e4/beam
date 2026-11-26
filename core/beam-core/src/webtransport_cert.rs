//! The certificate a WebTransport server presents to browsers.
//!
//! Browsers can't check a phone's certificate against a certificate authority, so the browser
//! pins it by its SHA-256 instead (the phone sends the hash through the sealed signaling room).
//! Browsers only accept pinned certificates that use ECDSA P-256 and are valid for at most
//! 14 days, so a new one is made every time the server starts.

use std::time::Duration;

use rcgen::{CertificateParams, KeyPair, PKCS_ECDSA_P256_SHA256};
use ring::digest::{SHA256, digest};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use time::OffsetDateTime;

use crate::{BeamError, Result};

/// The name in the certificate; browsers pinning by hash don't check it.
const NAME: &str = "beam";
/// Backdated a little, so a browser whose clock is slightly behind still accepts it.
const BACKDATE: Duration = Duration::from_hours(1);
/// Browsers refuse pinned certificates valid for longer than 14 days.
const LIFETIME: Duration = Duration::from_hours(13 * 24);

/// A fresh certificate, its key, and the SHA-256 that browsers pin.
pub(crate) struct PinnedCertificate {
    pub(crate) certificate: CertificateDer<'static>,
    pub(crate) key: PrivateKeyDer<'static>,
    pub(crate) hash: [u8; 32],
}

/// Makes a certificate valid from an hour before `now` for 13 days.
pub(crate) fn generate(now: OffsetDateTime) -> Result<PinnedCertificate> {
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(|err| failed(&err))?;
    let params = params(now)?;
    let certificate = params.self_signed(&key).map_err(|err| failed(&err))?;
    let der = certificate.der().clone();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(digest(&SHA256, &der).as_ref());
    Ok(PinnedCertificate {
        certificate: der,
        key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
        hash,
    })
}

fn params(now: OffsetDateTime) -> Result<CertificateParams> {
    let mut params = CertificateParams::new(vec![NAME.to_owned()]).map_err(|err| failed(&err))?;
    params.not_before = now - BACKDATE;
    params.not_after = now + LIFETIME;
    Ok(params)
}

fn failed(err: &dyn std::fmt::Display) -> BeamError {
    BeamError::Handshake(format!("WebTransport certificate failed: {err}"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn it_is_valid_for_less_than_14_days_and_already_valid() {
        let now = OffsetDateTime::now_utc();
        let params = params(now).unwrap();
        assert!(params.not_before < now);
        assert!(params.not_after > now);
        let lifetime = params.not_after - params.not_before;
        assert!(lifetime < time::Duration::days(14), "{lifetime}");
    }

    #[test]
    fn the_pin_is_the_sha256_of_the_certificate_and_each_one_is_new() {
        let now = OffsetDateTime::now_utc();
        let first = generate(now).unwrap();
        let second = generate(now).unwrap();
        assert_eq!(
            first.hash.as_slice(),
            digest(&SHA256, &first.certificate).as_ref()
        );
        assert_ne!(first.hash, second.hash);
    }

    #[test]
    fn it_uses_ecdsa_p256_as_browsers_require() {
        // The DER encoding of the prime256v1 curve's object identifier (1.2.840.10045.3.1.7).
        const P256: [u8; 10] = [0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
        let pinned = generate(OffsetDateTime::now_utc()).unwrap();
        assert!(pinned.certificate.windows(P256.len()).any(|w| w == P256));
    }
}
