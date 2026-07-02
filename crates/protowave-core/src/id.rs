//! Federation-qualified identifiers.
//!
//! Successors of legacy Wave's `ParticipantId`, `WaveId`, `WaveletId` and
//! `WaveletName` (see `legacy/apache-wave`,
//! `org.waveprotocol.wave.model.id`). Serialized forms:
//!
//! - participant: `local@domain`
//! - wave:        `domain/wave-id`
//! - wavelet name (globally unique wavelet): `domain/wave-id/wavelet-id`

use std::fmt;
use std::str::FromStr;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IdError {
    #[error("empty identifier component")]
    Empty,
    #[error("invalid character {0:?} in identifier component")]
    InvalidChar(char),
    #[error("malformed identifier: expected {expected}, got {got:?}")]
    Malformed { expected: &'static str, got: String },
    #[error("invalid domain: {0:?}")]
    InvalidDomain(String),
}

/// Characters allowed in local parts and wave/wavelet id tokens.
fn validate_token(s: &str) -> Result<(), IdError> {
    if s.is_empty() {
        return Err(IdError::Empty);
    }
    for c in s.chars() {
        if !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+' | '~')) {
            return Err(IdError::InvalidChar(c));
        }
    }
    Ok(())
}

/// DNS-name-shaped domain: dot-separated labels of alphanumerics and hyphens.
fn validate_domain(s: &str) -> Result<(), IdError> {
    let valid = !s.is_empty()
        && s.len() <= 253
        && s.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    if valid {
        Ok(())
    } else {
        Err(IdError::InvalidDomain(s.to_string()))
    }
}

/// A user or agent address: `local@domain`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ParticipantId {
    local: String,
    domain: String,
}

impl ParticipantId {
    pub fn new(local: &str, domain: &str) -> Result<Self, IdError> {
        validate_token(local)?;
        validate_domain(domain)?;
        Ok(Self {
            local: local.to_ascii_lowercase(),
            domain: domain.to_ascii_lowercase(),
        })
    }

    pub fn local(&self) -> &str {
        &self.local
    }

    pub fn domain(&self) -> &str {
        &self.domain
    }
}

impl FromStr for ParticipantId {
    type Err = IdError;

    fn from_str(s: &str) -> Result<Self, IdError> {
        match s.split_once('@') {
            Some((local, domain)) if !domain.contains('@') => Self::new(local, domain),
            _ => Err(IdError::Malformed {
                expected: "local@domain",
                got: s.to_string(),
            }),
        }
    }
}

impl fmt::Display for ParticipantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.local, self.domain)
    }
}

/// A wave identifier: `domain/wave-id`. The domain is the wave's origin
/// (home) server.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WaveId {
    domain: String,
    id: String,
}

impl WaveId {
    pub fn new(domain: &str, id: &str) -> Result<Self, IdError> {
        validate_domain(domain)?;
        validate_token(id)?;
        Ok(Self {
            domain: domain.to_ascii_lowercase(),
            id: id.to_string(),
        })
    }

    pub fn domain(&self) -> &str {
        &self.domain
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

impl FromStr for WaveId {
    type Err = IdError;

    fn from_str(s: &str) -> Result<Self, IdError> {
        match s.split_once('/') {
            Some((domain, id)) if !id.contains('/') => Self::new(domain, id),
            _ => Err(IdError::Malformed {
                expected: "domain/wave-id",
                got: s.to_string(),
            }),
        }
    }
}

impl fmt::Display for WaveId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.domain, self.id)
    }
}

/// A wavelet identifier: `domain/wavelet-id`. The domain is the wavelet's
/// home server, which holds control-plane authority (PRD §8.3); it may differ
/// from the wave's domain (e.g. a private reply wavelet created remotely).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WaveletId {
    domain: String,
    id: String,
}

impl WaveletId {
    pub fn new(domain: &str, id: &str) -> Result<Self, IdError> {
        validate_domain(domain)?;
        validate_token(id)?;
        Ok(Self {
            domain: domain.to_ascii_lowercase(),
            id: id.to_string(),
        })
    }

    pub fn domain(&self) -> &str {
        &self.domain
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

impl fmt::Display for WaveletId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.domain, self.id)
    }
}

/// The globally unique name of a wavelet: a (wave, wavelet) pair.
/// Serialized as `wave-domain/wave-id/wavelet-id` when both share a domain,
/// or `wave-domain/wave-id/wavelet-domain/wavelet-id` otherwise — the same
/// scheme as legacy `ModernIdSerialiser`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WaveletName {
    pub wave_id: WaveId,
    pub wavelet_id: WaveletId,
}

impl WaveletName {
    pub fn new(wave_id: WaveId, wavelet_id: WaveletId) -> Self {
        Self {
            wave_id,
            wavelet_id,
        }
    }
}

impl FromStr for WaveletName {
    type Err = IdError;

    fn from_str(s: &str) -> Result<Self, IdError> {
        let parts: Vec<&str> = s.split('/').collect();
        match parts.as_slice() {
            [wd, wid, lid] => Ok(Self::new(WaveId::new(wd, wid)?, WaveletId::new(wd, lid)?)),
            [wd, wid, ld, lid] => Ok(Self::new(WaveId::new(wd, wid)?, WaveletId::new(ld, lid)?)),
            _ => Err(IdError::Malformed {
                expected: "wave-domain/wave-id[/wavelet-domain]/wavelet-id",
                got: s.to_string(),
            }),
        }
    }
}

impl fmt::Display for WaveletName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.wave_id.domain() == self.wavelet_id.domain() {
            write!(f, "{}/{}", self.wave_id, self.wavelet_id.id())
        } else {
            write!(f, "{}/{}", self.wave_id, self.wavelet_id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn participant_roundtrip_and_lowercasing() {
        let p: ParticipantId = "Ada.Lovelace@Example.ORG".parse().unwrap();
        assert_eq!(p.local(), "ada.lovelace");
        assert_eq!(p.domain(), "example.org");
        assert_eq!(p.to_string(), "ada.lovelace@example.org");
    }

    #[test]
    fn participant_rejects_malformed() {
        assert!("no-at-sign".parse::<ParticipantId>().is_err());
        assert!("two@at@signs".parse::<ParticipantId>().is_err());
        assert!("@example.org".parse::<ParticipantId>().is_err());
        assert!("user@".parse::<ParticipantId>().is_err());
        assert!("us er@example.org".parse::<ParticipantId>().is_err());
        assert!("user@-bad-.org".parse::<ParticipantId>().is_err());
    }

    #[test]
    fn wave_id_roundtrip() {
        let w: WaveId = "example.org/w+abc123".parse().unwrap();
        assert_eq!(w.domain(), "example.org");
        assert_eq!(w.id(), "w+abc123");
        assert_eq!(w.to_string(), "example.org/w+abc123");
        assert!("example.org/a/b".parse::<WaveId>().is_err());
        assert!("no-slash".parse::<WaveId>().is_err());
    }

    #[test]
    fn wavelet_name_same_domain_compact_form() {
        let n: WaveletName = "example.org/w+1/conv+root".parse().unwrap();
        assert_eq!(n.wave_id.to_string(), "example.org/w+1");
        assert_eq!(n.wavelet_id.to_string(), "example.org/conv+root");
        assert_eq!(n.to_string(), "example.org/w+1/conv+root");
    }

    #[test]
    fn wavelet_name_cross_domain_form() {
        let n: WaveletName = "a.org/w+1/b.org/user+data".parse().unwrap();
        assert_eq!(n.wave_id.domain(), "a.org");
        assert_eq!(n.wavelet_id.domain(), "b.org");
        assert_eq!(n.to_string(), "a.org/w+1/b.org/user+data");
        let back: WaveletName = n.to_string().parse().unwrap();
        assert_eq!(back, n);
    }
}
