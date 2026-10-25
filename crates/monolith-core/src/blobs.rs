//! The staged files of the transfers of one installation
//! (`docs/adr/0014-transfer-staging.md`): blobs on disk in the profile
//! directory, or in memory for an ephemeral installation.
//!
//! Every operation on a blob runs on the blocking pool and holds the blob
//! for its whole length, so that the writes to a blob and the offset taken
//! from it are one after the other. A removal never waits for a holder and
//! never removes a blob that is being read: a blob held by an operation or
//! a save goes when its holder lets go of it, and a save under way is
//! cancelled. A blob is kept open while its transfer uses it. A
//! reservation is made under the lock of the disk, which the writes of the
//! stores take too (`crate::persist::Serialized`): none of them meets the
//! margin a reservation takes for a moment. A removal that fails is tried
//! again later, and every blob no record names goes when the installation
//! opens.

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

/// One blob: its slot, held by one operation at a time, and whether it is
/// to be removed. That is decided under the lock of `doomed`, which a
/// holder takes too to let go of the blob: a removal that finds the blob
/// held leaves the removal to its holder, which carries it out as it lets
/// go, and none is lost between the two.
#[derive(Default)]
struct Cell {
    slot: Arc<AsyncMutex<Slot>>,
    doomed: Mutex<bool>,
}

/// What every holder of a blob may change: the cells, and the removals
/// that failed.
#[derive(Default)]
struct Common {
    cells: Mutex<HashMap<BlobId, Arc<Cell>>>,
    stale: Mutex<HashSet<BlobId>>,
}

/// Removes the blob `id`, held by `slot`: closed first, which Windows
/// needs, and then its file. One whose file does not go is tried again
/// later.
fn erase(dir: &SharedDir, id: BlobId, slot: &mut Slot, common: &Common) {
    slot.gone = true;
    slot.blob = None;
    let removed = match dir.remove(&id.name()) {
        Ok(()) | Err(StorageError::NotFound) => true,
        Err(_) => dir.open_blob(&id.name()).is_ok_and(|file| file.is_none()),
    };
    lock(&common.cells).remove(&id);
    let mut stale = lock(&common.stale);
    if removed {
        stale.remove(&id);
    } else {
        stale.insert(id);
    }
}

/// Lets go of the blob `id`, held by `slot`, after a removal decided
/// meanwhile is carried out.
fn let_go(
    dir: &SharedDir,
    id: BlobId,
    cell: &Cell,
    mut slot: OwnedMutexGuard<Slot>,
    common: &Common,
) {
    let doomed = lock(&cell.doomed);
    if *doomed {
        erase(dir, id, &mut slot, common);
    }
    drop(slot);
    drop(doomed);
}

/// Why a staging failed: the file it read, or the disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StageError {
    /// The source failed, or gave fewer or more bytes than its size.
    Source,
    /// The blob could not be made or written.
    Storage(StorageError),
}

/// The source of a staging, watched: whether it failed, and how many bytes
/// it gave, so that its failure is told apart from one of the disk.
struct Watched {
    inner: Box<dyn Read + Send>,
    read: u64,
    failed: bool,
    ended: bool,
}

impl Read for Watched {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.inner.read(buf) {
            Ok(0) if !buf.is_empty() => {
                self.ended = true;
                Ok(0)
            }
            Ok(count) => {
                self.read = self
                    .read
                    .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
                Ok(count)
            }
            Err(error) => {
                if error.kind() != std::io::ErrorKind::Interrupted {
                    self.failed = true;
                }
                Err(error)
            }
        }
    }
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
    common: Arc<Common>,
    /// One file is staged or saved at a time.
    jobs: Arc<Semaphore>,
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
            common: Arc::new(Common::default()),
            jobs: Arc::new(Semaphore::new(MAX_STAGING_JOBS)),
        }
    }

    fn key(&self) -> Result<Arc<StagingKey>, StorageError> {
        self.key.clone().ok_or(StorageError::Randomness)
    }

    fn cell(&self, id: BlobId) -> Arc<Cell> {
        lock(&self.common.cells).entry(id).or_default().clone()
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
        let common = self.common.clone();
        let cell = self.cell(id);
        let mut slot = cell.slot.clone().lock_owned().await;
        tokio::task::spawn_blocking(move || {
            let result = if slot.gone {
                Err(StorageError::NotFound)
            } else {
                let opened = match slot.blob.take() {
                    Some(blob) => Ok(blob),
                    None => Blob::open(&*dir, &key, id, size)
                        .and_then(|blob| blob.ok_or(StorageError::NotFound)),
                };
                opened.and_then(|mut blob| {
                    let result = work(&mut blob);
                    // A blob that failed is opened again for the next use,
                    // which looks at it afresh.
                    if result.is_ok() {
                        slot.blob = Some(blob);
                    }
                    result
                })
            };
            let_go(&dir, id, &cell, slot, &common);
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
        self.cell(id).slot.lock().await.blob = Some(blob);
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
        source: Box<dyn Read + Send>,
    ) -> Result<(BlobId, [u8; 32]), StageError> {
        let permit = self
            .jobs
            .clone()
            .try_acquire_owned()
            .map_err(|_| StageError::Storage(StorageError::Busy))?;
        let key = self.key().map_err(StageError::Storage)?;
        let dir = self.dir.clone();
        let disk = self.disk.clone();
        let margin = self.margin;
        let (blob, digest) = tokio::task::spawn_blocking(move || {
            let blob = {
                let _disk = lock(&disk);
                Blob::create(&*dir, &key, size, margin).map_err(StageError::Storage)?
            };
            let mut watched = Watched {
                inner: source,
                read: 0,
                failed: false,
                ended: false,
            };
            let staged = blob::fill(&*dir, blob, &mut watched);
            drop(permit);
            staged.map_err(|error| {
                if watched.failed || watched.read > size || (watched.ended && watched.read < size) {
                    StageError::Source
                } else {
                    StageError::Storage(error)
                }
            })
        })
        .await
        .map_err(|_| StageError::Storage(StorageError::Internal))??;
        let id = blob.id();
        self.cell(id).slot.lock().await.blob = Some(blob);
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
    /// the file has `size` bytes and the SHA-256 `digest`. A removal of the
    /// blob meanwhile makes its next read fail: the save is cancelled, and
    /// the blob goes once the reader is dropped.
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
        let common = self.common.clone();
        let cell = self.cell(id);
        let mut slot = cell.slot.clone().lock_owned().await;
        tokio::task::spawn_blocking(move || {
            let opened = if slot.gone {
                Err(StorageError::NotFound)
            } else {
                match slot.blob.take() {
                    Some(blob) => Ok(blob),
                    None => Blob::open(&*dir, &key, id, size)
                        .and_then(|blob| blob.ok_or(StorageError::NotFound)),
                }
            };
            match opened {
                Ok(blob) => Ok(SaveReader {
                    reader: Some(BlobReader::new(blob, digest)),
                    slot: Some(slot),
                    cell,
                    common,
                    dir,
                    id,
                    _permit: permit,
                }),
                Err(error) => {
                    let_go(&dir, id, &cell, slot, &common);
                    Err(error)
                }
            }
        })
        .await
        .map_err(|_| StorageError::Internal)?
    }

    /// Removes the blob `id`: now if nothing holds it, otherwise when its
    /// holder lets go of it, a save cancelled. It never waits for a holder,
    /// and never removes a blob that is being read. A removal that fails is
    /// tried again with [`Self::retry`].
    pub(crate) async fn remove(&self, id: BlobId) {
        let cell = self.cell(id);
        let held = {
            let mut doomed = lock(&cell.doomed);
            *doomed = true;
            cell.slot.clone().try_lock_owned().ok()
        };
        let Some(mut slot) = held else {
            return;
        };
        let dir = self.dir.clone();
        let common = self.common.clone();
        let _ = tokio::task::spawn_blocking(move || erase(&dir, id, &mut slot, &common)).await;
    }

    /// Tries again every removal that failed.
    pub(crate) async fn retry(&self) {
        let stale: Vec<BlobId> = lock(&self.common.stale).iter().copied().collect();
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
                lock(&self.common.stale).insert(id);
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
/// file has its size and its digest. Once the transfer is removed (its
/// identity deleted, its contact ended), every read fails.
pub struct SaveReader {
    reader: Option<BlobReader>,
    slot: Option<OwnedMutexGuard<Slot>>,
    cell: Arc<Cell>,
    common: Arc<Common>,
    dir: SharedDir,
    id: BlobId,
    _permit: OwnedSemaphorePermit,
}

impl fmt::Debug for SaveReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SaveReader").finish_non_exhaustive()
    }
}

impl Drop for SaveReader {
    fn drop(&mut self) {
        // Closed before the blob is let go of, and removed if it is to go.
        drop(self.reader.take());
        if let Some(slot) = self.slot.take() {
            let_go(&self.dir, self.id, &self.cell, slot, &self.common);
        }
    }
}

impl Read for SaveReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if *lock(&self.cell.doomed) {
            return Err(std::io::Error::other("the transfer was removed"));
        }
        match self.reader.as_mut() {
            Some(reader) => reader.read(buf),
            None => Err(std::io::Error::other("the reader was closed")),
        }
    }
}
