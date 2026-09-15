use crate::error::FaceAuthError;
use crate::user::validate_username;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

const EMBEDDING_VERSION: u32 = 1;
const EMBEDDING_DIM: u32 = 512;

/// Upper bound on stored embeddings per user.
///
/// `count` is read straight off disk and drives an allocation, so it is
/// bounded before use: an unbounded `u32` here asks for ~96 GB and aborts
/// the process. Enrolment adds 5 at a time, so this is generous in practice.
const MAX_EMBEDDINGS: u32 = 256;

/// Biometric templates. Readable only by root — see `deploy.sh`.
const EMBEDDINGS_FILE_MODE: u32 = 0o600;
const EMBEDDINGS_DIR_MODE: u32 = 0o700;

#[derive(Debug, Clone, Default)]
pub struct EmbeddingStore {
    pub embeddings: Vec<Vec<f32>>,
}

/// Build the per-user store path, rejecting anything that would escape
/// `embeddings_dir`. The username reaching here originates from PAM, but it
/// is a path component either way and is checked rather than trusted.
fn user_store_dir(user: &str, embeddings_dir: &Path) -> anyhow::Result<PathBuf> {
    validate_username(user)?;
    Ok(embeddings_dir.join(user))
}

impl EmbeddingStore {
    pub fn load(user: &str, embeddings_dir: &Path) -> anyhow::Result<Self> {
        let path = user_store_dir(user, embeddings_dir)?.join("embeddings.bin");
        if !path.exists() {
            return Err(FaceAuthError::NoEmbeddings.into());
        }

        let file = File::open(&path)?;
        let mut reader = BufReader::new(file);

        let version = reader.read_u32::<LittleEndian>()?;
        if version != EMBEDDING_VERSION {
            return Err(FaceAuthError::InvalidEmbeddingFormat.into());
        }

        let count = reader.read_u32::<LittleEndian>()?;
        let dim = reader.read_u32::<LittleEndian>()?;

        if dim != EMBEDDING_DIM || count > MAX_EMBEDDINGS {
            return Err(FaceAuthError::InvalidEmbeddingFormat.into());
        }

        let mut embeddings = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let mut embedding = vec![0.0f32; EMBEDDING_DIM as usize];
            for val in &mut embedding {
                *val = reader.read_f32::<LittleEndian>()?;
            }
            // A non-finite stored value makes every comparison NaN, which
            // fails closed but silently. Reject it as corruption instead.
            if !embedding.iter().all(|v| v.is_finite()) {
                return Err(FaceAuthError::InvalidEmbeddingFormat.into());
            }
            embeddings.push(embedding);
        }

        // Trailing bytes mean this is not the file we think it is.
        let mut trailing = [0u8; 1];
        if reader.read(&mut trailing)? != 0 {
            return Err(FaceAuthError::InvalidEmbeddingFormat.into());
        }

        Ok(Self { embeddings })
    }

    pub fn save(&self, user: &str, embeddings_dir: &Path) -> anyhow::Result<()> {
        if self.embeddings.len() > MAX_EMBEDDINGS as usize {
            anyhow::bail!(
                "refusing to store {} embeddings (limit {})",
                self.embeddings.len(),
                MAX_EMBEDDINGS
            );
        }

        let user_dir = user_store_dir(user, embeddings_dir)?;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(EMBEDDINGS_DIR_MODE)
            .create(&user_dir)?;

        let tmp_path = user_dir.join("embeddings.bin.tmp");
        let path = user_dir.join("embeddings.bin");

        {
            let file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(EMBEDDINGS_FILE_MODE)
                .open(&tmp_path)?;
            let mut writer = BufWriter::new(file);

            writer.write_u32::<LittleEndian>(EMBEDDING_VERSION)?;
            writer.write_u32::<LittleEndian>(self.embeddings.len() as u32)?;
            writer.write_u32::<LittleEndian>(EMBEDDING_DIM)?;

            for embedding in &self.embeddings {
                anyhow::ensure!(
                    embedding.len() == EMBEDDING_DIM as usize,
                    "embedding has {} dimensions, expected {}",
                    embedding.len(),
                    EMBEDDING_DIM
                );
                for &val in embedding {
                    writer.write_f32::<LittleEndian>(val)?;
                }
            }

            writer.flush()?;
            // Rename alone is atomic but not durable: without this a crash can
            // leave a present-but-empty template file, locking the user out.
            writer.get_ref().sync_all()?;
        }

        fs::rename(&tmp_path, &path)?;

        // Persist the rename itself.
        if let Ok(dir) = File::open(&user_dir) {
            let _ = dir.sync_all();
        }

        Ok(())
    }

    pub fn add_embedding(&mut self, embedding: Vec<f32>) {
        self.embeddings.push(embedding);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "face-auth-test-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample(seed: f32) -> Vec<f32> {
        (0..EMBEDDING_DIM).map(|i| seed + i as f32 * 1e-3).collect()
    }

    #[test]
    fn round_trips() {
        let dir = tmpdir("roundtrip");
        let mut store = EmbeddingStore::default();
        store.add_embedding(sample(0.1));
        store.add_embedding(sample(0.2));
        store.save("alice", &dir).unwrap();

        let loaded = EmbeddingStore::load("alice", &dir).unwrap();
        assert_eq!(loaded.embeddings.len(), 2);
        assert_eq!(loaded.embeddings[1], sample(0.2));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn stores_templates_unreadable_by_others() {
        let dir = tmpdir("perms");
        let mut store = EmbeddingStore::default();
        store.add_embedding(sample(0.1));
        store.save("alice", &dir).unwrap();

        let file_mode = fs::metadata(dir.join("alice/embeddings.bin"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file_mode, EMBEDDINGS_FILE_MODE);

        let dir_mode = fs::metadata(dir.join("alice")).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, EMBEDDINGS_DIR_MODE);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_path_traversal_in_username() {
        let dir = tmpdir("traversal");
        let mut store = EmbeddingStore::default();
        store.add_embedding(sample(0.1));

        assert!(store.save("../escaped", &dir).is_err());
        assert!(store.save("../../etc/shadow", &dir).is_err());
        assert!(EmbeddingStore::load("../escaped", &dir).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_absurd_embedding_count() {
        let dir = tmpdir("count");
        fs::create_dir_all(dir.join("alice")).unwrap();
        let mut buf = Vec::new();
        buf.write_u32::<LittleEndian>(EMBEDDING_VERSION).unwrap();
        buf.write_u32::<LittleEndian>(u32::MAX).unwrap();
        buf.write_u32::<LittleEndian>(EMBEDDING_DIM).unwrap();
        fs::write(dir.join("alice/embeddings.bin"), &buf).unwrap();

        // Must fail on the bound, not by attempting a 96 GB allocation.
        assert!(EmbeddingStore::load("alice", &dir).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_non_finite_values() {
        let dir = tmpdir("nan");
        fs::create_dir_all(dir.join("alice")).unwrap();
        let mut buf = Vec::new();
        buf.write_u32::<LittleEndian>(EMBEDDING_VERSION).unwrap();
        buf.write_u32::<LittleEndian>(1).unwrap();
        buf.write_u32::<LittleEndian>(EMBEDDING_DIM).unwrap();
        for _ in 0..EMBEDDING_DIM {
            buf.write_f32::<LittleEndian>(f32::NAN).unwrap();
        }
        fs::write(dir.join("alice/embeddings.bin"), &buf).unwrap();

        assert!(EmbeddingStore::load("alice", &dir).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_trailing_bytes() {
        let dir = tmpdir("trailing");
        let mut store = EmbeddingStore::default();
        store.add_embedding(sample(0.1));
        store.save("alice", &dir).unwrap();

        let path = dir.join("alice/embeddings.bin");
        let mut data = fs::read(&path).unwrap();
        data.extend_from_slice(b"extra");
        fs::write(&path, data).unwrap();

        assert!(EmbeddingStore::load("alice", &dir).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_store_reports_no_embeddings() {
        let dir = tmpdir("missing");
        let err = EmbeddingStore::load("alice", &dir).unwrap_err();
        assert!(matches!(
            err.downcast_ref::<FaceAuthError>(),
            Some(FaceAuthError::NoEmbeddings)
        ));
        fs::remove_dir_all(&dir).unwrap();
    }
}
