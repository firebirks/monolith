//! The staged files of the transfers of one installation
//! (`docs/adr/0014-transfer-staging.md`): blobs on disk in the profile
//! directory, or in memory for an ephemeral installation.
//!
//! Every operation on a blob runs on the blocking pool and holds the blob
//! for its whole length, so that the writes to a blob and the offset taken
//! from it are one after the other, and a removal waits for whatever reads
//! the blob, a save included. A blob is kept open while its transfer uses
//! it. A reservation is made under the lock of the disk, which the writes
//! of the stores take too (`crate::persist::Serialized`): none of them
//! meets the margin a reservation takes for a moment. A removal that fails
//! is tried again later, and every blob no record names goes when the
//! installation opens.

use core::fmt;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::{Arc, Mutex, MutexGuard};

use monolith_protocol::limits::{FILE_FREE_SPACE_MARGIN, MAX_STAGING_JOBS};
use monolith_storage::StorageError;
use monolith_storage::blob::{self, Blob, BlobId, BlobReader, SEGMENT_LEN, StagingKey};
use monolith_storage::dir::VaultDir;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard, OwnedSemaphorePermit, Semaphore};
use zeroize::Zeroizing;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The directory of an installation, shared with its stores.
pub(crate) type SharedDir = Arc<dyn VaultDir + Send + Sync>;

/// The lock of the disk of an installation: held by every reservation and
/// every write of a store.
pub(crate) type DiskLock = Arc<Mutex<()>>;

/// A blob as its slot holds it: open, or not open now.
#[derive(Default)]
struct Slot {
    blob: Option<Blob>,
    /// The blob was removed: nothing opens it again.
    gone: bool,
}

/// What the verification of a staged file found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verified {
    /// Every segment authenticates; the SHA-256 of the file.
    Digest([u8; 32]),
    /// A segment does not authenticate: so many from the start do, and
    /// everything after them counts as not written.
    Damaged(u64),
}

/// The staged files of one installation. See the module documentation.
pub(crate) struct Blobs {
    dir: SharedDir,
    key: Option<Arc<StagingKey>>,
    /// The free space a reservation leaves; none in memory.
    margin: u64,
    disk: DiskLock,
    slots: Mutex<HashMap<BlobId, Arc<AsyncMutex<Slot>>>>,
    /// One file is staged or saved at a time.
    jobs: Arc<Semaphore>,
    /// Blobs whose removal failed, tried again.
    stale: Mutex<HashSet<BlobId>>,
}

impl fmt::Debug for Blobs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Blobs")
            .field("margin", &self.margin)
            .finish_non_exhaustive()
    }
}

impl Blobs {
    /// The blobs of a persistent installation, in its directory.
    pub(crate) fn on_disk(dir: SharedDir, key: StagingKey, disk: DiskLock) -> Self {
        Self::new(dir, Some(key), FILE_FREE_SPACE_MARGIN, disk)
    }

    /// The blobs of an ephemeral installation, in memory, under `key`;
    /// none can be made without one.
    pub(crate) fn in_memory(key: Option<StagingKey>) -> Self {
        Self::new(
            Arc::new(monolith_storage::ephemeral::EphemeralDir::new()),
            key,
            0,
            Arc::new(Mutex::new(())),
        )
    }

    fn new(dir: SharedDir, key: Option<StagingKey>, margin: u64, disk: DiskLock) -> Self {
        Self {
            dir,
            key: key.map(Arc::new),
            margin,
            disk,
            slots: Mutex::new(HashMap::new()),
            jobs: Arc::new(Semaphore::new(MAX_STAGING_JOBS)),
            stale: Mutex::new(HashSet::new()),
        }
    }

    fn key(&self) -> Result<Arc<StagingKey>, StorageError> {
        self.key.clone().ok_or(StorageError::Randomness)
    }

    fn slot(&self, id: BlobId) -> Arc<AsyncMutex<Slot>> {
        lock(&self.slots)
            .entry(id)
            .or_insert_with(|| Arc::new(AsyncMutex::new(Slot::default())))
            .clone()
    }

    /// Runs `work` on the blob `id` of a file of `size` bytes on the
    /// blocking pool, opening it if it is not open, and holding it
    /// meanwhile. Fails with [`StorageError::NotFound`] if it is gone, and
    /// with [`StorageError::BadFormat`] if its header or length is not that
    /// of its record.
    async fn with<R: Send + 'static>(
        &self,
        id: BlobId,
        size: u64,
        work: impl FnOnce(&mut Blob) -> Result<R, StorageError> + Send + 'static,
    ) -> Result<R, StorageError> {
        let key = self.key()?;
        let dir = self.dir.clone();
        let mut slot = self.slot(id).lock_owned().await;
        tokio::task::spawn_blocking(move || {
            if slot.gone {
                return Err(StorageError::NotFound);
            }
            if slot.blob.is_none() {
                slot.blob = Some(Blob::open(&*dir, &key, id, size)?.ok_or(StorageError::NotFound)?);
            }
            let blob = slot.blob.as_mut().ok_or(StorageError::Internal)?;
            let result = work(blob);
            if result.is_err() {
                // Opened again for the next use, which looks at it afresh.
                slot.blob = None;
            }
            result
        })
        .await
        .map_err(|_| StorageError::Internal)?
    }

    /// Creates a blob for a file of `size` bytes, reserved with the margin
    /// and durable with its directory, and keeps it open. The record that
    /// names it is written afterwards.
    pub(crate) async fn create(&self, size: u64) -> Result<BlobId, StorageError> {
        let key = self.key()?;
        let dir = self.dir.clone();
        let disk = self.disk.clone();
        let margin = self.margin;
        let blob = tokio::task::spawn_blocking(move || {
            let _disk = lock(&disk);
            Blob::create(&*dir, &key, size, margin)
        })
        .await
        .map_err(|_| StorageError::Internal)??;
        let id = blob.id();
        let slot = self.slot(id);
        slot.lock().await.blob = Some(blob);
        Ok(id)
    }

    /// Stages the whole file `source` gives, of `size` bytes, in a new
    /// blob: reserved under the lock of the disk, then written, hashed and
    /// synced with its directory. Fails with [`StorageError::Busy`] while
    /// another file is staged or saved; a staging that fails removes its
    /// blob. Returns the blob, kept open, and the SHA-256 of the file.
    pub(crate) async fn stage(
        &self,
        size: u64,
        mut source: Box<dyn Read + Send>,
    ) -> Result<(BlobId, [u8; 32]), StorageError> {
        let permit = self
            .jobs
            .clone()
            .try_acquire_owned()
            .map_err(|_| StorageError::Busy)?;
        let key = self.key()?;
        let dir = self.dir.clone();
        let disk = self.disk.clone();
        let margin = self.margin;
        let (blob, digest) = tokio::task::spawn_blocking(move || {
            let blob = {
                let _disk = lock(&disk);
                Blob::create(&*dir, &key, size, margin)?
            };
            let staged = blob::fill(&*dir, blob, &mut *source);
            drop(permit);
            staged
        })
        .await
        .map_err(|_| StorageError::Internal)??;
        let id = blob.id();
        self.slot(id).lock().await.blob = Some(blob);
        Ok((id, digest))
    }

    /// The bytes of the file in segment `index` of the blob `id`, read and
    /// authenticated.
    pub(crate) async fn read_chunk(
        &self,
        id: BlobId,
        size: u64,
        index: u64,
    ) -> Result<Vec<u8>, StorageError> {
        self.with(id, size, move |blob| {
            let mut segment = Zeroizing::new([0_u8; SEGMENT_LEN]);
            let len = blob.read_segment(index, &mut segment)?;
            Ok(segment.get(..len).ok_or(StorageError::Internal)?.to_vec())
        })
        .await
    }

    /// Writes `data`, the bytes of the file in segment `index`, to the
    /// blob `id`. Durable only after [`Self::sync`].
    pub(crate) async fn write_chunk(
        &self,
        id: BlobId,
        size: u64,
        index: u64,
        data: Vec<u8>,
    ) -> Result<(), StorageError> {
        self.with(id, size, move |blob| blob.write_segment(index, &data))
            .await
    }

    /// Flushes what was written to the blob `id`.
    pub(crate) async fn sync(&self, id: BlobId, size: u64) -> Result<(), StorageError> {
        self.with(id, size, Blob::sync).await
    }

    /// The number of segments of the blob `id` that authenticate from the
    /// start, once the blob is synced: how much of an accepted transfer is
    /// staged. `None` if the blob is missing or does not match its record,
    /// which then counts as nothing staged.
    pub(crate) async fn scan(&self, id: BlobId, size: u64) -> Result<Option<u64>, StorageError> {
        match self
            .with(id, size, |blob| {
                blob.sync()?;
                blob.scan()
            })
            .await
        {
            Ok(segments) => Ok(Some(segments)),
            Err(StorageError::NotFound | StorageError::BadFormat) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Syncs the blob `id` and computes the SHA-256 of its whole file.
    pub(crate) async fn verify(&self, id: BlobId, size: u64) -> Result<Verified, StorageError> {
        self.with(id, size, |blob| {
            blob.sync()?;
            match blob.digest() {
                Ok(digest) => Ok(Verified::Digest(digest)),
                Err(StorageError::AuthenticationFailed) => blob.scan().map(Verified::Damaged),
                Err(error) => Err(error),
            }
        })
        .await
    }

    /// A reader of the file of the complete blob `id`, for a save: it holds
    /// the blob until it is dropped, and one save or staging runs at a
    /// time ([`StorageError::Busy`]). After the last byte it fails unless
    /// the file has `size` bytes and the SHA-256 `digest`.
    pub(crate) async fn reader(
        &self,
        id: BlobId,
        size: u64,
        digest: [u8; 32],
    ) -> Result<SaveReader, StorageError> {
        let permit = self
            .jobs
            .clone()
            .try_acquire_owned()
            .map_err(|_| StorageError::Busy)?;
        let key = self.key()?;
        let dir = self.dir.clone();
        let mut slot = self.slot(id).lock_owned().await;
        tokio::task::spawn_blocking(move || {
            if slot.gone {
                return Err(StorageError::NotFound);
            }
            let blob = match slot.blob.take() {
                Some(blob) => blob,
                None => Blob::open(&*dir, &key, id, size)?.ok_or(StorageError::NotFound)?,
            };
            Ok(SaveReader {
                reader: BlobReader::new(blob, digest),
                _slot: slot,
                _permit: permit,
            })
        })
        .await
        .map_err(|_| StorageError::Internal)?
    }

    /// Removes the blob `id`, once whatever holds it lets go of it. A
    /// removal that fails is tried again with [`Self::retry`].
    pub(crate) async fn remove(&self, id: BlobId) {
        let dir = self.dir.clone();
        let slot = self.slot(id);
        let mut held = slot.lock_owned().await;
        let removed = tokio::task::spawn_blocking(move || {
            held.gone = true;
            // Closed before it is removed, which Windows needs.
            held.blob = None;
            match dir.remove(&id.name()) {
                Ok(()) | Err(StorageError::NotFound) => true,
                Err(_) => dir.open_blob(&id.name()).is_ok_and(|file| file.is_none()),
            }
        })
        .await
        .unwrap_or(false);
        lock(&self.slots).remove(&id);
        let mut stale = lock(&self.stale);
        if removed {
            stale.remove(&id);
        } else {
            stale.insert(id);
        }
    }

    /// Tries again every removal that failed.
    pub(crate) async fn retry(&self) {
        let stale: Vec<BlobId> = lock(&self.stale).iter().copied().collect();
        for id in stale {
            self.remove(id).await;
        }
    }

    /// Removes every blob in the directory that `named` does not hold:
    /// once the transfer store opened, what no record names. Run when the
    /// installation opens, before anything uses a blob. Returns how many
    /// went; one that does not go is tried again later.
    pub(crate) fn clean(&self, named: &HashSet<BlobId>) -> Result<usize, StorageError> {
        let mut removed = 0_usize;
        for name in self.dir.list()? {
            let Some(id) = BlobId::from_name(&name) else {
                continue;
            };
            if named.contains(&id) {
                continue;
            }
            if self.dir.remove(&name).is_ok() {
                removed = removed.saturating_add(1);
            } else {
                lock(&self.stale).insert(id);
            }
        }
        if removed > 0 {
            self.dir.sync()?;
        }
        Ok(removed)
    }
}

/// The file of a complete transfer, read for a save
/// (`docs/adr/0014-transfer-staging.md`, "Saving a received file"). It
/// holds the blob, which is removed only once this is dropped. After the
/// last byte it returns an error, never the end of the file, unless the
/// file has its size and its digest.
pub struct SaveReader {
    reader: BlobReader,
    _slot: OwnedMutexGuard<Slot>,
    _permit: OwnedSemaphorePermit,
}

impl fmt::Debug for SaveReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SaveReader").finish_non_exhaustive()
    }
}

impl Read for SaveReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.reader.read(buf)
    }
}
