//! TLS for IRC connections: rustls with the ring provider and Mozilla's
//! roots, a client certificate when one is set, and certificates the user
//! chose to trust by fingerprint (for servers with self-signed or otherwise
//! unverifiable certificates).

use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};

use rustls::StreamOwned;
use rustls::client::ClientConnection;
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

pub type TlsStream = StreamOwned<ClientConnection, std::net::TcpStream>;

/// TLS settings for one connection.
#[derive(Clone, Debug, Default)]
pub struct TlsOptions {
    /// PEM file with the client certificate and its private key.
    pub client_cert: Option<String>,
    /// SHA-256 fingerprints (lowercase hex) of server certificates the user
    /// trusts even though they do not verify.
    pub trusted: Vec<String>,
}

/// A server certificate that did not verify and is not trusted. The window
/// shows this and can add the fingerprint to the trusted list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UntrustedCert {
    pub host: String,
    pub fingerprint: String,
    pub reason: String,
}

impl std::fmt::Display for UntrustedCert {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the certificate of {} is not trusted ({}); SHA-256 {}",
            self.host, self.reason, self.fingerprint
        )
    }
}

impl std::error::Error for UntrustedCert {}

/// The rejected certificate's fingerprint and reason, filled in by the
/// verifier when it refuses one.
pub type Rejection = Arc<Mutex<Option<(String, String)>>>;

pub fn sha256_hex(der: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, der)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug)]
struct TrustingVerifier {
    inner: Arc<WebPkiServerVerifier>,
    trusted: Vec<String>,
    rejected: Rejection,
}

impl ServerCertVerifier for TrustingVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Ok(verified) => Ok(verified),
            Err(err) => {
                let fingerprint = sha256_hex(end_entity.as_ref());
                if self
                    .trusted
                    .iter()
                    .any(|item| item.eq_ignore_ascii_case(&fingerprint))
                {
                    return Ok(ServerCertVerified::assertion());
                }
                if let Ok(mut slot) = self.rejected.lock() {
                    *slot = Some((fingerprint, describe(&err)));
                }
                Err(err)
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// A short, plain description of why a certificate was refused.
fn describe(err: &rustls::Error) -> String {
    use rustls::CertificateError as C;
    match err {
        rustls::Error::InvalidCertificate(C::UnknownIssuer) => {
            "issued by an unknown authority, or self-signed".into()
        }
        rustls::Error::InvalidCertificate(C::Expired | C::ExpiredContext { .. }) => {
            "expired".into()
        }
        rustls::Error::InvalidCertificate(C::NotValidYet | C::NotValidYetContext { .. }) => {
            "not valid yet".into()
        }
        rustls::Error::InvalidCertificate(
            C::NotValidForName | C::NotValidForNameContext { .. },
        ) => "made for a different server name".into(),
        rustls::Error::InvalidCertificate(C::Revoked) => "revoked".into(),
        rustls::Error::InvalidCertificate(C::Other(other))
            if format!("{other:?}").contains("CaUsedAsEndEntity") =>
        {
            "self-signed".into()
        }
        rustls::Error::InvalidCertificate(C::BadSignature | C::BadEncoding) => {
            "damaged or badly signed".into()
        }
        other => other.to_string(),
    }
}

/// Read a client certificate chain and private key from one PEM file.
pub fn load_client_cert(
    path: &str,
) -> io::Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(path)
        .map_err(|err| io::Error::other(format!("cannot read certificate {path}: {err}")))?
        .collect::<Result<_, _>>()
        .map_err(|err| io::Error::other(format!("cannot read certificate {path}: {err}")))?;
    if certs.is_empty() {
        return Err(io::Error::other(format!("{path} holds no certificate")));
    }
    let key = PrivateKeyDer::from_pem_file(path)
        .map_err(|err| io::Error::other(format!("cannot read the private key in {path}: {err}")))?;
    Ok((certs, key))
}

pub fn wrap(
    host: &str,
    tcp: std::net::TcpStream,
    options: &TlsOptions,
) -> io::Result<(TlsStream, Rejection)> {
    let name = ServerName::try_from(host)
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("`{host}` is not a DNS name rustls can use for TLS"),
            )
        })?
        .to_owned();

    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let inner = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
        .build()
        .map_err(io::Error::other)?;
    let rejected: Rejection = Arc::default();
    let verifier = Arc::new(TrustingVerifier {
        inner,
        trusted: options.trusted.clone(),
        rejected: rejected.clone(),
    });
    let builder = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(io::Error::other)?
        .dangerous()
        .with_custom_certificate_verifier(verifier);
    let config = match options
        .client_cert
        .as_deref()
        .filter(|path| !path.trim().is_empty())
    {
        Some(path) => {
            let (certs, key) = load_client_cert(path)?;
            builder.with_client_auth_cert(certs, key).map_err(|err| {
                io::Error::other(format!("the client certificate cannot be used: {err}"))
            })?
        }
        None => builder.with_no_client_auth(),
    };

    let connection = ClientConnection::new(Arc::new(config), name).map_err(io::Error::other)?;
    Ok((StreamOwned::new(connection, tcp), rejected))
}

pub struct Socket {
    inner: SocketInner,
}

enum SocketInner {
    Plain(std::net::TcpStream),
    Tls(Box<TlsStream>),
}

impl Socket {
    pub fn plain(stream: std::net::TcpStream) -> Self {
        Self {
            inner: SocketInner::Plain(stream),
        }
    }

    pub fn tls(stream: TlsStream) -> Self {
        Self {
            inner: SocketInner::Tls(Box::new(stream)),
        }
    }

    pub fn peer_sha256(&self) -> Option<String> {
        let SocketInner::Tls(stream) = &self.inner else {
            return None;
        };
        let cert = stream.conn.peer_certificates()?.first()?;
        Some(sha256_hex(cert.as_ref()))
    }
}

impl Read for Socket {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match &mut self.inner {
            SocketInner::Plain(stream) => stream.read(buf),
            SocketInner::Tls(stream) => stream.read(buf),
        }
    }
}

impl Write for Socket {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &mut self.inner {
            SocketInner::Plain(stream) => stream.write(buf),
            SocketInner::Tls(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.inner {
            SocketInner::Plain(stream) => stream.flush(),
            SocketInner::Tls(stream) => stream.flush(),
        }
    }
}
