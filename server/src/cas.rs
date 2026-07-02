//! Content-addressed blob store (PRD §7, §11).
//!
//! Attachment bytes keyed by BLAKE3 hex digest. Identical content dedups
//! automatically (FR-36). This same CAS layer later backs P2P folder
//! sharing (Phase 5) — one storage layer for both features.

use std::fs;
use std::io;
use std::path::PathBuf;

pub struct Cas {
    dir: PathBuf,
    fsync: bool,
}

impl Cas {
    pub fn open(dir: impl Into<PathBuf>, fsync: bool) -> io::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        Ok(Self { dir, fsync })
    }

    fn path_for(&self, hash: &str) -> PathBuf {
        // Two-level fanout keeps directories small at scale.
        self.dir.join(&hash[..2]).join(hash)
    }

    /// Store bytes, returning the BLAKE3 hex digest. Writing existing
    /// content is a no-op (dedup).
    pub fn put(&self, bytes: &[u8]) -> io::Result<String> {
        let hash = blake3::hash(bytes).to_hex().to_string();
        let path = self.path_for(&hash);
        if path.exists() {
            return Ok(hash);
        }
        fs::create_dir_all(path.parent().unwrap())?;
        let tmp = path.with_extension("tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            io::Write::write_all(&mut f, bytes)?;
            if self.fsync {
                f.sync_data()?;
            }
        }
        fs::rename(tmp, path)?;
        Ok(hash)
    }

    /// Retrieve bytes, verifying integrity against the hash (defense against
    /// on-disk corruption; O(content), NFR-C9 spirit).
    pub fn get(&self, hash: &str) -> io::Result<Option<Vec<u8>>> {
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(None);
        }
        let bytes = match fs::read(self.path_for(hash)) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        if blake3::hash(&bytes).to_hex().to_string() != hash {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "CAS blob failed integrity check",
            ));
        }
        Ok(Some(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_dedup_and_verify() {
        let dir = tempfile::tempdir().unwrap();
        let cas = Cas::open(dir.path(), false).unwrap();
        let h1 = cas.put(b"hello wave").unwrap();
        let h2 = cas.put(b"hello wave").unwrap();
        assert_eq!(h1, h2);
        assert_eq!(cas.get(&h1).unwrap().unwrap(), b"hello wave");
        assert_eq!(cas.get(&"0".repeat(64)).unwrap(), None);
        assert_eq!(cas.get("not-a-hash").unwrap(), None);
    }
}
