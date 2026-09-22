//! Encrypts provider API keys at rest with a random per-install key
//! (ChaCha20-Poly1305). The key file lives next to the database; this guards
//! against leaking keys through database backups or copies, not against an
//! attacker who can read the data directory.

use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

use crate::{DbError, Result};

#[derive(Clone)]
pub struct KeyRing {
    cipher: ChaCha20Poly1305,
}

impl KeyRing {
    pub fn load_or_create(path: &Path) -> Result<KeyRing> {
        let key_bytes = match std::fs::read(path) {
            Ok(b) if b.len() == 32 => b,
            Ok(_) => return Err(DbError::Crypto(format!("{} is not a 32-byte key", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let k = ChaCha20Poly1305::generate_key(&mut OsRng);
                std::fs::write(path, k.as_slice())?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
                }
                k.to_vec()
            }
            Err(e) => return Err(e.into()),
        };
        Ok(KeyRing { cipher: ChaCha20Poly1305::new(Key::from_slice(&key_bytes)) })
    }

    pub fn ephemeral() -> KeyRing {
        KeyRing { cipher: ChaCha20Poly1305::new(&ChaCha20Poly1305::generate_key(&mut OsRng)) }
    }

    pub fn encrypt(&self, plain: &str) -> Result<String> {
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ct = self
            .cipher
            .encrypt(&nonce, plain.as_bytes())
            .map_err(|e| DbError::Crypto(e.to_string()))?;
        let mut out = nonce.to_vec();
        out.extend(ct);
        Ok(B64.encode(out))
    }

    pub fn decrypt(&self, enc: &str) -> Result<String> {
        let raw = B64.decode(enc).map_err(|e| DbError::Crypto(e.to_string()))?;
        if raw.len() < 12 {
            return Err(DbError::Crypto("ciphertext too short".into()));
        }
        let (nonce, ct) = raw.split_at(12);
        let pt = self
            .cipher
            .decrypt(Nonce::from_slice(nonce), ct)
            .map_err(|_| DbError::Crypto("cannot decrypt stored key (keyring changed?)".into()))?;
        String::from_utf8(pt).map_err(|e| DbError::Crypto(e.to_string()))
    }
}
