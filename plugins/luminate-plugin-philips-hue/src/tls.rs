// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Hue-specific TLS trust and bridge-identity binding.

use std::io;
use std::net::{TcpStream, ToSocketAddrs as _};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::{
    CertificateError, ClientConfig, ClientConnection, DigitallySignedStruct, Error as RustlsError,
    RootCertStore, SignatureScheme, StreamOwned,
};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use thiserror::Error;
use x509_parser::parse_x509_certificate;

use crate::configuration::{BridgeId, Endpoint};

const MAX_RESOLVED_ADDRESSES: usize = 16;

// Signify publishes both roots so clients continue working when issuance moves
// from the currently active root to the secondary root.
const HUE_ROOTS_PEM: &str = "-----BEGIN CERTIFICATE-----
MIICMjCCAdigAwIBAgIUO7FSLbaxikuXAljzVaurLXWmFw4wCgYIKoZIzj0EAwIw
OTELMAkGA1UEBhMCTkwxFDASBgNVBAoMC1BoaWxpcHMgSHVlMRQwEgYDVQQDDAty
b290LWJyaWRnZTAiGA8yMDE3MDEwMTAwMDAwMFoYDzIwMzgwMTE5MDMxNDA3WjA5
MQswCQYDVQQGEwJOTDEUMBIGA1UECgwLUGhpbGlwcyBIdWUxFDASBgNVBAMMC3Jv
b3QtYnJpZGdlMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEjNw2tx2AplOf9x86
aTdvEcL1FU65QDxziKvBpW9XXSIcibAeQiKxegpq8Exbr9v6LBnYbna2VcaK0G22
jOKkTqOBuTCBtjAPBgNVHRMBAf8EBTADAQH/MA4GA1UdDwEB/wQEAwIBhjAdBgNV
HQ4EFgQUZ2ONTFrDT6o8ItRnKfqWKnHFGmQwdAYDVR0jBG0wa4AUZ2ONTFrDT6o8
ItRnKfqWKnHFGmShPaQ7MDkxCzAJBgNVBAYTAk5MMRQwEgYDVQQKDAtQaGlsaXBz
IEh1ZTEUMBIGA1UEAwwLcm9vdC1icmlkZ2WCFDuxUi22sYpLlwJY81Wrqy11phcO
MAoGCCqGSM49BAMCA0gAMEUCIEBYYEOsa07TH7E5MJnGw557lVkORgit2Rm1h3B2
sFgDAiEA1Fj/C3AN5psFMjo0//mrQebo0eKd3aWRx+pQY08mk48=
-----END CERTIFICATE-----
-----BEGIN CERTIFICATE-----
MIIBzDCCAXOgAwIBAgICEAAwCgYIKoZIzj0EAwIwPDELMAkGA1UEBhMCTkwxFDAS
BgNVBAoMC1NpZ25pZnkgSHVlMRcwFQYDVQQDDA5IdWUgUm9vdCBDQSAwMTAgFw0y
NTAyMjUwMDAwMDBaGA8yMDUwMTIzMTIzNTk1OVowPDELMAkGA1UEBhMCTkwxFDAS
BgNVBAoMC1NpZ25pZnkgSHVlMRcwFQYDVQQDDA5IdWUgUm9vdCBDQSAwMTBZMBMG
ByqGSM49AgEGCCqGSM49AwEHA0IABFfOO0jfSAUXGQ9kjEDzyBrcMQ3ItyA5krE+
cyvb1Y3xFti7KlAad8UOnAx0FBLn7HZrlmIwm1QnX0fK3LPM13mjYzBhMB0GA1Ud
DgQWBBTF1pSpsCASX/z0VHLigxU2CAaqoTAfBgNVHSMEGDAWgBTF1pSpsCASX/z0
VHLigxU2CAaqoTAPBgNVHRMBAf8EBTADAQH/MA4GA1UdDwEB/wQEAwIBBjAKBggq
hkjOPQQDAgNHADBEAiAk7duT+IHbOGO4UUuGLAEpyYejGZK9Z7V9oSfnvuQ5BQIg
IYSgwwxHXm73/JgcU9lAM6c8Bmu3UE3kBIUwBs1qXFw=
-----END CERTIFICATE-----";

pub(crate) type TlsStream = StreamOwned<ClientConnection, TcpStream>;

pub(crate) fn client_config(bridge_id: &BridgeId) -> Result<Arc<ClientConfig>, TlsError> {
    let certificates = decode_pem_certificates(HUE_ROOTS_PEM)?;
    let mut roots = RootCertStore::empty();
    for certificate in certificates {
        roots
            .add(CertificateDer::from(certificate))
            .map_err(|_| TlsError::InvalidTrustAnchor)?;
    }
    let verifier = WebPkiServerVerifier::builder(Arc::new(roots))
        .build()
        .map_err(|_| TlsError::InvalidTrustAnchor)?;
    let verifier = Arc::new(HueServerVerifier {
        chain_verifier: verifier,
        bridge_id: bridge_id.clone(),
    });
    let mut config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

#[derive(Debug)]
struct HueServerVerifier {
    chain_verifier: Arc<WebPkiServerVerifier>,
    bridge_id: BridgeId,
}

impl ServerCertVerifier for HueServerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        match self.chain_verifier.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Ok(_)
            | Err(RustlsError::InvalidCertificate(
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
            )) => {}
            Err(error) => return Err(error),
        }

        let common_name = certificate_common_name(end_entity).ok_or_else(identity_error)?;
        if BridgeId::parse(&common_name).as_ref() != Ok(&self.bridge_id) {
            return Err(identity_error());
        }

        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        self.chain_verifier
            .verify_tls12_signature(message, certificate, signature)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        self.chain_verifier
            .verify_tls13_signature(message, certificate, signature)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.chain_verifier.supported_verify_schemes()
    }
}

fn certificate_common_name(certificate: &CertificateDer<'_>) -> Option<String> {
    let (remaining, certificate) = parse_x509_certificate(certificate.as_ref()).ok()?;
    if !remaining.is_empty() {
        return None;
    }
    let mut common_names = certificate.subject().iter_common_name();
    let common_name = common_names.next()?.as_str().ok()?;
    if common_names.next().is_some() {
        return None;
    }
    Some(common_name.to_owned())
}

fn identity_error() -> RustlsError {
    RustlsError::InvalidCertificate(CertificateError::ApplicationVerificationFailure)
}

pub(crate) fn connect(
    config: Arc<ClientConfig>,
    endpoint: &Endpoint,
    bridge_id: &BridgeId,
    timeout: Duration,
) -> Result<TlsStream, TlsError> {
    let started = Instant::now();
    let addresses = (endpoint.host(), endpoint.port())
        .to_socket_addrs()
        .map_err(TlsError::Io)?
        .take(MAX_RESOLVED_ADDRESSES)
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(TlsError::NoAddresses);
    }

    let mut last_error = None;
    let mut tcp = None;
    for address in addresses {
        let remaining = timeout
            .checked_sub(started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or(TlsError::TimedOut)?;
        match TcpStream::connect_timeout(&address, remaining) {
            Ok(stream) => {
                tcp = Some(stream);
                break;
            }
            Err(error) => last_error = Some(error),
        }
    }
    let tcp = tcp.ok_or_else(|| last_error.map_or(TlsError::NoAddresses, TlsError::Io))?;
    let remaining = timeout
        .checked_sub(started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or(TlsError::TimedOut)?;
    tcp.set_read_timeout(Some(remaining))?;
    tcp.set_write_timeout(Some(remaining))?;

    // Hue device certificates identify the bridge by its canonical bridge ID,
    // independently of the IP address or hostname used to route the socket.
    let server_name = ServerName::try_from(bridge_id.as_str().to_owned())
        .map_err(|_| TlsError::InvalidBridgeId)?;
    let connection = ClientConnection::new(config, server_name).map_err(TlsError::Rustls)?;
    Ok(StreamOwned::new(connection, tcp))
}

fn decode_pem_certificates(pem: &str) -> Result<Vec<Vec<u8>>, TlsError> {
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END: &str = "-----END CERTIFICATE-----";
    let mut certificates = Vec::new();
    let mut rest = pem;
    while let Some((_, after_begin)) = rest.split_once(BEGIN) {
        let (body, after_end) = after_begin.split_once(END).ok_or(TlsError::InvalidPem)?;
        certificates.push(decode_base64(body)?);
        rest = after_end;
    }
    if certificates.len() != 2 {
        return Err(TlsError::InvalidPem);
    }
    Ok(certificates)
}

fn decode_base64(value: &str) -> Result<Vec<u8>, TlsError> {
    let symbols = value
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    if symbols.is_empty() || symbols.len() % 4 != 0 {
        return Err(TlsError::InvalidPem);
    }
    let mut output = Vec::with_capacity(symbols.len() / 4 * 3);
    for quartet in symbols.chunks_exact(4) {
        let [a_symbol, b_symbol, c_symbol, d_symbol] = quartet else {
            return Err(TlsError::InvalidPem);
        };
        let a = decode_symbol(*a_symbol)?;
        let b = decode_symbol(*b_symbol)?;
        let c = decode_optional_symbol(*c_symbol)?;
        let d = decode_optional_symbol(*d_symbol)?;
        output.push((a << 2) | (b >> 4));
        if *c_symbol != b'=' {
            output.push((b << 4) | (c >> 2));
        }
        if *d_symbol != b'=' {
            output.push((c << 6) | d);
        }
    }
    Ok(output)
}

fn decode_optional_symbol(symbol: u8) -> Result<u8, TlsError> {
    if symbol == b'=' {
        Ok(0)
    } else {
        decode_symbol(symbol)
    }
}

fn decode_symbol(symbol: u8) -> Result<u8, TlsError> {
    match symbol {
        b'A'..=b'Z' => Ok(symbol - b'A'),
        b'a'..=b'z' => Ok(symbol - b'a' + 26),
        b'0'..=b'9' => Ok(symbol - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(TlsError::InvalidPem),
    }
}

#[derive(Debug, Error)]
pub(crate) enum TlsError {
    #[error("Hue trust-anchor bundle is malformed")]
    InvalidPem,
    #[error("Hue trust-anchor bundle contains an invalid certificate")]
    InvalidTrustAnchor,
    #[error("configured bridge ID cannot be used as a TLS server identity")]
    InvalidBridgeId,
    #[error("bridge endpoint resolved to no addresses")]
    NoAddresses,
    #[error("bridge connection deadline expired")]
    TimedOut,
    #[error("bridge TLS validation failed: {0}")]
    Rustls(rustls::Error),
    #[error("bridge connection failed: {0}")]
    Io(#[from] io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_hue_roots_decode_and_build_a_client_config() {
        let certificates = decode_pem_certificates(HUE_ROOTS_PEM).expect("decode Hue roots");
        assert_eq!(certificates.len(), 2);
        assert!(
            certificates
                .iter()
                .all(|certificate| certificate.first() == Some(&0x30))
        );
        let bridge_id = BridgeId::parse("001788fffe123456").expect("bridge ID");
        assert!(client_config(&bridge_id).is_ok());

        let active_root = CertificateDer::from(certificates[0].clone());
        assert_eq!(
            certificate_common_name(&active_root),
            Some("root-bridge".to_owned())
        );
    }

    #[test]
    fn malformed_pem_and_base64_are_rejected() {
        assert!(decode_pem_certificates("not pem").is_err());
        assert!(decode_base64("!!!!").is_err());
    }
}
