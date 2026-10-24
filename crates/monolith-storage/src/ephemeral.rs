//! The directory of an ephemeral installation's transfers: in memory only
//! (`docs/adr/0014-transfer-staging.md`, "Ephemeral installations").
//!
//! An ephemeral installation writes nothing to disk (S25), so it stages
//! its transfers here, in the same formats and under random keys. A file is
//! the set of byte ranges written to it: a reservation records a length
//! and allocates nothing, and each write allocates its own buffer with
//! fallible allocation, so nothing is ever allocated from a declared size.
//! Everything is gone when the value is dropped.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use zeroize::Zeroizing;

use crate::StorageError;
use crate::dir::{BlobFile, VaultDir};

/// A file in memory: its length and the ranges written to it, by offset.
#[derive(Debug, Default)]
struct File {
    len: u64,
    parts: BTreeMap<u64, Zeroizing<Vec<u8>>>,
}

impl File {
    /// Fills `buf` from `offset`: zero bytes where nothing was written,
    /// later writes over earlier ones.
    fn read(&self, offset: u64, buf: &mut [u8]) -> Result<(), StorageError> {
        let len = u64::try_from(buf.len()).map_err(|_| StorageError::Io)?;
        let end = offset.checked_add(len).ok_or(StorageError::Io)?;
        if end > self.len {
            return Err(StorageError::Io);
        }
        buf.fill(0);
        for (start, bytes) in &self.parts {
            let part_len = u64::try_from(bytes.len()).map_err(|_| StorageError::Io)?;
            let part_end = start.checked_add(part_len).ok_or(StorageError::Io)?;
            let from = (*start).max(offset);
            let to = part_end.min(end);
            if from >= to {
                continue;
            }
            let source =
                usize::try_from(from.saturating_sub(*start)).map_err(|_| StorageError::Io)?;
            let target =
                usize::try_from(from.saturating_sub(offset)).map_err(|_| StorageError::Io)?;
            let count = usize::try_from(to.saturating_sub(from)).map_err(|_| StorageError::Io)?;
            let source_end = source.checked_add(count).ok_or(StorageError::Io)?;
            let target_end = target.checked_add(count).ok_or(StorageError::Io)?;
            buf.get_mut(target..target_end)
                .ok_or(StorageError::Io)?
                .copy_from_slice(bytes.get(source..source_end).ok_or(StorageError::Io)?);
        }
        Ok(())
    }

    /// Writes `bytes` at `offset`. A range written again replaces what is
    /// there; ranges written partly over each other are kept apart and
    /// read in the order of their offsets.
    fn write(&mut self, offset: u64, bytes: &[u8]) -> Result<(), StorageError> {
        let mut copy = Zeroizing::new(Vec::new());
        copy.try_reserve_exact(bytes.len())
            .map_err(|_| StorageError::Full)?;
        copy.extend_from_slice(bytes);
        let len = u64::try_from(bytes.len()).map_err(|_| StorageError::TooLarge)?;
        let end = offset.checked_add(len).ok_or(StorageError::TooLarge)?;
        // Ranges this write covers whole are dropped; their bytes are
        // written over.
        let covered: Vec<u64> = self
            .parts
            .range(offset..end)
            .filter(|(start, part)| {
                u64::try_from(part.len())
                    .ok()
                    .and_then(|part_len| start.checked_add(part_len))
                    .is_some_and(|part_end| part_end <= end)
            })
            .map(|(start, _)| *start)
            .collect();
        for start in covered {
            self.parts.remove(&start);
        }
        self.parts.insert(offset, copy);
        self.len = self.len.max(end);
        Ok(())
    }

    fn whole(&self) -> Result<Vec<u8>, StorageError> {
        let mut bytes = vec![0; usize::try_from(self.len).map_err(|_| StorageError::TooLarge)?];
        self.read(0, &mut bytes)?;
        Ok(bytes)
    }
}

/// An in-memory directory for the transfers of an ephemeral installation.
/// Clones share the same directory.
#[derive(Clone, Debug, Default)]
pub struct EphemeralDir {
    files: Arc<Mutex<HashMap<String, Arc<Mutex<File>>>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every change is made in one statement, so a panic leaves a
    // consistent state behind.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl EphemeralDir {
    /// An empty directory.
    pub fn new() -> Self {
        Self::default()
    }

    fn file(&self, name: &str) -> Option<Arc<Mutex<File>>> {
        lock(&self.files).get(name).map(Arc::clone)
    }
}

/// A [`BlobFile`] of an [`EphemeralDir`].
#[derive(Debug)]
struct EphemeralBlob(Arc<Mutex<File>>);

impl BlobFile for EphemeralBlob {
    fn length(&self) -> Result<u64, StorageError> {
        Ok(lock(&self.0).len)
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), StorageError> {
        lock(&self.0).read(offset, buf)
    }

    fn write_at(&mut self, offset: u64, bytes: &[u8]) -> Result<(), StorageError> {
        lock(&self.0).write(offset, bytes)
    }

    fn sync(&mut self) -> Result<(), StorageError> {
        Ok(())
    }
}

impl VaultDir for EphemeralDir {
    fn read(&self, name: &str, max: u64) -> Result<Option<Vec<u8>>, StorageError> {
        let Some(file) = self.file(name) else {
            return Ok(None);
        };
        let file = lock(&file);
        if file.len > max {
            return Err(StorageError::TooLarge);
        }
        file.whole().map(Some)
    }

    fn write_new(&self, name: &str, bytes: &[u8]) -> Result<(), StorageError> {
        let mut file = File::default();
        file.write(0, bytes)?;
        let mut files = lock(&self.files);
        if files.contains_key(name) {
            return Err(StorageError::Exists);
        }
        files.insert(name.to_owned(), Arc::new(Mutex::new(file)));
        Ok(())
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), StorageError> {
        let mut files = lock(&self.files);
        let file = files.remove(from).ok_or(StorageError::Io)?;
        files.insert(to.to_owned(), file);
        Ok(())
    }

    fn link(&self, from: &str, to: &str) -> Result<(), StorageError> {
        let mut files = lock(&self.files);
        if files.contains_key(to) {
            return Err(StorageError::Exists);
        }
        let file = Arc::clone(files.get(from).ok_or(StorageError::Io)?);
        files.insert(to.to_owned(), file);
        Ok(())
    }

    fn remove(&self, name: &str) -> Result<(), StorageError> {
        lock(&self.files)
            .remove(name)
            .map(|_| ())
            .ok_or(StorageError::Io)
    }

    fn sync(&self) -> Result<(), StorageError> {
        Ok(())
    }

    /// Records the length and allocates nothing; the margin is not known
    /// in memory, where the budget of the installation bounds what is
    /// staged.
    fn create_reserved(
        &self,
        name: &str,
        len: u64,
        _margin: u64,
    ) -> Result<Box<dyn BlobFile>, StorageError> {
        let mut files = lock(&self.files);
        if files.contains_key(name) {
            return Err(StorageError::Exists);
        }
        let file = Arc::new(Mutex::new(File {
            len,
            parts: BTreeMap::new(),
        }));
        files.insert(name.to_owned(), Arc::clone(&file));
        Ok(Box::new(EphemeralBlob(file)))
    }

    fn open_blob(&self, name: &str) -> Result<Option<Box<dyn BlobFile>>, StorageError> {
        Ok(self
            .file(name)
            .map(|file| Box::new(EphemeralBlob(file)) as Box<dyn BlobFile>))
    }

    fn list(&self) -> Result<Vec<String>, StorageError> {
        Ok(lock(&self.files).keys().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob::{Blob, BlobReader, SEGMENT_LEN, StagingKey, stage};
    use crate::transfers::{TransferFile, TransferRecords, TransferStoreKey};
    use std::io::Read;

    #[test]
    fn a_large_reservation_allocates_nothing_and_reads_as_zero_bytes() {
        let dir = EphemeralDir::new();
        let mut blob = dir.create_reserved("transfer-x", 1 << 40, 0).unwrap();
        assert_eq!(blob.length().unwrap(), 1 << 40);
        let mut buf = [1; 8];
        blob.read_at((1 << 40) - 8, &mut buf).unwrap();
        assert_eq!(buf, [0; 8]);
        assert_eq!(blob.read_at(1 << 40, &mut buf), Err(StorageError::Io));
    }

    #[test]
    fn writes_over_each_other_read_back_as_the_last_one_wrote() {
        let dir = EphemeralDir::new();
        let mut blob = dir.create_reserved("transfer-x", 16, 0).unwrap();
        blob.write_at(0, b"aaaaaaaa").unwrap();
        blob.write_at(4, b"bbbbbbbb").unwrap();
        blob.write_at(0, b"cccccccc").unwrap();
        let mut buf = [0; 16];
        blob.read_at(0, &mut buf).unwrap();
        assert_eq!(&buf, b"ccccbbbbbbbb\0\0\0\0");
    }

    #[test]
    fn transfers_stage_and_store_in_memory_as_on_disk() {
        let dir = EphemeralDir::new();
        let key = StagingKey::random().unwrap();
        let data: Vec<u8> = (0..2 * SEGMENT_LEN + 3)
            .map(|i| u8::try_from(i % 7).unwrap())
            .collect();
        let size = data.len() as u64;
        let (id, digest) = stage(&dir, &key, size, 0, &mut data.as_slice()).unwrap();
        let blob = Blob::open(&dir, &key, id, size).unwrap().unwrap();
        let mut read = Vec::new();
        BlobReader::new(blob, digest)
            .read_to_end(&mut read)
            .unwrap();
        assert_eq!(read, data);
        let (mut store, records, _) =
            TransferFile::open(dir.clone(), TransferStoreKey::random().unwrap()).unwrap();
        assert!(records.records.is_empty());
        store.write(&TransferRecords::default()).unwrap();
        assert_eq!(store.generation(), 2);
    }
}
