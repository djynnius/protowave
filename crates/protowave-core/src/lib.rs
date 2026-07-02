//! ProtoWave core types.
//!
//! Federation-qualified identifiers (PRD §8.2) and ed25519 server signing
//! primitives (PRD §8.3). Every ID carries its domain so that nothing in the
//! system has to be retrofitted when federation ships in Phase 3.

mod id;
mod signing;

pub use id::{IdError, ParticipantId, WaveId, WaveletId, WaveletName};
pub use signing::{ServerKeypair, ServerPublicKey, Signature, SigningError};
