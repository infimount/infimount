//! Linux descriptor confinement for the optional hosted artifact service.
//!
//! File data still passes exclusively through OpenDAL. The small OS boundary
//! here pins directories and regular files without following symlinks, and
//! publishes a completed staging inode with an atomic create-only hard link.
//! A pathname check followed by an ordinary OpenDAL open would be racy.
use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures::io::AsyncReadExt;
use opendal::Operator;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::models::{CoreError, Entry, Result, Source, SourceKind};

pub const STAGING_SEGMENT: &str = ".infimount-hosted-staging";
pub const STREAM_BUFFER_BYTES: usize = 64 * 1024;

fn invalid_path() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "invalid confined path")
}

/// Hosted paths have one canonical spelling. No URL decoding is performed.
pub fn validate_relative_path(path: &str, allow_empty: bool) -> io::Result<()> {
    if path.len() > 4096
        || (!allow_empty && path.is_empty())
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains(['\\', '%'])
        || path.chars().any(char::is_control)
        || (!path.is_empty()
            && path.split('/').any(|part| {
                part.is_empty() || matches!(part, "." | ".." | STAGING_SEGMENT) || part.len() > 255
            }))
    {
        return Err(invalid_path());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIdentity {
    pub device: u64,
    pub inode: u64,
    pub size: u64,
    pub modified_seconds: i64,
    pub modified_nanos: i64,
    pub changed_seconds: i64,
    pub changed_nanos: i64,
}

#[derive(Clone)]
pub struct ConfinedRoot {
    directory: Arc<File>,
    _lease: Arc<File>,
}

pub struct PinnedFile {
    file: File,
    operator: Operator,
    key: String,
}

impl PinnedFile {
    pub fn operator(&self) -> &Operator {
        &self.operator
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn identity(&self) -> io::Result<FileIdentity> {
        platform::identity(&self.file)
    }
}

impl ConfinedRoot {
    /// Existing absolute root only. Each component, including ancestors, is
    /// opened with O_NOFOLLOW. A later rename does not move this authority.
    pub fn open(path: &Path) -> Result<Self> {
        let directory = platform::open_root(path)?;
        let staging = platform::open_directory(&directory, STAGING_SEGMENT, true)?;
        platform::require_private_directory(&staging)?;
        let lease = platform::lock_and_cleanup(&staging)?;
        Ok(Self {
            directory: Arc::new(directory),
            _lease: Arc::new(lease),
        })
    }

    fn directory(&self, relative: &str, create: bool) -> io::Result<File> {
        validate_relative_path(relative, true)?;
        let mut current = self.directory.try_clone()?;
        for segment in relative.split('/').filter(|segment| !segment.is_empty()) {
            current = platform::open_directory(&current, segment, create)?;
        }
        Ok(current)
    }

    pub fn ensure_directory(&self, relative: &str) -> Result<()> {
        self.directory(relative, true)?;
        Ok(())
    }

    pub fn pin_file(&self, relative: &str) -> Result<PinnedFile> {
        validate_relative_path(relative, false)?;
        let (parent, leaf) = relative.rsplit_once('/').unwrap_or(("", relative));
        let parent = self.directory(parent, false)?;
        let file = platform::open_regular(&parent, leaf)?;
        let (operator, key) = descriptor_operator(&file)?;
        Ok(PinnedFile {
            file,
            operator,
            key,
        })
    }

    pub async fn stat(&self, relative: &str) -> Result<Entry> {
        validate_relative_path(relative, true)?;
        let (file, is_dir) = if relative.is_empty() {
            (self.directory.try_clone()?, true)
        } else {
            let (parent, leaf) = relative.rsplit_once('/').unwrap_or(("", relative));
            platform::open_entry(&self.directory(parent, false)?, leaf)?
        };
        let (op, key) = descriptor_operator(&file)?;
        let metadata = op.stat(&key).await?;
        Ok(Entry {
            path: relative.to_owned(),
            name: relative.rsplit('/').next().unwrap_or_default().to_owned(),
            is_dir,
            size: if is_dir { 0 } else { metadata.content_length() },
            modified_at: metadata.last_modified().map(|time| time.to_string()),
            etag: None,
        })
    }

    /// Enumerate names through the held directory, then authorize and pin each
    /// child before OpenDAL metadata access. OpenDAL's FS lister probes child
    /// metadata eagerly, so it cannot enforce this boundary itself.
    pub async fn list_page(
        &self,
        relative: &str,
        offset: usize,
        limit: usize,
    ) -> Result<(Vec<Entry>, Option<usize>)> {
        self.list_page_with_filter(relative, offset, limit, |_| true)
            .await
    }

    /// Apply child policy before opening the child or probing its metadata.
    /// Offsets and the scan bound still count denied names, bounding sparse
    /// listings without disclosing those names in the result or cursor.
    pub async fn list_page_with_filter<F>(
        &self,
        relative: &str,
        offset: usize,
        limit: usize,
        mut include: F,
    ) -> Result<(Vec<Entry>, Option<usize>)>
    where
        F: FnMut(&str) -> bool,
    {
        let directory = self.directory(relative, false)?;
        // file_name() reads only the directory entry. Never call DirEntry's
        // metadata or file_type helpers before the policy callback.
        let names = std::fs::read_dir(format!(
            "/proc/self/fd/{}",
            platform::descriptor(&directory)?
        ))?;
        let mut examined = 0usize;
        let mut entries = Vec::new();
        for entry in names {
            let entry = entry?;
            examined += 1;
            if examined > 10_000 {
                return Err(CoreError::Config(
                    "confined listing scan limit reached".into(),
                ));
            }
            if examined <= offset {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if validate_relative_path(name, false).is_err() {
                continue;
            }
            let child_path = if relative.is_empty() {
                name.to_owned()
            } else {
                format!("{relative}/{name}")
            };
            if !include(&child_path) {
                continue;
            }
            if entries.len() == limit {
                return Ok((entries, Some(examined - 1)));
            }
            // Read relative to the already pinned listing directory. Reopening
            // via the root could list a different directory after a rename.
            let (child, is_dir) = match platform::open_entry(&directory, name) {
                Ok(entry) => entry,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
                    ) =>
                {
                    continue
                }
                Err(error) => return Err(error.into()),
            };
            let (child_op, key) = descriptor_operator(&child)?;
            let metadata = child_op.stat(&key).await?;
            entries.push(Entry {
                path: child_path,
                name: name.to_owned(),
                is_dir,
                size: if is_dir { 0 } else { metadata.content_length() },
                modified_at: metadata.last_modified().map(|time| time.to_string()),
                etag: None,
            });
        }
        Ok((entries, None))
    }

    pub fn new_stage(&self, expected_bytes: u64) -> Result<StagedFile> {
        let directory = platform::open_directory(&self.directory, STAGING_SEGMENT, true)?;
        platform::require_private_directory(&directory)?;
        StagedFile::in_directory(directory, expected_bytes, None)
    }

    /// No pathname-derived byte access occurs here. linkat links the completed
    /// inode into a pinned destination directory and fails if the name exists.
    pub fn publish(&self, stage: &mut StagedFile, relative: &str) -> Result<()> {
        validate_relative_path(relative, false)?;
        if !stage.finished {
            return Err(CoreError::Config("staging file is not complete".into()));
        }
        let (parent, leaf) = relative.rsplit_once('/').unwrap_or(("", relative));
        let parent = self.directory(parent, false)?;
        let name = stage.name.as_deref().ok_or_else(invalid_path)?;
        platform::publish(&stage.directory, name, &stage.file, &parent, leaf)?;
        platform::unlink(&stage.directory, name)?;
        stage.name = None;
        Ok(())
    }
}

/// A bounded private file, addressed through its held descriptor by OpenDAL.
/// Local-destination uploads use the destination filesystem so commit is an
/// atomic link. Other stages use a private temporary directory.
pub struct StagedFile {
    directory: File,
    file: Arc<File>,
    name: Option<String>,
    operator: Operator,
    key: String,
    writer: Option<StageWriter>,
    retention: Option<Arc<dyn Send + Sync>>,
    expected_bytes: u64,
    written_bytes: u64,
    digest: Sha256,
    sha256: Option<String>,
    finished: bool,
    _temporary: Option<tempfile::TempDir>,
}

enum StageCommand {
    Write(Vec<u8>, tokio::sync::oneshot::Sender<Result<()>>),
    Close(tokio::sync::oneshot::Sender<Result<()>>),
}

/// A dropped request must not release a descriptor while Tokio still has an
/// OS open queued for its /proc path. This worker owns the exact descriptor,
/// not a dup with another number, and all request resources until its accepted
/// chunk and final close complete. The one-command queue bounds detached work.
struct StageWriter {
    commands: tokio::sync::mpsc::Sender<StageCommand>,
    worker: tokio::task::JoinHandle<()>,
}

impl StageWriter {
    fn new(
        operator: Operator,
        key: String,
        file: Arc<File>,
        retention: Option<Arc<dyn Send + Sync>>,
    ) -> Result<Self> {
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| io::Error::other("staging requires an async runtime"))?;
        let (commands, mut receiver) = tokio::sync::mpsc::channel(1);
        let worker_runtime = runtime.clone();
        // A blocking task itself is not cancelled during runtime shutdown.
        // Holding the descriptor in an ordinary async task would still permit
        // queued filesystem operations to outlive that task's cancellation.
        let worker = runtime.spawn_blocking(move || {
            worker_runtime.block_on(async {
                let mut writer: Option<opendal::Writer> = None;
                while let Some(command) = receiver.recv().await {
                    match command {
                        StageCommand::Write(bytes, reply) => {
                            let result = async {
                                if writer.is_none() {
                                    writer = Some(operator.writer_with(&key).concurrent(1).await?);
                                }
                                writer
                                    .as_mut()
                                    .expect("writer initialized")
                                    .write(bytes)
                                    .await?;
                                Ok(())
                            }
                            .await;
                            let failed = result.is_err();
                            if reply.send(result).is_err() || failed {
                                break;
                            }
                        }
                        StageCommand::Close(reply) => {
                            let result = match writer.take() {
                                Some(mut writer) => {
                                    writer.close().await.map(|_| ()).map_err(CoreError::from)
                                }
                                None => Ok(()),
                            };
                            let _ = reply.send(result);
                            return;
                        }
                    }
                }
                // OpenDAL FS can buffer one chunk. Drain it before releasing
                // the descriptor and reservation even when the caller left.
                if let Some(mut writer) = writer {
                    let _ = writer.close().await;
                }
            });
            drop(file);
            drop(retention);
        });
        Ok(Self { commands, worker })
    }

    async fn write(&self, bytes: Vec<u8>) -> Result<()> {
        let (reply, result) = tokio::sync::oneshot::channel();
        self.commands
            .send(StageCommand::Write(bytes, reply))
            .await
            .map_err(|_| io::Error::other("staging writer stopped"))?;
        result
            .await
            .map_err(|_| io::Error::other("staging writer stopped"))?
    }

    async fn close(self) -> Result<()> {
        let (reply, result) = tokio::sync::oneshot::channel();
        self.commands
            .send(StageCommand::Close(reply))
            .await
            .map_err(|_| io::Error::other("staging writer stopped"))?;
        let result = result
            .await
            .map_err(|_| io::Error::other("staging writer stopped"))?;
        self.worker
            .await
            .map_err(|_| io::Error::other("staging writer stopped"))?;
        result
    }
}

impl StagedFile {
    pub fn temporary(expected_bytes: u64) -> Result<Self> {
        let mut builder = tempfile::Builder::new();
        builder.prefix("infimount-hosted-");
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(std::fs::Permissions::from_mode(0o700));
        }
        let temporary = builder.tempdir()?;
        let directory = platform::open_root(temporary.path())?;
        platform::require_private_directory(&directory)?;
        let mut stage = Self::in_directory(directory, expected_bytes, Some(temporary))?;
        // Remote uploads and download snapshots never need a named commit.
        // Unlink now so process death cannot orphan their data on disk.
        if let Some(name) = stage.name.take() {
            platform::unlink(&stage.directory, &name)?;
        }
        Ok(stage)
    }

    fn in_directory(
        directory: File,
        expected_bytes: u64,
        temporary: Option<tempfile::TempDir>,
    ) -> Result<Self> {
        let name = uuid::Uuid::new_v4().to_string();
        let file = platform::create_file(&directory, &name)?;
        let (operator, key) = descriptor_operator(&file)?;
        Ok(Self {
            directory,
            file: Arc::new(file),
            name: Some(name),
            operator,
            key,
            writer: None,
            retention: None,
            expected_bytes,
            written_bytes: 0,
            digest: Sha256::new(),
            sha256: None,
            finished: false,
            _temporary: temporary,
        })
    }

    pub fn operator(&self) -> &Operator {
        &self.operator
    }
    pub fn key(&self) -> &str {
        &self.key
    }
    pub fn size(&self) -> u64 {
        self.written_bytes
    }
    pub fn sha256(&self) -> Option<&str> {
        self.sha256.as_deref()
    }

    /// Resource ownership only. Idle grants retain their quota, and active
    /// operations additionally retain their bounded request permits.
    pub fn set_resource_retention(&mut self, retention: Option<Arc<dyn Send + Sync>>) {
        self.retention = retention;
    }

    pub async fn write_chunk(&mut self, bytes: &[u8]) -> Result<()> {
        if self.finished
            || self.written_bytes.saturating_add(bytes.len() as u64) > self.expected_bytes
        {
            return Err(CoreError::Config("staging byte limit exceeded".into()));
        }
        if self.writer.is_none() {
            self.writer = Some(StageWriter::new(
                self.operator.clone(),
                self.key.clone(),
                self.file.clone(),
                self.retention.clone(),
            )?);
        }
        // OpenDAL's FS writer writes bounded chunks to the pinned staging file.
        for chunk in bytes.chunks(STREAM_BUFFER_BYTES) {
            self.writer
                .as_mut()
                .expect("writer initialized")
                .write(chunk.to_vec())
                .await?;
            self.digest.update(chunk);
            self.written_bytes += chunk.len() as u64;
        }
        Ok(())
    }

    pub async fn finish(&mut self, expected_sha256: Option<&str>) -> Result<()> {
        if self.finished || self.written_bytes != self.expected_bytes {
            return Err(CoreError::Config("staging size mismatch".into()));
        }
        let digest = format!("{:x}", self.digest.clone().finalize());
        if expected_sha256.is_some_and(|expected| expected != digest) {
            return Err(CoreError::Config("staging checksum mismatch".into()));
        }
        if let Some(writer) = self.writer.take() {
            writer.close().await?;
        }
        let metadata = self.operator.stat(&self.key).await?;
        if metadata.content_length() != self.expected_bytes {
            return Err(CoreError::Config("staging size mismatch".into()));
        }
        self.sha256 = Some(digest);
        self.finished = true;
        Ok(())
    }
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        // The only unlink target is our random file in a held private stage
        // directory. Never delete a destination after an ambiguous write.
        if let Some(name) = &self.name {
            let _ = platform::unlink(&self.directory, name);
        }
    }
}

/// Copy a verified private stage through OpenDAL's conditional writer. Reject
/// a backend lacking atomic create-only support before opening any writer.
pub async fn publish_stage(stage: &StagedFile, destination: &Operator, path: &str) -> Result<()> {
    publish_stage_checked(stage, destination, path, None, None, || Ok(())).await
}

/// Opaque request resources are held through asynchronous cancellation cleanup.
/// This contains no identity or authorization. The caller still checks those
/// explicitly before every write and commit.
pub struct PublishCleanup {
    pub retention: Option<Arc<dyn Send + Sync>>,
    pub failed: Arc<AtomicBool>,
}

struct AbortOnDrop {
    writer: Option<opendal::Writer>,
    cleanup: Option<PublishCleanup>,
}

impl AbortOnDrop {
    fn writer(&mut self) -> &mut opendal::Writer {
        self.writer.as_mut().expect("active writer")
    }
    fn disarm(&mut self) {
        self.writer.take();
        self.cleanup.take();
    }
    async fn abort(&mut self) -> bool {
        let result =
            tokio::time::timeout(std::time::Duration::from_secs(5), self.writer().abort()).await;
        let success = matches!(result, Ok(Ok(())));
        if !success {
            if let Some(cleanup) = &self.cleanup {
                cleanup.failed.store(true, Ordering::Release);
            }
        }
        self.disarm();
        success
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        let Some(mut writer) = self.writer.take() else {
            return;
        };
        let cleanup = self.cleanup.take();
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn(async move {
                    let success = matches!(
                        tokio::time::timeout(std::time::Duration::from_secs(5), writer.abort())
                            .await,
                        Ok(Ok(()))
                    );
                    if !success {
                        if let Some(cleanup) = &cleanup {
                            cleanup.failed.store(true, Ordering::Release);
                        }
                    }
                    // Dropping cleanup after abort releases staging quota and
                    // request permits. Never delete an ambiguous destination.
                    drop(cleanup);
                });
            }
            Err(_) => {
                if let Some(cleanup) = cleanup {
                    cleanup.failed.store(true, Ordering::Release);
                }
            }
        }
    }
}

pub async fn publish_stage_checked<F>(
    stage: &StagedFile,
    destination: &Operator,
    path: &str,
    content_type: Option<&str>,
    cleanup: Option<PublishCleanup>,
    check: F,
) -> Result<()>
where
    F: Fn() -> Result<()>,
{
    if !stage.finished || !destination.info().capability().write_with_if_not_exists {
        return Err(CoreError::Config(
            "safe create-only writes are unsupported".into(),
        ));
    }
    if content_type.is_some() && !destination.info().capability().write_with_content_type {
        return Err(CoreError::Config(
            "content type writes are unsupported".into(),
        ));
    }
    check()?;
    let mut reader = stage
        .operator
        .reader(&stage.key)
        .await?
        .into_futures_async_read(0..stage.written_bytes)
        .await?;
    let request = destination.writer_with(path).if_not_exists(true);
    let request = match content_type {
        Some(value) => request.content_type(value),
        None => request,
    };
    let mut writer = AbortOnDrop {
        writer: Some(request.await?),
        cleanup,
    };
    let result = async {
        let mut digest = Sha256::new();
        let mut total = 0_u64;
        let mut buffer = vec![0_u8; STREAM_BUFFER_BYTES];
        loop {
            check()?;
            let count = reader.read(&mut buffer).await?;
            if count == 0 {
                break;
            }
            total = total.saturating_add(count as u64);
            if total > stage.written_bytes {
                return Err(CoreError::Config("staging changed".into()));
            }
            digest.update(&buffer[..count]);
            writer.writer().write(buffer[..count].to_vec()).await?;
        }
        if total != stage.written_bytes || Some(format!("{:x}", digest.finalize())) != stage.sha256
        {
            return Err(CoreError::Config("staging changed".into()));
        }
        check()?;
        writer.writer().close().await?;
        Ok(())
    }
    .await;
    if result.is_err() {
        if !writer.abort().await {
            return Err(CoreError::TransferCleanupRequired);
        }
    } else {
        writer.disarm();
    }
    result
}

fn descriptor_operator(file: &File) -> Result<(Operator, String)> {
    let key = platform::descriptor(file)?.to_string();
    let source = Source {
        id: "hosted-pinned".into(),
        name: "hosted-pinned".into(),
        kind: SourceKind::Local,
        root: "/proc/self/fd".into(),
        config: serde_json::json!({"root":"/proc/self/fd"}),
    };
    Ok((crate::registry::build_operator(&source)?, key))
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::MetadataExt;

    fn name(value: &str) -> io::Result<CString> {
        if value.is_empty() || value.contains('/') || matches!(value, "." | "..") {
            return Err(invalid_path());
        }
        CString::new(value).map_err(|_| invalid_path())
    }
    fn opened(fd: libc::c_int) -> io::Result<File> {
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            // SAFETY: this fd was just returned by open/openat and is uniquely owned.
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
    pub fn descriptor(file: &File) -> io::Result<i32> {
        Ok(file.as_raw_fd())
    }
    pub fn open_root(path: &Path) -> io::Result<File> {
        if !path.is_absolute() {
            return Err(invalid_path());
        }
        // SAFETY: NUL-terminated static path, valid flags, returned fd is owned.
        let mut current = opened(unsafe {
            libc::open(
                c"/".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        })?;
        for component in path.components() {
            match component {
                std::path::Component::RootDir => {}
                std::path::Component::Normal(segment) => {
                    current = open_directory(
                        &current,
                        segment.to_str().ok_or_else(invalid_path)?,
                        false,
                    )?;
                }
                _ => return Err(invalid_path()),
            }
        }
        Ok(current)
    }
    pub fn open_directory(parent: &File, segment: &str, create: bool) -> io::Result<File> {
        let segment = name(segment)?;
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        // SAFETY: descriptor and CString live throughout the syscall.
        let first = unsafe { libc::openat(parent.as_raw_fd(), segment.as_ptr(), flags) };
        if first >= 0 {
            return opened(first);
        }
        let error = io::Error::last_os_error();
        if !create || error.kind() != io::ErrorKind::NotFound {
            return Err(error);
        }
        // SAFETY: fd-relative mkdir, restrictive mode, no byte access.
        if unsafe { libc::mkdirat(parent.as_raw_fd(), segment.as_ptr(), 0o700) } != 0
            && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists
        {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: reopening with O_NOFOLLOW rejects a concurrent symlink swap.
        opened(unsafe { libc::openat(parent.as_raw_fd(), segment.as_ptr(), flags) })
    }
    pub fn require_private_directory(file: &File) -> io::Result<()> {
        let metadata = file.metadata()?;
        // SAFETY: geteuid has no parameters or ownership effects.
        if !metadata.is_dir()
            || metadata.mode() & 0o077 != 0
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(invalid_path());
        }
        Ok(())
    }
    pub fn open_entry(parent: &File, segment: &str) -> io::Result<(File, bool)> {
        let segment = name(segment)?;
        // O_PATH does not open FIFOs or devices for I/O. Inspect the pinned
        // inode before obtaining a readable descriptor through procfs.
        // SAFETY: fd and name are valid and returned fd is uniquely owned.
        let pinned = opened(unsafe {
            libc::openat(
                parent.as_raw_fd(),
                segment.as_ptr(),
                libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })?;
        let metadata = pinned.metadata()?;
        let is_dir = metadata.is_dir();
        if !is_dir && (!metadata.is_file() || metadata.nlink() != 1) {
            return Err(invalid_path());
        }
        let proc_path = CString::new(format!("/proc/self/fd/{}", pinned.as_raw_fd()))
            .map_err(|_| invalid_path())?;
        let flags = libc::O_RDONLY | libc::O_CLOEXEC | if is_dir { libc::O_DIRECTORY } else { 0 };
        // SAFETY: procfs opens exactly the live pinned descriptor. It cannot
        // resolve a caller-controlled symlink because O_PATH already pinned it.
        let file = opened(unsafe { libc::open(proc_path.as_ptr(), flags) })?;
        Ok((file, is_dir))
    }
    pub fn open_regular(parent: &File, segment: &str) -> io::Result<File> {
        let (file, is_dir) = open_entry(parent, segment)?;
        if is_dir {
            return Err(invalid_path());
        }
        Ok(file)
    }
    pub fn identity(file: &File) -> io::Result<FileIdentity> {
        let meta = file.metadata()?;
        if !meta.is_file() || meta.nlink() != 1 {
            return Err(invalid_path());
        }
        Ok(FileIdentity {
            device: meta.dev(),
            inode: meta.ino(),
            size: meta.len(),
            modified_seconds: meta.mtime(),
            modified_nanos: meta.mtime_nsec(),
            changed_seconds: meta.ctime(),
            changed_nanos: meta.ctime_nsec(),
        })
    }
    pub fn create_file(parent: &File, value: &str) -> io::Result<File> {
        let value = name(value)?;
        let flags =
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        // SAFETY: unique file, restrictive mode, valid descriptor and CString.
        opened(unsafe {
            libc::openat(
                parent.as_raw_fd(),
                value.as_ptr(),
                flags,
                0o600 as libc::c_int,
            )
        })
    }
    pub fn lock_and_cleanup(directory: &File) -> io::Result<File> {
        use fs2::FileExt;
        let lock_name = c"instance.lock";
        // SAFETY: private directory, fixed single component, no symlink follow.
        let lease = opened(unsafe {
            libc::openat(
                directory.as_raw_fd(),
                lock_name.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600 as libc::c_int,
            )
        })?;
        let metadata = lease.metadata()?;
        // SAFETY: geteuid has no ownership effects.
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.mode() & 0o077 != 0
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(invalid_path());
        }
        lease.try_lock_exclusive()?;
        // Only metadata cleanup of our private staging namespace. A second
        // process cannot clean a live writer because it cannot acquire lease.
        for entry in std::fs::read_dir(format!("/proc/self/fd/{}", directory.as_raw_fd()))? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().ok_or_else(invalid_path)?;
            if name == "instance.lock" {
                continue;
            }
            let id = uuid::Uuid::parse_str(name).map_err(|_| invalid_path())?;
            if id.get_version() != Some(uuid::Version::Random) || id.to_string() != name {
                return Err(invalid_path());
            }
            let _file = open_regular(directory, name)?;
            unlink(directory, name)?;
        }
        Ok(lease)
    }
    pub fn unlink(parent: &File, value: &str) -> io::Result<()> {
        let value = name(value)?;
        // SAFETY: private staging directory and server-generated component.
        if unsafe { libc::unlinkat(parent.as_raw_fd(), value.as_ptr(), 0) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    pub fn publish(
        stage: &File,
        value: &str,
        file: &File,
        parent: &File,
        leaf: &str,
    ) -> io::Result<()> {
        let value = name(value)?;
        let leaf = name(leaf)?;
        // The staging directory is private and only this process writes its
        // random names. Still check the name resolves to our exact held inode.
        let (named, directory) = open_entry(stage, value.to_str().map_err(|_| invalid_path())?)?;
        if directory || identity(&named)? != identity(file)? {
            return Err(invalid_path());
        }
        // SAFETY: two pinned directories, valid names. flags=0 never follows
        // source symlinks, and linkat never replaces a destination entry.
        if unsafe {
            libc::linkat(
                stage.as_raw_fd(),
                value.as_ptr(),
                parent.as_raw_fd(),
                leaf.as_ptr(),
                0,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
mod platform {
    use super::*;
    fn unsupported<T>() -> io::Result<T> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "hosted descriptor confinement requires Linux",
        ))
    }
    pub fn descriptor(_: &File) -> io::Result<i32> {
        unsupported()
    }
    pub fn open_root(_: &Path) -> io::Result<File> {
        unsupported()
    }
    pub fn open_directory(_: &File, _: &str, _: bool) -> io::Result<File> {
        unsupported()
    }
    pub fn require_private_directory(_: &File) -> io::Result<()> {
        unsupported()
    }
    pub fn open_entry(_: &File, _: &str) -> io::Result<(File, bool)> {
        unsupported()
    }
    pub fn open_regular(_: &File, _: &str) -> io::Result<File> {
        unsupported()
    }
    pub fn identity(_: &File) -> io::Result<FileIdentity> {
        unsupported()
    }
    pub fn create_file(_: &File, _: &str) -> io::Result<File> {
        unsupported()
    }
    pub fn lock_and_cleanup(_: &File) -> io::Result<File> {
        unsupported()
    }
    pub fn unlink(_: &File, _: &str) -> io::Result<()> {
        unsupported()
    }
    pub fn publish(_: &File, _: &str, _: &File, _: &File, _: &str) -> io::Result<()> {
        unsupported()
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[tokio::test]
    async fn pinned_descriptor_survives_rename_and_rejects_symlinks_and_hardlinks() {
        let temporary = tempfile::tempdir().unwrap();
        let root_path = temporary.path().join("root");
        std::fs::create_dir(&root_path).unwrap();
        std::fs::create_dir(root_path.join("alice")).unwrap();
        std::fs::write(root_path.join("alice/report.bin"), b"alice bytes").unwrap();
        std::fs::write(temporary.path().join("outside.bin"), b"outside secret").unwrap();
        let root = ConfinedRoot::open(&root_path).unwrap();
        let pinned = root.pin_file("alice/report.bin").unwrap();
        std::fs::rename(root_path.join("alice"), root_path.join("old-alice")).unwrap();
        symlink(temporary.path(), root_path.join("alice")).unwrap();
        assert!(root.pin_file("alice/outside.bin").is_err());
        let bytes = pinned.operator().read(pinned.key()).await.unwrap().to_vec();
        assert_eq!(bytes, b"alice bytes");
        symlink(
            temporary.path().join("outside.bin"),
            root_path.join("link.bin"),
        )
        .unwrap();
        assert!(root.pin_file("link.bin").is_err());
        std::fs::hard_link(
            temporary.path().join("outside.bin"),
            root_path.join("hard.bin"),
        )
        .unwrap();
        assert!(root.pin_file("hard.bin").is_err());
        let (entries, _) = root.list_page("", 0, 100).await.unwrap();
        assert!(entries.iter().all(|entry| !matches!(
            entry.name.as_str(),
            "alice" | "link.bin" | "hard.bin" | STAGING_SEGMENT
        )));
    }

    #[tokio::test]
    async fn staged_binary_commit_is_exact_create_only_and_cleanup_is_private() {
        let temporary = tempfile::tempdir().unwrap();
        let root = ConfinedRoot::open(temporary.path()).unwrap();
        root.ensure_directory("alice/nested").unwrap();
        let bytes = [0, 255, 128, 1, 13, 10, 0];
        let mut stage = root.new_stage(bytes.len() as u64).unwrap();
        stage.write_chunk(&bytes[..3]).await.unwrap();
        stage.write_chunk(&bytes[3..]).await.unwrap();
        stage.finish(None).await.unwrap();
        root.publish(&mut stage, "alice/nested/report.bin").unwrap();
        assert_eq!(
            std::fs::read(temporary.path().join("alice/nested/report.bin")).unwrap(),
            bytes
        );
        let mut collision = root.new_stage(3).unwrap();
        collision.write_chunk(b"bad").await.unwrap();
        collision.finish(None).await.unwrap();
        let error = root
            .publish(&mut collision, "alice/nested/report.bin")
            .unwrap_err();
        assert!(matches!(error,CoreError::Io(error) if error.kind()==io::ErrorKind::AlreadyExists));
        drop(collision);
        assert_eq!(
            std::fs::read(temporary.path().join("alice/nested/report.bin")).unwrap(),
            bytes
        );
        let stage_names = std::fs::read_dir(temporary.path().join(STAGING_SEGMENT))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(stage_names, vec![std::ffi::OsString::from("instance.lock")]);
    }

    #[tokio::test]
    async fn listing_denied_child_never_opens_it_for_metadata() {
        use std::ffi::CString;
        use std::io::Read;
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::ffi::OsStrExt;

        let temporary = tempfile::tempdir().unwrap();
        let root = ConfinedRoot::open(temporary.path()).unwrap();
        std::fs::write(temporary.path().join("allowed.bin"), b"allowed").unwrap();
        let denied_path = temporary.path().join("denied.bin");
        std::fs::write(&denied_path, b"denied metadata").unwrap();
        // IN_OPEN detects the readable descriptor needed by the OpenDAL stat
        // probe. The ordinary listing below verifies this watch is effective.
        // SAFETY: supported initialization flags, returned descriptor is owned.
        let descriptor = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        assert!(descriptor >= 0);
        // SAFETY: inotify_init1 returned this newly owned descriptor.
        let mut events = unsafe { File::from_raw_fd(descriptor) };
        let denied = CString::new(denied_path.as_os_str().as_bytes()).unwrap();
        // SAFETY: valid inotify fd, NUL-terminated path, supported event mask.
        assert!(
            unsafe { libc::inotify_add_watch(events.as_raw_fd(), denied.as_ptr(), libc::IN_OPEN) }
                >= 0
        );
        let (entries, next) = root
            .list_page_with_filter("", 0, 100, |path| path != "denied.bin")
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "allowed.bin");
        assert!(next.is_none());
        let mut buffer = [0_u8; 1024];
        assert_eq!(
            events.read(&mut buffer).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        root.list_page("", 0, 100).await.unwrap();
        assert!(events.read(&mut buffer).unwrap() > 0);
    }

    #[tokio::test]
    async fn unlinked_snapshot_is_readable_and_hash_mismatch_never_publishes() {
        let mut stage = StagedFile::temporary(4).unwrap();
        stage.write_chunk(&[1, 2, 3, 4]).await.unwrap();
        stage.finish(None).await.unwrap();
        assert_eq!(
            stage.operator().read(stage.key()).await.unwrap().to_vec(),
            [1, 2, 3, 4]
        );
        let temporary = tempfile::tempdir().unwrap();
        let root = ConfinedRoot::open(temporary.path()).unwrap();
        let mut invalid = root.new_stage(4).unwrap();
        invalid.write_chunk(&[1, 2, 3, 4]).await.unwrap();
        assert!(invalid.finish(Some(&"0".repeat(64))).await.is_err());
        assert!(root.publish(&mut invalid, "output.bin").is_err());
        assert!(!temporary.path().join("output.bin").exists());
    }

    #[test]
    fn exclusive_root_lease_reaps_only_our_orphans_and_refuses_unknown_files() {
        let temporary = tempfile::tempdir().unwrap();
        let root = ConfinedRoot::open(temporary.path()).unwrap();
        assert!(ConfinedRoot::open(temporary.path()).is_err());
        let orphan = temporary
            .path()
            .join(STAGING_SEGMENT)
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::write(&orphan, b"orphaned transfer").unwrap();
        drop(root);
        let root = ConfinedRoot::open(temporary.path()).unwrap();
        assert!(!orphan.exists());
        let unknown = temporary.path().join(STAGING_SEGMENT).join("do-not-delete");
        std::fs::write(&unknown, b"unknown file").unwrap();
        drop(root);
        assert!(ConfinedRoot::open(temporary.path()).is_err());
        assert!(unknown.exists());
    }

    #[test]
    fn canonical_paths_reject_ambiguous_encoding_but_preserve_unicode() {
        for path in [
            "../alice",
            "alice/../bob",
            "alice//x",
            "alice/%2e%2e/x",
            "alice\\x",
            "alice/\0x",
            "alice/.infimount-hosted-staging/x",
        ] {
            assert!(validate_relative_path(path, false).is_err(), "{path:?}");
        }
        assert!(validate_relative_path("alice/报告.bin", false).is_ok());
    }

    #[derive(Debug, Default)]
    struct SlowWriteSignals {
        started: tokio::sync::Notify,
        release_write: tokio::sync::Notify,
        closed: tokio::sync::Notify,
        abort_started: tokio::sync::Notify,
        release_abort: tokio::sync::Notify,
        fail_abort: bool,
        stage_write: bool,
    }

    #[derive(Debug, Clone)]
    struct SlowWriteLayer(Arc<SlowWriteSignals>);
    impl opendal::raw::Layer for SlowWriteLayer {
        fn apply_service(&self, inner: opendal::raw::Servicer) -> opendal::raw::Servicer {
            Arc::new(SlowWriteService {
                inner,
                signals: self.0.clone(),
            })
        }
    }
    #[derive(Debug)]
    struct SlowWriteService {
        inner: opendal::raw::Servicer,
        signals: Arc<SlowWriteSignals>,
    }
    impl opendal::raw::Service for SlowWriteService {
        type Reader = opendal::raw::oio::Reader;
        type Writer = SlowWriter;
        type Lister = opendal::raw::oio::Lister;
        type Deleter = opendal::raw::oio::Deleter;
        type Copier = opendal::raw::oio::Copier;
        fn info(&self) -> opendal::raw::ServiceInfo {
            self.inner.info()
        }
        fn capability(&self) -> opendal::Capability {
            self.inner.capability()
        }
        async fn create_dir(
            &self,
            ctx: &opendal::OperationContext,
            path: &str,
            args: opendal::raw::OpCreateDir,
        ) -> opendal::Result<opendal::raw::RpCreateDir> {
            self.inner.create_dir(ctx, path, args).await
        }
        async fn stat(
            &self,
            ctx: &opendal::OperationContext,
            path: &str,
            args: opendal::raw::OpStat,
        ) -> opendal::Result<opendal::raw::RpStat> {
            self.inner.stat(ctx, path, args).await
        }
        fn read(
            &self,
            ctx: &opendal::OperationContext,
            path: &str,
            args: opendal::raw::OpRead,
        ) -> opendal::Result<Self::Reader> {
            self.inner.read(ctx, path, args)
        }
        fn write(
            &self,
            ctx: &opendal::OperationContext,
            path: &str,
            args: opendal::raw::OpWrite,
        ) -> opendal::Result<Self::Writer> {
            Ok(SlowWriter {
                inner: self.inner.write(ctx, path, args)?,
                signals: self.signals.clone(),
            })
        }
        fn delete(&self, ctx: &opendal::OperationContext) -> opendal::Result<Self::Deleter> {
            self.inner.delete(ctx)
        }
        fn list(
            &self,
            ctx: &opendal::OperationContext,
            path: &str,
            args: opendal::raw::OpList,
        ) -> opendal::Result<Self::Lister> {
            self.inner.list(ctx, path, args)
        }
        fn copy(
            &self,
            ctx: &opendal::OperationContext,
            from: &str,
            to: &str,
            args: opendal::raw::OpCopy,
            options: opendal::raw::OpCopier,
        ) -> opendal::Result<Self::Copier> {
            self.inner.copy(ctx, from, to, args, options)
        }
        async fn rename(
            &self,
            ctx: &opendal::OperationContext,
            from: &str,
            to: &str,
            args: opendal::raw::OpRename,
        ) -> opendal::Result<opendal::raw::RpRename> {
            self.inner.rename(ctx, from, to, args).await
        }
        async fn presign(
            &self,
            ctx: &opendal::OperationContext,
            path: &str,
            args: opendal::raw::OpPresign,
        ) -> opendal::Result<opendal::raw::RpPresign> {
            self.inner.presign(ctx, path, args).await
        }
    }
    struct SlowWriter {
        inner: opendal::raw::oio::Writer,
        signals: Arc<SlowWriteSignals>,
    }
    impl opendal::raw::oio::Write for SlowWriter {
        async fn write(&mut self, buffer: opendal::Buffer) -> opendal::Result<()> {
            if self.signals.stage_write {
                self.signals.started.notify_one();
                self.signals.release_write.notified().await;
                return self.inner.write(buffer).await;
            }
            self.inner.write(buffer).await?;
            self.signals.started.notify_one();
            futures::future::pending().await
        }
        async fn close(&mut self) -> opendal::Result<opendal::Metadata> {
            if self.signals.stage_write {
                let result = self.inner.close().await;
                self.signals.closed.notify_one();
                return result;
            }
            self.signals.started.notify_one();
            futures::future::pending().await
        }
        async fn abort(&mut self) -> opendal::Result<()> {
            self.signals.abort_started.notify_one();
            self.signals.release_abort.notified().await;
            self.inner.abort().await?;
            if self.signals.fail_abort {
                return Err(opendal::Error::new(
                    opendal::ErrorKind::Unexpected,
                    "synthetic abort failure",
                ));
            }
            Ok(())
        }
    }

    async fn cancellation_case(fail_abort: bool) {
        let mut stage = StagedFile::temporary(3).unwrap();
        stage.write_chunk(b"abc").await.unwrap();
        stage.finish(None).await.unwrap();
        let base = Operator::new(opendal::services::Memory::default()).unwrap();
        let signals = Arc::new(SlowWriteSignals {
            fail_abort,
            ..Default::default()
        });
        let destination = base.clone().layer(SlowWriteLayer(signals.clone()));
        let semaphore = Arc::new(tokio::sync::Semaphore::new(1));
        let lease = Arc::new(semaphore.clone().acquire_owned().await.unwrap());
        let weak = Arc::downgrade(&lease);
        let failed = Arc::new(AtomicBool::new(false));
        let cleanup = PublishCleanup {
            retention: Some(lease),
            failed: failed.clone(),
        };
        let task = tokio::spawn(async move {
            publish_stage_checked(
                &stage,
                &destination,
                "output.bin",
                None,
                Some(cleanup),
                || Ok(()),
            )
            .await
        });
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            signals.started.notified(),
        )
        .await
        .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            signals.abort_started.notified(),
        )
        .await
        .unwrap();
        assert!(
            weak.upgrade().is_some(),
            "quota and request permits must remain held during abort"
        );
        assert_eq!(semaphore.available_permits(), 0);
        signals.release_abort.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while weak.upgrade().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(semaphore.available_permits(), 1);
        assert_eq!(failed.load(Ordering::Acquire), fail_abort);
        assert_eq!(
            base.stat("output.bin").await.unwrap_err().kind(),
            opendal::ErrorKind::NotFound
        );
    }

    #[tokio::test]
    async fn cancelled_conditional_write_aborts_and_retains_resources_until_cleanup() {
        cancellation_case(false).await;
    }

    #[tokio::test]
    async fn cancelled_staging_write_keeps_exact_descriptor_and_resources_until_close() {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::MetadataExt;

        let mut stage = StagedFile::temporary(3).unwrap();
        let descriptor = stage.file.as_raw_fd();
        let inode = stage.file.metadata().unwrap().ino();
        let signals = Arc::new(SlowWriteSignals {
            stage_write: true,
            ..Default::default()
        });
        stage.operator = stage
            .operator
            .clone()
            .layer(SlowWriteLayer(signals.clone()));
        let semaphore = Arc::new(tokio::sync::Semaphore::new(1));
        let lease = Arc::new(semaphore.clone().acquire_owned().await.unwrap());
        let weak = Arc::downgrade(&lease);
        stage.set_resource_retention(Some(lease));
        let task = tokio::spawn(async move { stage.write_chunk(b"abc").await });
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            signals.started.notified(),
        )
        .await
        .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(weak.upgrade().is_some());
        assert_eq!(semaphore.available_permits(), 0);
        assert_eq!(
            std::fs::metadata(format!("/proc/self/fd/{descriptor}"))
                .unwrap()
                .ino(),
            inode
        );
        let another = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(another.path(), b"unrelated file").unwrap();
        assert_ne!(another.as_file().as_raw_fd(), descriptor);
        signals.release_write.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(2), signals.closed.notified())
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while weak.upgrade().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(semaphore.available_permits(), 1);
        assert_eq!(std::fs::read(another.path()).unwrap(), b"unrelated file");
    }
    #[tokio::test]
    async fn failed_cancellation_cleanup_marks_storage_unavailable() {
        cancellation_case(true).await;
    }
}
