//! The client certificate used to log in with CertFP or SASL EXTERNAL.
//!
//! OpenIRC can create a self-signed certificate (ECDSA P-256) and save it
//! with its private key in one PEM file. Services identify you by the
//! certificate's SHA-256 fingerprint, which you register once, for example
//! with `/msg NickServ CERT ADD`.

use std::io;
use std::path::Path;

use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;

/// Create a certificate for `common_name` and write it, with its private
/// key, to `path`. Returns the certificate's SHA-256 fingerprint.
pub fn create(common_name: &str, path: &Path) -> io::Result<String> {
    let key = rcgen::KeyPair::generate().map_err(io::Error::other)?;
    let mut params =
        rcgen::CertificateParams::new(Vec::<String>::new()).map_err(io::Error::other)?;
    let name = if common_name.trim().is_empty() {
        "OpenIRC user"
    } else {
        common_name.trim()
    };
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, name);
    let cert = params.self_signed(&key).map_err(io::Error::other)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let pem = format!("{}{}", cert.pem(), key.serialize_pem());
    write_private(path, pem.as_bytes())?;
    Ok(crate::tls::sha256_hex(cert.der()))
}

/// The SHA-256 fingerprint of the first certificate in a PEM file.
pub fn fingerprint(path: &str) -> io::Result<String> {
    let cert = CertificateDer::pem_file_iter(path)
        .map_err(|err| io::Error::other(format!("cannot read {path}: {err}")))?
        .next()
        .ok_or_else(|| io::Error::other(format!("{path} holds no certificate")))?
        .map_err(|err| io::Error::other(format!("cannot read {path}: {err}")))?;
    Ok(crate::tls::sha256_hex(cert.as_ref()))
}

/// Write a file only the user can read: it holds a private key.
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    // A file that was already there keeps its old permissions when opened;
    // make it private before the key goes in.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_a_usable_certificate_and_reads_its_fingerprint() {
        let dir = std::env::temp_dir().join(format!("openirc-cert-{}", std::process::id()));
        let path = dir.join("client.pem");
        std::fs::create_dir_all(&dir).unwrap();
        // An older file anyone could read is made private.
        std::fs::write(&path, b"old").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        let made = create("tester", &path).unwrap();
        assert_eq!(made.len(), 64);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "{mode:o}");
        }
        assert_eq!(fingerprint(path.to_str().unwrap()).unwrap(), made);
        let (certs, _key) = crate::tls::load_client_cert(path.to_str().unwrap()).unwrap();
        assert_eq!(certs.len(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
