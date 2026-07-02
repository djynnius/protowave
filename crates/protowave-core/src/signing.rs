//! Server identity and signing (PRD §8.3).
//!
//! Every ProtoWave server holds an ed25519 keypair; its public key is
//! published via `/.well-known/protowave`, and every federated update batch
//! is signed with it — the successor of legacy Wave's certificate-based delta
//! signing (`SignatureHandler`, `SignerInfoStore`).

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;

pub const SIGNATURE_LEN: usize = 64;
pub const PUBLIC_KEY_LEN: usize = 32;
const SECRET_KEY_LEN: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum SigningError {
    #[error("invalid key material: {0}")]
    InvalidKey(String),
    #[error("signature verification failed")]
    BadSignature,
    #[error("invalid signature encoding")]
    BadSignatureEncoding,
}

/// A detached ed25519 signature over a byte payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature(pub [u8; SIGNATURE_LEN]);

impl Signature {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SigningError> {
        let arr: [u8; SIGNATURE_LEN] = bytes
            .try_into()
            .map_err(|_| SigningError::BadSignatureEncoding)?;
        Ok(Self(arr))
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

/// A server's public identity key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerPublicKey(VerifyingKey);

impl ServerPublicKey {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SigningError> {
        let arr: [u8; PUBLIC_KEY_LEN] = bytes
            .try_into()
            .map_err(|_| SigningError::InvalidKey("expected 32 bytes".into()))?;
        VerifyingKey::from_bytes(&arr)
            .map(Self)
            .map_err(|e| SigningError::InvalidKey(e.to_string()))
    }

    pub fn to_bytes(&self) -> [u8; PUBLIC_KEY_LEN] {
        self.0.to_bytes()
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.to_bytes())
    }

    pub fn verify(&self, message: &[u8], signature: &Signature) -> Result<(), SigningError> {
        let sig = ed25519_dalek::Signature::from_bytes(&signature.0);
        self.0
            .verify(message, &sig)
            .map_err(|_| SigningError::BadSignature)
    }
}

/// A server's signing keypair.
pub struct ServerKeypair(SigningKey);

impl ServerKeypair {
    pub fn generate() -> Self {
        Self(SigningKey::generate(&mut OsRng))
    }

    pub fn from_secret_bytes(bytes: &[u8]) -> Result<Self, SigningError> {
        let arr: [u8; SECRET_KEY_LEN] = bytes
            .try_into()
            .map_err(|_| SigningError::InvalidKey("expected 32 bytes".into()))?;
        Ok(Self(SigningKey::from_bytes(&arr)))
    }

    pub fn to_secret_bytes(&self) -> [u8; SECRET_KEY_LEN] {
        self.0.to_bytes()
    }

    pub fn public_key(&self) -> ServerPublicKey {
        ServerPublicKey(self.0.verifying_key())
    }

    pub fn sign(&self, message: &[u8]) -> Signature {
        Signature(self.0.sign(message).to_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_roundtrip() {
        let kp = ServerKeypair::generate();
        let msg = b"protowave update batch";
        let sig = kp.sign(msg);
        kp.public_key().verify(msg, &sig).unwrap();
    }

    #[test]
    fn tampered_message_fails() {
        let kp = ServerKeypair::generate();
        let sig = kp.sign(b"original");
        assert!(matches!(
            kp.public_key().verify(b"tampered", &sig),
            Err(SigningError::BadSignature)
        ));
    }

    #[test]
    fn wrong_key_fails() {
        let a = ServerKeypair::generate();
        let b = ServerKeypair::generate();
        let sig = a.sign(b"msg");
        assert!(b.public_key().verify(b"msg", &sig).is_err());
    }

    #[test]
    fn keypair_persists_via_secret_bytes() {
        let kp = ServerKeypair::generate();
        let restored = ServerKeypair::from_secret_bytes(&kp.to_secret_bytes()).unwrap();
        assert_eq!(kp.public_key(), restored.public_key());
        let sig = restored.sign(b"msg");
        kp.public_key().verify(b"msg", &sig).unwrap();
    }
}
