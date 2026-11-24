//! TLS settings for the QUIC transport.
//!
//! QUIC always runs TLS 1.3, but Beam does not rely on it for identity: every connection runs a
//! Noise handshake inside the QUIC stream, which authenticates the peer's device key and
//! encrypts end to end. TLS here only needs to set up the QUIC packet keys, so each endpoint
//! uses a throwaway self-signed certificate and clients accept any certificate. TLS signatures
//! are still verified, so the TLS handshake itself stays well formed.

use std::sync::Arc;

use rustls::DigitallySignedStruct;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, ring, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};

use crate::{BeamError, Result};

/// ALPN protocol identifier for Beam over QUIC.
pub const ALPN: &[u8] = b"beam/1";
/// Name placed in the throwaway certificate and sent as SNI.
pub const SERVER_NAME: &str = "beam";

/// TLS configuration for accepting QUIC connections.
pub(crate) fn server_config() -> Result<rustls::ServerConfig> {
    let certified = rcgen::generate_simple_self_signed(vec![SERVER_NAME.to_owned()])
        .map_err(|err| tls_error(&err))?;
    let cert = certified.cert.der().clone();
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        certified.signing_key.serialize_der(),
    ));
    let mut config = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|err| tls_error(&err))?
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(|err| tls_error(&err))?;
    config.alpn_protocols = vec![ALPN.to_vec()];
    Ok(config)
}

/// TLS configuration for opening QUIC connections.
pub(crate) fn client_config() -> Result<rustls::ClientConfig> {
    let provider = provider();
    let verifier = Arc::new(AnyCertificate(provider.clone()));
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|err| tls_error(&err))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    config.alpn_protocols = vec![ALPN.to_vec()];
    Ok(config)
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(ring::default_provider())
}

fn tls_error(err: &dyn std::fmt::Display) -> BeamError {
    BeamError::Handshake(format!("QUIC TLS setup failed: {err}"))
}

/// Accepts any server certificate; identity is proven by the Noise handshake instead.
#[derive(Debug)]
struct AnyCertificate(Arc<CryptoProvider>);

impl ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}
