// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Bounded plaintext and reloadable-TLS listener support.

use std::fmt;
use std::fs;
use std::future;
use std::io::{self, Read as _};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, ensure};
use axum::serve::Listener;
use luminate_platform::secure_storage::open_private_file_for_read;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::{ServerConfig, crypto};
use rustls_pki_types::pem::PemObject as _;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::error::Elapsed;
use tokio::time::timeout;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;
use x509_parser::extensions::GeneralName;
use x509_parser::parse_x509_certificate;
use x509_parser::time::ASN1Time;

const CERTIFICATE_EXPIRY_WARNING: Duration = Duration::from_secs(30 * 24 * 60 * 60);

#[derive(Clone, Debug)]
pub(crate) struct TlsFiles {
    certificate: PathBuf,
    private_key: PathBuf,
}

impl TlsFiles {
    pub(crate) fn new(certificate: PathBuf, private_key: PathBuf) -> Self {
        Self {
            certificate,
            private_key,
        }
    }
}

#[derive(Debug)]
struct ReloadingCertificate {
    key: RwLock<Arc<CertifiedKey>>,
}

impl ResolvesServerCert for ReloadingCertificate {
    #[allow(
        clippy::expect_used,
        reason = "poisoned locks invalidate the active TLS configuration"
    )]
    fn resolve(&self, _client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(Arc::clone(
            &self.key.read().expect("TLS certificate lock poisoned"),
        ))
    }
}

#[derive(Clone)]
pub(crate) struct TlsReloader {
    files: TlsFiles,
    certificate: Arc<ReloadingCertificate>,
}

impl TlsReloader {
    #[allow(
        clippy::expect_used,
        clippy::unwrap_in_result,
        reason = "poisoned locks invalidate the active TLS configuration"
    )]
    pub(crate) fn reload(&self) -> anyhow::Result<()> {
        let key = load_certified_key(&self.files)?;
        *self
            .certificate
            .key
            .write()
            .expect("TLS certificate lock poisoned") = Arc::new(key);
        Ok(())
    }
}

pub(crate) struct BoundedListener {
    listener: TcpListener,
    acceptor: Option<TlsAcceptor>,
    connections: Arc<Semaphore>,
    handshake_timeout: Duration,
}

impl BoundedListener {
    pub(crate) fn plaintext(listener: TcpListener, maximum_connections: usize) -> Self {
        Self {
            listener,
            acceptor: None,
            connections: Arc::new(Semaphore::new(maximum_connections)),
            handshake_timeout: Duration::ZERO,
        }
    }

    pub(crate) fn tls(
        listener: TcpListener,
        maximum_connections: usize,
        handshake_timeout: Duration,
        files: TlsFiles,
    ) -> anyhow::Result<(Self, TlsReloader)> {
        let certificate = Arc::new(ReloadingCertificate {
            key: RwLock::new(Arc::new(load_certified_key(&files)?)),
        });
        let resolver: Arc<dyn ResolvesServerCert> = Arc::clone(&certificate) as Arc<_>;
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(resolver);
        let reloader = TlsReloader { files, certificate };
        Ok((
            Self {
                listener,
                acceptor: Some(TlsAcceptor::from(Arc::new(config))),
                connections: Arc::new(Semaphore::new(maximum_connections)),
                handshake_timeout,
            },
            reloader,
        ))
    }

    pub(crate) async fn accept_connection(&mut self) -> (Connection, SocketAddr) {
        <Self as Listener>::accept(self).await
    }
}

fn load_certified_key(files: &TlsFiles) -> anyhow::Result<CertifiedKey> {
    let certificate_bytes = fs::read(&files.certificate).with_context(|| {
        format!(
            "failed to read TLS certificate {}",
            files.certificate.display()
        )
    })?;
    let certificates = CertificateDer::pem_slice_iter(&certificate_bytes)
        .map(|result| result.map(CertificateDer::into_owned))
        .collect::<Result<Vec<_>, _>>()
        .context("failed to parse TLS certificate PEM")?;
    ensure!(
        !certificates.is_empty(),
        "TLS certificate PEM contains no certificates"
    );
    let expires_in = validate_certificate_chain(&certificates, SystemTime::now())?;
    if expires_in <= CERTIFICATE_EXPIRY_WARNING {
        tracing::warn!(
            expires_in_days = expires_in.as_secs() / (24 * 60 * 60),
            "TLS certificate expires soon"
        );
    }

    let key_bytes = read_private_file(&files.private_key)?;
    let private_key = PrivateKeyDer::from_pem_slice(&key_bytes)
        .context("failed to parse TLS private key PEM")?
        .clone_key();
    CertifiedKey::from_der(
        certificates,
        private_key,
        &crypto::aws_lc_rs::default_provider(),
    )
    .context("TLS certificate and private key are invalid or do not match")
}

fn validate_certificate_chain(
    certificates: &[CertificateDer<'_>],
    now: SystemTime,
) -> anyhow::Result<Duration> {
    for certificate in certificates {
        let (remaining, _) = parse_x509_certificate(certificate.as_ref())
            .map_err(|_| anyhow::anyhow!("TLS certificate contains invalid X.509 data"))?;
        ensure!(
            remaining.is_empty(),
            "TLS certificate contains trailing X.509 data"
        );
    }

    let leaf = certificates
        .first()
        .ok_or_else(|| anyhow::anyhow!("TLS certificate PEM contains no certificates"))?;
    validate_leaf_certificate(leaf, now)
}

fn validate_leaf_certificate(
    certificate: &CertificateDer<'_>,
    now: SystemTime,
) -> anyhow::Result<Duration> {
    let (_, certificate) = parse_x509_certificate(certificate.as_ref())
        .map_err(|_| anyhow::anyhow!("TLS leaf certificate contains invalid X.509 data"))?;
    let now = system_time_as_x509(now)?;
    let validity = certificate.validity();
    ensure!(
        now >= validity.not_before,
        "TLS leaf certificate is not yet valid"
    );
    ensure!(
        now <= validity.not_after,
        "TLS leaf certificate has expired"
    );

    let subject_alternative_name = certificate
        .subject_alternative_name()
        .map_err(|_| {
            anyhow::anyhow!("TLS leaf certificate has an invalid subject alternative name")
        })?
        .ok_or_else(|| anyhow::anyhow!("TLS leaf certificate has no subject alternative name"))?;
    let has_usable_identity = subject_alternative_name
        .value
        .general_names
        .iter()
        .any(|name| match name {
            GeneralName::DNSName(name) => !name.is_empty(),
            GeneralName::IPAddress(address) => matches!(address.len(), 4 | 16),
            GeneralName::OtherName(_, _)
            | GeneralName::RFC822Name(_)
            | GeneralName::X400Address(_)
            | GeneralName::DirectoryName(_)
            | GeneralName::EDIPartyName(_)
            | GeneralName::URI(_)
            | GeneralName::RegisteredID(_)
            | GeneralName::Invalid(_, _) => false,
        });
    ensure!(
        has_usable_identity,
        "TLS leaf certificate has no usable DNS or IP subject alternative name"
    );

    let seconds = validity.not_after.timestamp() - now.timestamp();
    let seconds = u64::try_from(seconds)
        .context("TLS leaf certificate expiry cannot be represented by this system")?;
    Ok(Duration::from_secs(seconds))
}

fn system_time_as_x509(time: SystemTime) -> anyhow::Result<ASN1Time> {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .context("system time is before the Unix epoch")?
        .as_secs();
    let seconds = i64::try_from(seconds).context("system time is too far in the future")?;
    ASN1Time::from_timestamp(seconds).context("system time is outside the X.509 date range")
}

fn read_private_file(path: &Path) -> anyhow::Result<Vec<u8>> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to inspect TLS private key {}", path.display()))?;
    ensure!(metadata.is_file(), "TLS private key is not a regular file");
    validate_private_file_access(&metadata)?;
    let mut file = open_private_file_for_read(path)
        .with_context(|| format!("failed to open private TLS key {}", path.display()))?;
    let mut contents = Vec::new();
    #[allow(
        clippy::verbose_file_reads,
        reason = "read through the securely opened handle to avoid a path replacement race"
    )]
    file.read_to_end(&mut contents)
        .with_context(|| format!("failed to read TLS private key {}", path.display()))?;
    Ok(contents)
}

#[cfg(unix)]
fn validate_private_file_access(metadata: &fs::Metadata) -> anyhow::Result<()> {
    use std::os::unix::fs::MetadataExt as _;

    ensure!(
        metadata.mode().trailing_zeros() >= 6,
        "TLS private key permissions must deny group and other access"
    );
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_file_access(_metadata: &fs::Metadata) -> anyhow::Result<()> {
    Ok(())
}

pub(crate) enum Connection {
    Plain(TcpStream, OwnedSemaphorePermit),
    Tls(Box<TlsStream<TcpStream>>, OwnedSemaphorePermit),
}

impl AsyncRead for Connection {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(stream, _permit) => Pin::new(stream).poll_read(cx, buffer),
            Self::Tls(stream, _permit) => Pin::new(stream).poll_read(cx, buffer),
        }
    }
}

impl AsyncWrite for Connection {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Plain(stream, _permit) => Pin::new(stream).poll_write(cx, buffer),
            Self::Tls(stream, _permit) => Pin::new(stream).poll_write(cx, buffer),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(stream, _permit) => Pin::new(stream).poll_flush(cx),
            Self::Tls(stream, _permit) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(stream, _permit) => Pin::new(stream).poll_shutdown(cx),
            Self::Tls(stream, _permit) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

impl Listener for BoundedListener {
    type Io = Connection;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let permit = match Arc::clone(&self.connections).acquire_owned().await {
                Ok(permit) => permit,
                Err(_closed) => future::pending().await,
            };
            let (stream, address) = match self.listener.accept().await {
                Ok(connection) => connection,
                Err(error) => {
                    tracing::error!(%error, "HTTP listener accept failed");
                    continue;
                }
            };
            if let Some(acceptor) = &self.acceptor {
                match bounded_tls_handshake(self.handshake_timeout, acceptor.accept(stream)).await {
                    Ok(Ok(stream)) => return (Connection::Tls(Box::new(stream), permit), address),
                    Ok(Err(error)) => {
                        tracing::warn!(peer = %address, %error, "TLS handshake rejected");
                    }
                    Err(_elapsed) => tracing::warn!(peer = %address, "TLS handshake timed out"),
                }
            } else {
                return (Connection::Plain(stream, permit), address);
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

async fn bounded_tls_handshake<F, T, E>(
    duration: Duration,
    handshake: F,
) -> Result<Result<T, E>, Elapsed>
where
    F: Future<Output = Result<T, E>>,
{
    timeout(duration, handshake).await
}

impl fmt::Debug for BoundedListener {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundedListener")
            .field("local_addr", &self.listener.local_addr())
            .field("tls", &self.acceptor.is_some())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BoundedListener, Connection, TlsFiles, bounded_tls_handshake, load_certified_key,
        validate_leaf_certificate,
    };
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::crypto::aws_lc_rs::default_provider;
    use rustls::pki_types::{ServerName, UnixTime};
    use rustls::version::{TLS12, TLS13};
    use rustls::{ClientConfig, DigitallySignedStruct, ProtocolVersion, SignatureScheme};
    use rustls_pki_types::CertificateDer;
    use rustls_pki_types::pem::PemObject as _;
    use std::env;
    use std::fs;
    use std::future;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, RwLock};
    use std::time::{Duration, UNIX_EPOCH};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::{TcpListener, TcpStream};
    use tokio_rustls::TlsConnector;
    use x509_parser::parse_x509_certificate;

    const DNS_CERTIFICATE: &[u8] = include_bytes!("../testdata/tls/dns-cert.pem");
    const DNS_PRIVATE_KEY: &[u8] = include_bytes!("../testdata/tls/dns-key.pem");
    const IP_CERTIFICATE: &[u8] = include_bytes!("../testdata/tls/ip-cert.pem");
    const IP_PRIVATE_KEY: &[u8] = include_bytes!("../testdata/tls/ip-key.pem");
    const NO_SAN_CERTIFICATE: &[u8] = include_bytes!("../testdata/tls/no-san-cert.pem");

    #[tokio::test]
    async fn stalled_tls_handshake_is_bounded() {
        let result = bounded_tls_handshake(
            Duration::from_millis(1),
            future::pending::<Result<(), ()>>(),
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn configured_provider_accepts_tls_1_2_and_1_3() {
        for (version, expected) in [
            (&TLS12, ProtocolVersion::TLSv1_2),
            (&TLS13, ProtocolVersion::TLSv1_3),
        ] {
            assert_eq!(negotiate_with_versions(&[version]).await, expected);
        }
    }

    #[tokio::test]
    async fn tls_listener_rejects_plaintext_without_yielding_a_connection() {
        let directory = fixture_directory("plaintext-rejection");
        let files = write_tls_files(&directory, DNS_CERTIFICATE, DNS_PRIVATE_KEY);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind TLS listener");
        let address = listener.local_addr().expect("TLS listener address");
        let (mut listener, _reloader) =
            BoundedListener::tls(listener, 2, Duration::from_secs(1), files)
                .expect("configure TLS listener");
        let accept = tokio::spawn(async move { listener.accept_connection().await });

        let mut plaintext = TcpStream::connect(address)
            .await
            .expect("connect plaintext");
        plaintext
            .write_all(b"GET /health/live HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .expect("write plaintext request");
        let mut response = Vec::new();
        let _ = plaintext.read_to_end(&mut response).await;
        assert!(!response.starts_with(b"HTTP/"));
        assert!(!accept.is_finished());

        accept.abort();
        fs::remove_dir_all(directory).expect("remove fixture directory");
    }

    async fn negotiate_with_versions(
        versions: &[&'static rustls::SupportedProtocolVersion],
    ) -> ProtocolVersion {
        let directory = fixture_directory("version");
        let files = write_tls_files(&directory, DNS_CERTIFICATE, DNS_PRIVATE_KEY);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind TLS listener");
        let address = listener.local_addr().expect("TLS listener address");
        let (mut listener, _reloader) =
            BoundedListener::tls(listener, 1, Duration::from_secs(2), files)
                .expect("configure TLS listener");
        let accept = tokio::spawn(async move {
            match listener.accept_connection().await.0 {
                Connection::Tls(stream, _) => stream
                    .get_ref()
                    .1
                    .protocol_version()
                    .expect("negotiated protocol version"),
                Connection::Plain(_, _) => panic!("TLS listener yielded plaintext"),
            }
        });

        let provider = default_provider();
        let config = ClientConfig::builder_with_provider(Arc::new(provider))
            .with_protocol_versions(versions)
            .expect("supported test protocol version")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(TestCertificateVerifier))
            .with_no_client_auth();
        let connector = TlsConnector::from(Arc::new(config));
        let stream = TcpStream::connect(address)
            .await
            .expect("connect TLS client");
        let _stream = connector
            .connect(
                ServerName::try_from("localhost").expect("server name"),
                stream,
            )
            .await
            .expect("negotiate TLS");
        let negotiated = accept.await.expect("accept TLS connection");
        fs::remove_dir_all(directory).expect("remove fixture directory");
        negotiated
    }

    #[derive(Debug)]
    struct TestCertificateVerifier;

    impl ServerCertVerifier for TestCertificateVerifier {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _certificate: &CertificateDer<'_>,
            _signature: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _certificate: &CertificateDer<'_>,
            _signature: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            default_provider()
                .signature_verification_algorithms
                .supported_schemes()
        }
    }

    #[test]
    fn dns_and_ip_subject_alternative_names_are_accepted() {
        for pem in [DNS_CERTIFICATE, IP_CERTIFICATE] {
            let certificate = certificate(pem);
            let (_, parsed) = parse_x509_certificate(certificate.as_ref()).expect("parse fixture");
            let validity = parsed.validity();
            let midpoint = validity.not_before.timestamp()
                + (validity.not_after.timestamp() - validity.not_before.timestamp()) / 2;
            let now = UNIX_EPOCH + Duration::from_secs(u64::try_from(midpoint).expect("timestamp"));

            assert!(validate_leaf_certificate(&certificate, now).is_ok());
        }
    }

    #[test]
    fn absent_usable_subject_alternative_name_is_rejected() {
        let certificate = certificate(NO_SAN_CERTIFICATE);
        let (_, parsed) = parse_x509_certificate(certificate.as_ref()).expect("parse fixture");
        let now = UNIX_EPOCH
            + Duration::from_secs(
                u64::try_from(parsed.validity().not_before.timestamp() + 1).expect("timestamp"),
            );

        let error = validate_leaf_certificate(&certificate, now).expect_err("missing SAN");
        assert!(error.to_string().contains("no subject alternative name"));
    }

    #[test]
    fn expired_and_not_yet_valid_certificates_are_rejected() {
        let certificate = certificate(DNS_CERTIFICATE);
        let (_, parsed) = parse_x509_certificate(certificate.as_ref()).expect("parse fixture");
        let before = UNIX_EPOCH
            + Duration::from_secs(
                u64::try_from(parsed.validity().not_before.timestamp() - 1).expect("timestamp"),
            );
        let after = UNIX_EPOCH
            + Duration::from_secs(
                u64::try_from(parsed.validity().not_after.timestamp() + 1).expect("timestamp"),
            );

        assert!(
            validate_leaf_certificate(&certificate, before)
                .expect_err("not yet valid")
                .to_string()
                .contains("not yet valid")
        );
        assert!(
            validate_leaf_certificate(&certificate, after)
                .expect_err("expired")
                .to_string()
                .contains("expired")
        );
    }

    #[test]
    fn malformed_certificate_is_rejected() {
        let directory = fixture_directory("malformed");
        let files = write_tls_files(
            &directory,
            b"-----BEGIN CERTIFICATE-----\nnot-base64!\n-----END CERTIFICATE-----\n",
            DNS_PRIVATE_KEY,
        );

        let error = load_certified_key(&files).expect_err("malformed certificate");
        assert!(error.to_string().contains("parse TLS certificate PEM"));
        fs::remove_dir_all(directory).expect("remove fixture directory");
    }

    #[test]
    fn mismatched_pair_is_rejected_and_reload_is_atomic() {
        let directory = fixture_directory("reload");
        let files = write_tls_files(&directory, DNS_CERTIFICATE, DNS_PRIVATE_KEY);
        let certificate = Arc::new(super::ReloadingCertificate {
            key: RwLock::new(Arc::new(load_certified_key(&files).expect("initial pair"))),
        });
        let reloader = super::TlsReloader {
            files: files.clone(),
            certificate: Arc::clone(&certificate),
        };
        let initial = Arc::clone(&certificate.key.read().expect("read active certificate"));

        fs::write(&files.certificate, IP_CERTIFICATE).expect("replace certificate");
        assert!(reloader.reload().is_err());
        assert!(Arc::ptr_eq(
            &initial,
            &certificate.key.read().expect("read retained certificate")
        ));

        fs::write(&files.private_key, IP_PRIVATE_KEY).expect("replace key");
        reloader.reload().expect("reload matching replacement");
        assert!(!Arc::ptr_eq(
            &initial,
            &certificate
                .key
                .read()
                .expect("read replacement certificate")
        ));
        fs::remove_dir_all(directory).expect("remove fixture directory");
    }

    fn certificate(pem: &[u8]) -> CertificateDer<'static> {
        CertificateDer::from_pem_slice(pem)
            .expect("decode certificate fixture")
            .into_owned()
    }

    fn fixture_directory(name: &str) -> PathBuf {
        let directory =
            env::temp_dir().join(format!("luminate-http-tls-{name}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&directory).expect("create fixture directory");
        directory
    }

    fn write_tls_files(directory: &Path, certificate: &[u8], key: &[u8]) -> TlsFiles {
        let certificate_path = directory.join("certificate.pem");
        let key_path = directory.join("key.pem");
        fs::write(&certificate_path, certificate).expect("write certificate");
        fs::write(&key_path, key).expect("write key");
        protect_test_key(&key_path);
        TlsFiles::new(certificate_path, key_path)
    }

    #[cfg(unix)]
    fn protect_test_key(path: &Path) {
        use std::os::unix::fs::PermissionsExt as _;

        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("protect key");
    }

    #[cfg(not(unix))]
    fn protect_test_key(_path: &Path) {}
}
