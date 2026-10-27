// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Encryption at rest: AES-256-GCM over every record, the hint file and the
//! HNSW graphs file.
//!
//! The master key is 32 random bytes, given as 64 hexadecimal characters in
//! `KIVADB_ENCRYPTION_KEY` or in the file named by `encryption.key_file`. It
//! never touches the data directory. From it come, through HKDF-SHA256, the
//! key that encrypts the data and a 4-byte key id stored next to every
//! encrypted piece, so that a record says which key it needs.
//!
//! An encrypted record keeps the usual 28-byte header in clear (sizes, type,
//! flags, timestamps) with `FLAG_ENCRYPTED` set; its key and value are
//! encrypted together and followed by a trailer:
//!
//! `header | enc(key) | enc(value) | tag (16) | nonce (12) | key id (4)`
//!
//! `k_size` is the length of the key, `v_size` that of the stored value plus
//! the 32-byte trailer. The header and the key id are authenticated: changing
//! a byte anywhere makes the record fail to decrypt. Each encryption uses a
//! random 96-bit nonce.

use crate::config::Config;
use crate::error::{KivaError, Result};
use crate::format::{parse_header, FLAG_ENCRYPTED, REC_HEADER_LEN};
use ring::aead::{Aad, LessSafeKey, Nonce, Tag, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};
use std::path::Path;
use std::sync::Arc;

/// Bytes an encryption adds: tag, nonce and key id.
pub const TRAILER: usize = 16 + NONCE_LEN + 4;

const TAG_LEN: usize = 16;

/// The environment variable holding the master key.
pub const KEY_ENV: &str = "KIVADB_ENCRYPTION_KEY";
/// The previous master key, still accepted for reading during a rotation.
pub const OLD_KEY_ENV: &str = "KIVADB_ENCRYPTION_OLD_KEY";

/// What is being encrypted, mixed into the authenticated data so that a
/// piece of one kind can never be passed off as another.
#[derive(Clone, Copy)]
pub enum Domain {
    Record,
    Hint,
    Graphs,
}

impl Domain {
    fn label(self) -> &'static [u8] {
        match self {
            Domain::Record => b"kivadb record v1",
            Domain::Hint => b"kivadb hint v1",
            Domain::Graphs => b"kivadb graphs v1",
        }
    }
}

/// One master key, ready to use.
pub struct Key {
    pub id: [u8; 4],
    aead: LessSafeKey,
}

struct Len(usize);

impl ring::hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

impl Key {
    fn derive(master: &[u8; 32]) -> Key {
        let prk = ring::hkdf::Salt::new(ring::hkdf::HKDF_SHA256, b"kivadb encryption at rest").extract(master);
        let mut data = [0u8; 32];
        let mut id = [0u8; 4];
        prk.expand(&[b"data key"], Len(32)).and_then(|okm| okm.fill(&mut data)).expect("HKDF output of 32 bytes");
        prk.expand(&[b"key id"], Len(4)).and_then(|okm| okm.fill(&mut id)).expect("HKDF output of 4 bytes");
        let aead = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &data).expect("AES-256 takes 32 bytes"));
        Key { id, aead }
    }

    pub fn id_hex(&self) -> String {
        self.id.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// The keys of an instance: the one that encrypts, and older ones that may
/// still be needed to read data written before a rotation.
pub struct Keys {
    pub current: Key,
    old: Vec<Key>,
    rng: SystemRandom,
}

impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print key material.
        write!(f, "Keys(id {})", self.current.id_hex())
    }
}

/// Why something encrypted could not be read.
#[derive(Debug, PartialEq, Eq)]
pub enum OpenError {
    /// Encrypted, and this instance has no key.
    NoKey,
    /// Encrypted with a key this instance does not have.
    UnknownKey([u8; 4]),
    /// The right key, but the bytes were changed (or are damaged).
    Tampered,
}

impl OpenError {
    pub fn into_error(self, what: &str) -> KivaError {
        match self {
            OpenError::NoKey => KivaError::Denied(format!(
                "{what} is encrypted: start KivaDB with its key in {KEY_ENV} or encryption.key_file"
            )),
            OpenError::UnknownKey(id) => KivaError::Denied(format!(
                "{what} is encrypted with key {}, which this server does not have (current key: see 'encryption')",
                id.iter().map(|b| format!("{b:02x}")).collect::<String>()
            )),
            OpenError::Tampered => {
                KivaError::Corrupted(format!("{what} fails authentication: it was modified or damaged"))
            }
        }
    }
}

/// Parses a master key: 64 hexadecimal characters, spaces around ignored.
pub fn parse_key(text: &str) -> Result<[u8; 32]> {
    let t = text.trim();
    if t.len() != 64 || !t.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(KivaError::Invalid(
            "an encryption key is 64 hexadecimal characters (32 bytes); create one with 'kivadb keygen'".into(),
        ));
    }
    let mut key = [0u8; 32];
    for (i, k) in key.iter_mut().enumerate() {
        *k = u8::from_str_radix(&t[i * 2..i * 2 + 2], 16).expect("checked hex");
    }
    Ok(key)
}

/// A new random master key, as 64 hexadecimal characters.
pub fn generate_key() -> String {
    let mut key = [0u8; 32];
    SystemRandom::new().fill(&mut key).expect("the system random generator works");
    key.iter().map(|b| format!("{b:02x}")).collect()
}

fn read_key_file(path: &str, setting: &str) -> Result<[u8; 32]> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| KivaError::Invalid(format!("{setting}: cannot read the key file '{path}': {e}")))?;
    parse_key(&text).map_err(|e| KivaError::Invalid(format!("{setting} ('{path}'): {e}")))
}

impl Keys {
    pub fn new(current: &[u8; 32], old: &[[u8; 32]]) -> Keys {
        Keys { current: Key::derive(current), old: old.iter().map(Key::derive).collect(), rng: SystemRandom::new() }
    }

    /// The identifier of the key new records are sealed with.
    pub fn id_hex(&self) -> String {
        self.current.id_hex()
    }

    /// The keys configured for an instance, from the environment first, then
    /// from the key files named in `kiva.conf`. `None` when encryption is off;
    /// an error when it is required but no key is given, or a key is unusable.
    pub fn load(cfg: &Config) -> Result<Option<Arc<Keys>>> {
        let env = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let current = match env(KEY_ENV) {
            Some(v) => Some(parse_key(&v).map_err(|e| KivaError::Invalid(format!("{KEY_ENV}: {e}")))?),
            None => match cfg.get("encryption", "key_file") {
                f if f.is_empty() => None,
                f => Some(read_key_file(&f, "encryption.key_file")?),
            },
        };
        let old = match env(OLD_KEY_ENV) {
            Some(v) => Some(parse_key(&v).map_err(|e| KivaError::Invalid(format!("{OLD_KEY_ENV}: {e}")))?),
            None => match cfg.get("encryption", "old_key_file") {
                f if f.is_empty() => None,
                f => Some(read_key_file(&f, "encryption.old_key_file")?),
            },
        };
        match current {
            Some(k) => Ok(Some(Arc::new(Keys::new(&k, old.as_slice())))),
            None if cfg.get_bool("encryption", "required") => Err(KivaError::Denied(format!(
                "encryption.required is on but no key is given: set {KEY_ENV} or encryption.key_file"
            ))),
            None => Ok(None),
        }
    }

    fn find(&self, id: [u8; 4]) -> Option<&Key> {
        std::iter::once(&self.current).chain(&self.old).find(|k| k.id == id)
    }

    fn nonce(&self) -> [u8; NONCE_LEN] {
        let mut n = [0u8; NONCE_LEN];
        self.rng.fill(&mut n).expect("the system random generator works");
        n
    }

    fn aad(domain: Domain, extra: &[u8], id: [u8; 4]) -> Vec<u8> {
        let mut aad = domain.label().to_vec();
        aad.extend_from_slice(extra);
        aad.extend_from_slice(&id);
        aad
    }

    /// Encrypts `data` in place with the current key and appends the trailer.
    pub fn seal(&self, domain: Domain, extra: &[u8], data: &mut Vec<u8>) {
        let key = &self.current;
        let nonce = self.nonce();
        let tag = key
            .aead
            .seal_in_place_separate_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(Self::aad(domain, extra, key.id)),
                data,
            )
            .expect("AES-GCM encrypts up to 64 GB per message");
        data.extend_from_slice(tag.as_ref());
        data.extend_from_slice(&nonce);
        data.extend_from_slice(&key.id);
    }

    /// Decrypts what [`Keys::seal`] produced; returns the plaintext.
    pub fn open(&self, domain: Domain, extra: &[u8], sealed: &[u8]) -> std::result::Result<Vec<u8>, OpenError> {
        open_with(Some(self), domain, extra, sealed)
    }
}

/// The key id of something sealed, if it is long enough to have one.
pub fn sealed_key_id(sealed: &[u8]) -> Option<[u8; 4]> {
    sealed.len().checked_sub(4).map(|i| sealed[i..].try_into().expect("4 bytes"))
}

/// Like [`Keys::open`], with the case of an instance without keys.
pub fn open_with(
    keys: Option<&Keys>,
    domain: Domain,
    extra: &[u8],
    sealed: &[u8],
) -> std::result::Result<Vec<u8>, OpenError> {
    if sealed.len() < TRAILER {
        return Err(OpenError::Tampered);
    }
    let keys = keys.ok_or(OpenError::NoKey)?;
    let n = sealed.len() - TRAILER;
    let id: [u8; 4] = sealed[n + TAG_LEN + NONCE_LEN..].try_into().expect("4 bytes");
    let key = keys.find(id).ok_or(OpenError::UnknownKey(id))?;
    let nonce: [u8; NONCE_LEN] = sealed[n + TAG_LEN..n + TAG_LEN + NONCE_LEN].try_into().expect("12 bytes");
    let tag: [u8; TAG_LEN] = sealed[n..n + TAG_LEN].try_into().expect("16 bytes");
    let mut data = sealed[..n].to_vec();
    key.aead
        .open_in_place_separate_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(Keys::aad(domain, extra, id)),
            Tag::from(tag),
            &mut data,
            0..,
        )
        .map_err(|_| OpenError::Tampered)?;
    Ok(data)
}

// -- records ------------------------------------------------------------------

/// Encodes a record, encrypted when `keys` is given.
pub fn encode_record(
    keys: Option<&Keys>,
    key: &str,
    value: &[u8],
    ty: u8,
    flags: u8,
    timestamp: i64,
    expires_at: i64,
) -> Vec<u8> {
    let Some(keys) = keys else {
        return crate::format::encode_record(key, value, ty, flags, timestamp, expires_at);
    };
    let k = key.as_bytes();
    let flags = flags | FLAG_ENCRYPTED;
    let mut buf = Vec::with_capacity(REC_HEADER_LEN + k.len() + value.len() + TRAILER);
    buf.extend_from_slice(&[0u8; 4]);
    buf.extend_from_slice(&(k.len() as u16).to_le_bytes());
    buf.extend_from_slice(&((value.len() + TRAILER) as u32).to_le_bytes());
    buf.push(ty);
    buf.push(flags);
    buf.extend_from_slice(&timestamp.to_le_bytes());
    buf.extend_from_slice(&expires_at.to_le_bytes());
    let mut body = Vec::with_capacity(k.len() + value.len() + TRAILER);
    body.extend_from_slice(k);
    body.extend_from_slice(value);
    keys.seal(Domain::Record, &buf[4..REC_HEADER_LEN], &mut body);
    buf.extend_from_slice(&body);
    let crc = crc32fast::hash(&buf[4..]);
    buf[..4].copy_from_slice(&crc.to_le_bytes());
    buf
}

/// The key and stored value of a whole record whose checksum was verified,
/// decrypting it when it is encrypted.
pub fn open_record(keys: Option<&Keys>, rec: &[u8]) -> std::result::Result<(Vec<u8>, Vec<u8>), OpenError> {
    let h = parse_header(rec);
    let k = h.k_size as usize;
    if h.flags & FLAG_ENCRYPTED == 0 {
        return Ok((rec[REC_HEADER_LEN..REC_HEADER_LEN + k].to_vec(), rec[REC_HEADER_LEN + k..].to_vec()));
    }
    let mut plain = open_with(keys, Domain::Record, &rec[4..REC_HEADER_LEN], &rec[REC_HEADER_LEN..])?;
    if plain.len() < k {
        return Err(OpenError::Tampered);
    }
    let value = plain.split_off(k);
    Ok((plain, value))
}

/// The key id of an encrypted record.
pub fn record_key_id(rec: &[u8]) -> Option<[u8; 4]> {
    (parse_header(rec).flags & FLAG_ENCRYPTED != 0).then(|| sealed_key_id(rec)).flatten()
}

/// Reads the master key of `kivadb keygen --file` or a key file, for tests and tools.
pub fn key_from_file(path: &Path) -> Result<[u8; 32]> {
    read_key_file(&path.display().to_string(), "key file")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(seed: u8) -> Keys {
        Keys::new(&[seed; 32], &[])
    }

    #[test]
    fn records_round_trip_and_reject_any_change() {
        let k = keys(1);
        let rec = encode_record(Some(&k), "user:1", b"secret value", 1, 0, 1700, 0);
        assert!(crate::format::verify_record(&rec));
        assert!(!rec.windows(6).any(|w| w == b"user:1"), "the key is not in clear");
        assert!(!rec.windows(6).any(|w| w == b"secret"), "the value is not in clear");
        let (key, value) = open_record(Some(&k), &rec).unwrap();
        assert_eq!((key.as_slice(), value.as_slice()), (b"user:1".as_slice(), b"secret value".as_slice()));
        assert_eq!(record_key_id(&rec), Some(k.current.id));

        // Every byte after the checksum is authenticated, header included.
        for i in 4..rec.len() {
            let mut bad = rec.clone();
            bad[i] ^= 1;
            assert!(open_record(Some(&k), &bad).is_err(), "byte {i} changed without notice");
        }
        assert_eq!(open_record(None, &rec), Err(OpenError::NoKey));
        assert_eq!(open_record(Some(&keys(2)), &rec), Err(OpenError::UnknownKey(k.current.id)));
        // The same plaintext never gives the same bytes twice.
        assert_ne!(rec, encode_record(Some(&k), "user:1", b"secret value", 1, 0, 1700, 0));
    }

    #[test]
    fn an_old_key_still_reads_after_a_rotation() {
        let old = keys(1);
        let rec = encode_record(Some(&old), "a", b"1", 1, 0, 0, 0);
        let rotated = Keys::new(&[2; 32], &[[1; 32]]);
        assert_eq!(open_record(Some(&rotated), &rec).unwrap().1, b"1");
        assert_ne!(rotated.current.id, old.current.id);
    }

    #[test]
    fn plain_records_stay_readable() {
        let rec = crate::format::encode_record("a", b"1", 1, 0, 0, 0);
        assert_eq!(open_record(Some(&keys(1)), &rec).unwrap().1, b"1");
        assert_eq!(record_key_id(&rec), None);
    }

    #[test]
    fn keys_are_64_hex_characters() {
        let k = generate_key();
        assert_eq!(parse_key(&format!("  {k}\n")).unwrap().len(), 32);
        assert!(parse_key("abc").is_err());
        assert!(parse_key(&"g".repeat(64)).is_err());
        assert_ne!(generate_key(), k);
    }

    #[test]
    fn domains_do_not_mix() {
        let k = keys(3);
        let mut data = b"hint body".to_vec();
        k.seal(Domain::Hint, b"id", &mut data);
        assert_eq!(k.open(Domain::Hint, b"id", &data).unwrap(), b"hint body");
        assert_eq!(k.open(Domain::Graphs, b"id", &data), Err(OpenError::Tampered));
        assert_eq!(k.open(Domain::Hint, b"other", &data), Err(OpenError::Tampered));
    }
}
