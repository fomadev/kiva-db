// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Encrypted connections (TLS) on every port, without a second port.
//!
//! A TLS connection starts with the byte 0x16 (a handshake record), which no
//! KivaDB, HTTP or Redis client sends first in clear. Each listener looks at
//! that first byte: plain connections go on as before, encrypted ones are
//! decrypted by a relay and handed to the same code as a plain connection.
//!
//! The relay pairs the client's socket with a loopback socket: the protocol
//! code reads and writes plain bytes on its end of the pair, two threads move
//! bytes between the pair and the encrypted socket. The client's real address
//! is passed along separately, so "connections from this machine" stay what
//! they were.
//!
//! The certificate is the one configured (`tls.cert`, `tls.key`), or one
//! KivaDB creates on first start in `<home>/tls/`. Clients that cannot check
//! it against an authority (a server known by its IP address has none) check
//! its SHA-256 fingerprint instead, as SSH does with host keys.

use crate::config::Config;
use crate::error::{KivaError, Result};
use crate::instance::Instance;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{ring as provider, verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{ClientConfig, Connection, DigitallySignedStruct, ServerConfig, SignatureScheme};
use sha2::{Digest, Sha256};
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// First byte of a TLS handshake record.
pub const HANDSHAKE: u8 = 0x16;

fn crypto() -> Arc<CryptoProvider> {
    Arc::new(provider::default_provider())
}

fn tls_err(e: impl std::fmt::Display) -> KivaError {
    KivaError::Other(format!("TLS: {e}"))
}

// ---------------------------------------------------------------- fingerprints

/// `AB:CD:...`: the SHA-256 of a certificate, as browsers and Node.js show it.
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der).iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

/// A fingerprint written any usual way (with or without colons, any case),
/// as 64 lowercase hex digits; `None` if it is not one.
pub fn normalize_fingerprint(s: &str) -> Option<String> {
    let hex: String =
        s.trim().trim_start_matches("sha256/").trim_start_matches("SHA256:").chars().filter(|c| *c != ':').collect();
    (hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit())).then(|| hex.to_ascii_lowercase())
}

// ---------------------------------------------------------------- server side

/// The server's certificate and the settings that go with it.
pub struct ServerTls {
    config: Arc<ServerConfig>,
    pub fingerprint: String,
    pub cert_path: PathBuf,
}

/// Loads the configured certificate, or creates one in `<home>/tls/`.
pub fn load_server(home: &Path, cfg: &Config) -> Result<ServerTls> {
    let (cert, key, hosts) = (cfg.get("tls", "cert"), cfg.get("tls", "key"), cfg.get("tls", "hosts"));
    let (cert_path, key_path) = if cert.trim().is_empty() {
        let dir = home.join("tls");
        let (c, k) = (dir.join("cert.pem"), dir.join("key.pem"));
        if !c.exists() || !k.exists() {
            generate(&dir, &c, &k, &hosts)?;
        }
        (c, k)
    } else {
        if key.trim().is_empty() {
            return Err(KivaError::Invalid("tls.cert is set but tls.key is not".into()));
        }
        (PathBuf::from(cert.trim()), PathBuf::from(key.trim()))
    };
    let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(&cert_path)
        .map_err(|e| tls_err(format!("cannot read {}: {e}", cert_path.display())))?
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| tls_err(format!("bad certificate in {}: {e}", cert_path.display())))?;
    let first = chain.first().ok_or_else(|| tls_err(format!("no certificate in {}", cert_path.display())))?;
    let fingerprint = fingerprint(first);
    let key = PrivateKeyDer::from_pem_file(&key_path)
        .map_err(|e| tls_err(format!("cannot read the key {}: {e}", key_path.display())))?;
    let config = ServerConfig::builder_with_provider(crypto())
        .with_safe_default_protocol_versions()
        .map_err(tls_err)?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .map_err(tls_err)?;
    Ok(ServerTls { config: Arc::new(config), fingerprint, cert_path })
}

/// A self-signed certificate valid for this machine's usual names, plus
/// `extra` (comma-separated names or IP addresses, from `tls.hosts`).
fn generate(dir: &Path, cert: &Path, key: &Path, extra: &str) -> Result<()> {
    let mut names = vec!["localhost".to_string(), "127.0.0.1".to_string(), "::1".to_string()];
    if let Ok(h) = std::env::var("HOSTNAME").or_else(|_| std::env::var("COMPUTERNAME")) {
        names.push(h.to_ascii_lowercase());
    }
    names.extend(extra.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
    names.dedup();
    let key_pair = rcgen::KeyPair::generate().map_err(tls_err)?;
    let mut params = rcgen::CertificateParams::new(names).map_err(tls_err)?;
    params.distinguished_name.push(rcgen::DnType::CommonName, "KivaDB");
    params.not_before = rcgen::date_time_ymd(2025, 1, 1);
    params.not_after = rcgen::date_time_ymd(2125, 1, 1);
    let certificate = params.self_signed(&key_pair).map_err(tls_err)?;
    std::fs::create_dir_all(dir)?;
    std::fs::write(cert, certificate.pem())?;
    write_private(key, key_pair.serialize_pem().as_bytes())?;
    Ok(())
}

/// Writes a file only its owner can read (the private key).
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
        f.write_all(bytes)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes)
    }
}

/// What a listener does with a new connection: `Some((stream, peer))` to
/// serve, `None` when it was refused (the reason was written to the client).
///
/// `refusal` gives, from the client's first byte, the protocol's way of
/// saying "encryption required".
pub fn accept(
    inst: &Arc<Instance>,
    stream: TcpStream,
    refusal: &dyn Fn(u8) -> Vec<u8>,
) -> Option<(TcpStream, SocketAddr)> {
    let peer = stream.peer_addr().ok()?;
    let mut first = [0u8; 1];
    // A client that connects and says nothing for a minute is let go.
    let _ = stream.set_read_timeout(Some(Duration::from_secs(60)));
    match stream.peek(&mut first) {
        Ok(1) => {}
        _ => return None,
    }
    let _ = stream.set_read_timeout(None);
    if first[0] != HANDSHAKE {
        let required = inst.cfg().get_bool("tls", "required");
        if required && !peer.ip().is_loopback() {
            let mut s = stream;
            let _ = s.write_all(&refusal(first[0]));
            let _ = s.flush();
            // Closing with the client's request still unread would reset the
            // connection and lose the message: read it first, briefly.
            let _ = s.shutdown(Shutdown::Write);
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            let mut sink = [0u8; 4096];
            while matches!(s.read(&mut sink), Ok(n) if n > 0) {}
            return None;
        }
        return Some((stream, peer));
    }
    let Some(tls) = inst.tls() else {
        // Encryption is off: nothing sensible can be answered to a handshake.
        return None;
    };
    let conn = rustls::ServerConnection::new(tls.config.clone()).ok()?;
    let (inner, outer) = loopback_pair().ok()?;
    relay(Connection::Server(conn), stream, outer);
    Some((inner, peer))
}

// ---------------------------------------------------------------- client side

/// Checks the server's certificate by its fingerprint. With no fingerprint to
/// expect, it accepts the certificate and records it, so that the caller can
/// show it and ask whether to trust it (what SSH does on a first connection).
#[derive(Debug)]
struct Pinned {
    expected: Option<String>,
    seen: Mutex<Option<String>>,
    algorithms: rustls::crypto::WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let seen = fingerprint(end_entity);
        *self.seen.lock().unwrap_or_else(|e| e.into_inner()) = Some(seen.clone());
        match &self.expected {
            Some(want) if normalize_fingerprint(&seen).as_deref() != Some(want.as_str()) => {
                Err(rustls::Error::General(format!("certificate fingerprint {seen} is not the expected one")))
            }
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// An encrypted connection to a KivaDB server, used like a plain one: the
/// returned stream carries clear bytes, the relay encrypts them.
pub struct ClientStream {
    pub stream: TcpStream,
    /// The server's certificate fingerprint (`AB:CD:...`).
    pub fingerprint: String,
}

/// Connects with TLS to `addr`. With `expected`, the server's certificate must
/// have that fingerprint; without it, any certificate is accepted and its
/// fingerprint returned for the caller to check.
pub fn connect(addr: &str, expected: Option<&str>, timeout: Duration) -> Result<ClientStream> {
    let expected = match expected {
        Some(f) => Some(
            normalize_fingerprint(f)
                .ok_or_else(|| KivaError::Invalid(format!("'{f}' is not a SHA-256 fingerprint")))?,
        ),
        None => None,
    };
    let host = addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(addr).trim_matches(['[', ']']).to_string();
    let sock = addr.to_socket_addrs()?.next().ok_or_else(|| KivaError::Invalid(format!("cannot resolve '{addr}'")))?;
    let remote = TcpStream::connect_timeout(&sock, timeout)?;
    let _ = remote.set_nodelay(true);
    let provider = crypto();
    let verifier =
        Arc::new(Pinned { expected, seen: Mutex::new(None), algorithms: provider.signature_verification_algorithms });
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(tls_err)?
        .dangerous()
        .with_custom_certificate_verifier(verifier.clone())
        .with_no_client_auth();
    let name =
        ServerName::try_from(host.clone()).unwrap_or_else(|_| ServerName::try_from("localhost").expect("valid name"));
    let mut conn = rustls::ClientConnection::new(Arc::new(config), name.to_owned()).map_err(tls_err)?;
    // Finish the handshake here, so that a wrong certificate is an error now
    // and not a closed connection later.
    let mut r = remote.try_clone()?;
    let _ = r.set_read_timeout(Some(timeout));
    while conn.is_handshaking() {
        conn.complete_io(&mut r).map_err(|e| {
            match verifier.seen.lock().unwrap_or_else(|e| e.into_inner()).clone() {
                Some(seen) if e.to_string().contains("fingerprint") => KivaError::Denied(format!(
                    "the server's certificate ({seen}) is not the one expected; connection refused"
                )),
                _ => tls_err(e),
            }
        })?;
    }
    let _ = r.set_read_timeout(None);
    let fingerprint = verifier.seen.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default();
    let (inner, outer) = loopback_pair()?;
    relay(Connection::Client(conn), remote, outer);
    Ok(ClientStream { stream: inner, fingerprint })
}

// ---------------------------------------------------------------- known servers

/// `<home>/known_servers`: one `host:port fingerprint` per line, written when
/// the user trusts a server, read on the next connections.
pub fn known_fingerprint(home: &Path, addr: &str) -> Option<String> {
    std::fs::read_to_string(home.join("known_servers")).ok()?.lines().find_map(|l| {
        let (a, f) = l.split_once(' ')?;
        (a == addr).then(|| f.trim().to_string())
    })
}

pub fn remember_fingerprint(home: &Path, addr: &str, fp: &str) -> io::Result<()> {
    let path = home.join("known_servers");
    let mut lines: Vec<String> = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.split_once(' ').map(|(a, _)| a) != Some(addr))
        .map(str::to_string)
        .collect();
    lines.push(format!("{addr} {fp}"));
    std::fs::create_dir_all(home)?;
    std::fs::write(path, lines.join("\n") + "\n")
}

// ---------------------------------------------------------------- the relay

/// Two connected loopback sockets. The second one is checked to be the first
/// one's peer: another program racing to connect cannot slip in.
fn loopback_pair() -> io::Result<(TcpStream, TcpStream)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let a = TcpStream::connect(listener.local_addr()?)?;
    loop {
        let (b, from) = listener.accept()?;
        if from == a.local_addr()? {
            let _ = a.set_nodelay(true);
            let _ = b.set_nodelay(true);
            return Ok((a, b));
        }
    }
}

fn lock(conn: &Mutex<Connection>) -> std::sync::MutexGuard<'_, Connection> {
    conn.lock().unwrap_or_else(|e| e.into_inner())
}

/// Writes every pending TLS record of `conn` to `remote`.
fn flush(conn: &mut Connection, remote: &mut TcpStream) -> io::Result<()> {
    while conn.wants_write() {
        conn.write_tls(remote)?;
    }
    remote.flush()
}

/// Moves bytes between the encrypted socket `remote` and the plain loopback
/// socket `plain`, in both directions, until either side closes.
fn relay(conn: Connection, remote: TcpStream, plain: TcpStream) {
    let conn = Arc::new(Mutex::new(conn));
    let clones = (|| Ok::<_, io::Error>((remote.try_clone()?, remote.try_clone()?, plain.try_clone()?)))();
    let Ok((mut remote_in, mut remote_out_a, mut plain_in)) = clones else { return };
    let (mut remote_out_b, mut plain_out) = (remote, plain);
    // A client connection speaks first: send its hello.
    let _ = flush(&mut lock(&conn), &mut remote_out_a);

    // Encrypted -> clear.
    let c = conn.clone();
    let _ = std::thread::Builder::new().name("kiva-tls-in".into()).spawn(move || {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = match remote_in.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let mut clear = Vec::new();
            let mut closed = false;
            {
                let mut g = lock(&c);
                let mut rd = &buf[..n];
                let mut failed = false;
                // Record by record: rustls takes no more ciphertext while the
                // clear text it already decrypted has not been read.
                while !rd.is_empty() && !closed {
                    if g.read_tls(&mut rd).is_err() || g.process_new_packets().is_err() {
                        failed = true;
                        break;
                    }
                    match g.reader().read_to_end(&mut clear) {
                        Ok(_) => closed = true, // close_notify received
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                        Err(_) => closed = true,
                    }
                }
                if failed {
                    // Tell the other side why (an alert), then stop.
                    let _ = flush(&mut g, &mut remote_out_a);
                    break;
                }
                if flush(&mut g, &mut remote_out_a).is_err() {
                    break;
                }
            }
            if !clear.is_empty() && plain_in.write_all(&clear).is_err() {
                break;
            }
            if closed {
                break;
            }
        }
        let _ = plain_in.shutdown(Shutdown::Write);
    });

    // Clear -> encrypted.
    let _ = std::thread::Builder::new().name("kiva-tls-out".into()).spawn(move || {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = match plain_out.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let mut g = lock(&conn);
            if g.writer().write_all(&buf[..n]).is_err() || flush(&mut g, &mut remote_out_b).is_err() {
                break;
            }
        }
        let mut g = lock(&conn);
        g.send_close_notify();
        let _ = flush(&mut g, &mut remote_out_b);
        let _ = remote_out_b.shutdown(Shutdown::Both);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_are_compared_whatever_their_spelling() {
        let fp = fingerprint(b"certificate");
        assert_eq!(fp.len(), 32 * 3 - 1);
        let n = normalize_fingerprint(&fp).unwrap();
        assert_eq!(normalize_fingerprint(&n), Some(n.clone()));
        assert_eq!(normalize_fingerprint(&fp.to_lowercase()), Some(n.clone()));
        assert_eq!(normalize_fingerprint(&format!("SHA256:{fp}")), Some(n));
        assert_eq!(normalize_fingerprint("AB:CD"), None);
        assert_eq!(normalize_fingerprint(&"zz".repeat(32)), None);
    }

    #[test]
    fn known_servers_are_remembered_one_line_each() {
        let dir = std::env::temp_dir().join(format!("kiva-known-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(known_fingerprint(&dir, "db:2911"), None);
        remember_fingerprint(&dir, "db:2911", "AA").unwrap();
        remember_fingerprint(&dir, "other:2911", "BB").unwrap();
        remember_fingerprint(&dir, "db:2911", "CC").unwrap();
        assert_eq!(known_fingerprint(&dir, "db:2911").as_deref(), Some("CC"));
        assert_eq!(known_fingerprint(&dir, "other:2911").as_deref(), Some("BB"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
