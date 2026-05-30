// fs.rs — AgentFS FUSE implementation targeting fuser 0.17 + Rust edition 2024

pub mod allow_list;
pub mod handle_table;
pub mod inode_table;
pub mod policy;
pub mod policy_freshness;
pub mod stats;

pub use self::allow_list::CascadingAllowList;
pub use self::handle_table::HandleTable;
pub use self::inode_table::InodeTable;
pub use self::policy::Policy;
pub use self::policy_freshness::PolicyFreshnessGuard;

use self::stats::{AccessKind, OpType, StatsCollector};
use fuser::{
    Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags, Generation, INodeNo, LockOwner,
    OpenAccMode, OpenFlags, RenameFlags, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory,
    ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs, ReplyWrite, Request, WriteFlags,
};
use std::ffi::{OsStr, OsString};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};
use tracing::{debug, warn};

const TTL: Duration = Duration::from_secs(1);

// ──────────────────────────────────────────────
//  AgentIgnore
// ──────────────────────────────────────────────

pub struct AgentFS {
    pub root: PathBuf,
    /// Owns the policy + inode table behind RwLocks, and provides the
    /// read-check/write-reload freshness protocol.
    guard: PolicyFreshnessGuard,
    handles: Mutex<HandleTable>,
    stats: Option<Arc<StatsCollector>>,
}

impl AgentFS {
    pub fn new(root: PathBuf) -> Self {
        Self::with_stats(root, None)
    }

    /// Create `AgentIgnore` with an optional stats collector.
    pub fn with_stats(root: PathBuf, stats: Option<Arc<StatsCollector>>) -> Self {
        Self {
            guard: PolicyFreshnessGuard::new(&root),
            handles: Mutex::new(HandleTable::new()),
            root,
            stats,
        }
    }

    /// Override the policy freshness check interval (in seconds).
    ///
    /// Set to 0 to disable the fast-path and always re-check config mtimes.
    /// Used by tests that need immediate hot-reload detection.
    pub fn set_check_interval(&self, interval_secs: u64) {
        self.guard.set_check_interval(interval_secs);
    }

    /// Ensure the policy is fresh (fast path: atomic timestamp check).
    ///
    /// In the common case this is a near-zero-cost operation — a single
    /// `AtomicU64::load` plus a comparison.
    fn ensure_policy_fresh(&self) {
        self.guard.ensure_fresh();
    }

    pub fn is_hidden(&self, path: &Path) -> bool {
        self.ensure_policy_fresh();
        self.guard.policy_read().is_hidden(path)
    }

    pub fn real_path(&self, ino: INodeNo) -> Option<PathBuf> {
        self.guard.inodes_read().path(ino.0).cloned()
    }

    /// Resolve parent inode + child name → (canonical real path, inode).
    /// Returns `None` if the child is hidden, missing, or escapes the root.
    pub fn lookup_child(
        &self,
        parent: INodeNo,
        name: &OsStr,
        req: Option<&Request>,
    ) -> Option<(PathBuf, u64)> {
        self.ensure_policy_fresh();
        let parent_real = self.real_path(parent)?;
        let child_real = parent_real.join(name);
        let canonical = std::fs::canonicalize(&child_real).ok()?;

        if !canonical.starts_with(&self.root) {
            warn!("DENY path-escape: {child_real:?} → {canonical:?}");
            return None;
        }
        if self.is_hidden_for_request(&canonical, req) {
            debug!("DENY lookup hidden: {canonical:?}");
            return None;
        }
        let ino = self.guard.inodes_write().get_or_insert(&canonical);
        Some((canonical, ino))
    }

    /// Build a `FileAttr` from real filesystem metadata for the given inode + path.
    pub fn stat(&self, ino: INodeNo, real: &Path) -> Option<FileAttr> {
        let meta = std::fs::symlink_metadata(real).ok()?;
        let kind = if meta.is_dir() {
            FileType::Directory
        } else if meta.file_type().is_symlink() {
            FileType::Symlink
        } else {
            FileType::RegularFile
        };
        let atime = meta.accessed().unwrap_or(UNIX_EPOCH);
        let mtime = meta.modified().unwrap_or(UNIX_EPOCH);
        let ctime = UNIX_EPOCH + Duration::from_secs(meta.ctime().max(0) as u64);
        Some(FileAttr {
            ino,
            size: meta.size(),
            blocks: meta.blocks(),
            atime,
            mtime,
            ctime,
            crtime: ctime,
            kind,
            perm: meta.permissions().mode() as u16,
            nlink: meta.nlink() as u32,
            uid: meta.uid(),
            gid: meta.gid(),
            rdev: meta.rdev() as u32,
            blksize: meta.blksize() as u32,
            flags: 0,
        })
    }

    /// Access the policy under a read lock.
    pub fn policy_read(&self) -> std::sync::RwLockReadGuard<'_, Policy> {
        self.guard.policy_read()
    }

    /// Access the policy under a write lock.
    pub fn policy_write(&self) -> std::sync::RwLockWriteGuard<'_, Policy> {
        self.guard.policy_write()
    }

    /// Record a stats operation (no-op if stats collector is `None`).
    #[inline]
    fn record_op(&self, op: OpType, path: &Path, pid: u32, kind: AccessKind) {
        if let Some(ref stats) = self.stats {
            stats.record_op(op, path, pid, kind);
        }
    }

    /// Check if a path is hidden, considering request context and cascading allow lists.
    /// Also records stats for denials/bypasses.
    pub fn is_hidden_for_request(&self, path: &Path, req: Option<&Request>) -> bool {
        let policy = self.guard.policy_read();
        if let Some(req) = req {
            // Bypass check first (cheaper than gitignore matching)
            if policy.is_request_allowed(path, req) {
                // Check if path would be hidden (for bypass stats)
                if policy.is_hidden(path) {
                    self.record_op(OpType::Denied, path, req.pid(), AccessKind::Bypassed);
                }
                debug!("Bypass hiding for PID {} at {:?}", req.pid(), path);
                return false;
            }
        }

        let hidden = policy.is_hidden(path);
        if hidden {
            if let Some(req) = req {
                self.record_op(OpType::Denied, path, req.pid(), AccessKind::Denied);
            }
            debug!("DENY hidden: {path:?}");
        }
        hidden
    }
}

// ──────────────────────────────────────────────
//  FUSE trait  (fuser 0.17: &self, typed newtypes)
// ──────────────────────────────────────────────

impl Filesystem for AgentFS {
    // ── lookup ──────────────────────────────────
    fn lookup(&self, req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        match self.lookup_child(parent, name, Some(req)) {
            None => reply.error(Errno::ENOENT),
            Some((real, ino)) => {
                // Pass the canonical real path instead of bare filename
                self.record_op(OpType::Lookup, &real, req.pid(), AccessKind::Allowed);
                match self.stat(INodeNo(ino), &real) {
                    None => reply.error(Errno::ENOENT),
                    Some(attr) => reply.entry(&TTL, &attr, Generation(0)),
                }
            }
        }
    }

    // ── getattr ─────────────────────────────────
    fn getattr(&self, req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        self.ensure_policy_fresh();
        let Some(real) = self.real_path(ino) else {
            return reply.error(Errno::ENOENT);
        };
        if ino != INodeNo::ROOT && self.is_hidden_for_request(&real, Some(req)) {
            return reply.error(Errno::ENOENT);
        }
        match self.stat(ino, &real) {
            Some(attr) => {
                self.record_op(OpType::Getattr, &real, req.pid(), AccessKind::Allowed);
                reply.attr(&TTL, &attr);
            }
            None => reply.error(Errno::ENOENT),
        }
    }

    // ── readdir ─────────────────────────────────
    fn readdir(
        &self,
        req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        self.ensure_policy_fresh();
        let Some(real_dir) = self.real_path(ino) else {
            return reply.error(Errno::ENOENT);
        };
        if self.is_hidden_for_request(&real_dir, Some(req)) {
            return reply.error(Errno::ENOENT);
        }

        self.record_op(OpType::Readdir, &real_dir, req.pid(), AccessKind::Allowed);

        let entries = match std::fs::read_dir(&real_dir) {
            Ok(e) => e,
            Err(e) => return reply.error(Errno::from(e)),
        };

        let mut visible: Vec<(u64, FileType, OsString)> = Vec::new();

        // "." and ".."
        visible.push((ino.0, FileType::Directory, OsString::from(".")));
        let parent_ino = self.guard.inodes_read().parent_ino(&real_dir);
        visible.push((parent_ino, FileType::Directory, OsString::from("..")));

        for entry in entries.flatten() {
            let child_real = entry.path();

            // FAST REJECT — pure string comparison, no syscall.
            // The parent dir path is already canonical (from lookup_child or
            // real_path), so the joined child path can be checked against root
            // before calling canonicalize().
            if !child_real.starts_with(&self.root) {
                warn!("DENY path-escape in readdir: {child_real:?}");
                continue;
            }

            let canonical = match std::fs::canonicalize(&child_real) {
                Ok(p) => p,
                Err(_) => continue,
            };
            if self.is_hidden_for_request(&canonical, Some(req)) {
                continue;
            }
            let child_ino = self.guard.inodes_write().get_or_insert(&canonical);
            let kind = match entry.file_type() {
                Ok(ft) if ft.is_dir() => FileType::Directory,
                Ok(ft) if ft.is_symlink() => FileType::Symlink,
                _ => FileType::RegularFile,
            };
            visible.push((child_ino, kind, entry.file_name()));
        }

        for (i, (child_ino, kind, name)) in visible.iter().enumerate().skip(offset as usize) {
            if reply.add(INodeNo(*child_ino), (i + 1) as u64, *kind, name) {
                break;
            }
        }
        reply.ok();
    }

    // ── open ────────────────────────────────────
    fn open(&self, req: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        self.ensure_policy_fresh();
        let Some(real) = self.real_path(ino) else {
            return reply.error(Errno::ENOENT);
        };
        if self.is_hidden_for_request(&real, Some(req)) {
            return reply.error(Errno::ENOENT);
        }

        let writable = flags.acc_mode() != OpenAccMode::O_RDONLY;
        let file = std::fs::OpenOptions::new()
            .read(!writable || flags.acc_mode() == OpenAccMode::O_RDWR)
            .write(writable)
            .open(&real);

        match file {
            Ok(f) => {
                let fh = self
                    .handles
                    .lock()
                    .expect("handles Mutex poisoned — fatal process state")
                    .insert(f);
                self.record_op(OpType::Open, &real, req.pid(), AccessKind::Allowed);
                if let Some(ref stats) = self.stats {
                    stats.record_handle_open();
                }
                reply.opened(fh, FopenFlags::empty());
            }
            Err(e) => reply.error(Errno::from(e)),
        }
    }

    // ── read ────────────────────────────────────
    fn read(
        &self,
        req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        use std::os::unix::io::AsRawFd;

        let Some(real) = self.real_path(ino) else {
            return reply.error(Errno::ENOENT);
        };
        if self.is_hidden_for_request(&real, Some(req)) {
            return reply.error(Errno::ENOENT);
        }

        let handles = self
            .handles
            .lock()
            .expect("handles Mutex poisoned — fatal process state");
        let Some(file) = handles.get(fh) else {
            return reply.error(Errno::EBADF);
        };
        let fd = file.as_raw_fd();
        drop(handles);

        let mut buf = vec![0u8; size as usize];
        // SAFETY:
        // - `fd` is valid: obtained via `file.as_raw_fd()` from a `std::fs::File`
        //   returned by `self.handles.get(fh)`. That file handle was inserted during
        //   `open()` and is guaranteed live while the handle table holds it.
        // - `buf` is correctly sized (`size as usize` bytes) and its pointer is
        //   valid for writes of that length.
        // - `offset` comes from the FUSE protocol and is valid for the file.
        // - `pread` does not modify the file descriptor state (unlike `read` on
        //   a seekable fd), so no aliasing concerns.
        let n = unsafe {
            libc::pread(
                fd,
                buf.as_mut_ptr().cast(),
                size as usize,
                offset as libc::off_t,
            )
        };
        if n < 0 {
            // SAFETY:
            // `__errno_location()` returns a pointer to a thread-local `int`.
            // It is safe to dereference immediately after a failed libc call
            // because no other operation has occurred that could overwrite errno.
            reply.error(Errno::from_i32(unsafe { *libc::__errno_location() }));
        } else {
            self.record_op(OpType::Read, &real, req.pid(), AccessKind::Allowed);
            reply.data(&buf[..n as usize]);
        }
    }

    // ── write ───────────────────────────────────
    fn write(
        &self,
        req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        use std::os::unix::io::AsRawFd;

        let Some(real) = self.real_path(ino) else {
            return reply.error(Errno::ENOENT);
        };
        if self.is_hidden_for_request(&real, Some(req)) {
            return reply.error(Errno::ENOENT);
        }

        let handles = self
            .handles
            .lock()
            .expect("handles Mutex poisoned — fatal process state");
        let Some(file) = handles.get(fh) else {
            return reply.error(Errno::EBADF);
        };
        let fd = file.as_raw_fd();
        drop(handles);

        // SAFETY:
        // - `fd` is valid: same reasoning as `pread` above — it comes from a live
        //   `std::fs::File` obtained via the handle table.
        // - `data.as_ptr()` points to the buffer provided by FUSE and is valid for
        //   `data.len()` bytes.
        // - `pwrite` does not modify the file descriptor state, so no aliasing
        //   concerns with other concurrent operations on the same fd.
        let n =
            unsafe { libc::pwrite(fd, data.as_ptr().cast(), data.len(), offset as libc::off_t) };
        if n < 0 {
            // SAFETY:
            // Thread-local errno read — same rationale as the pread errno block.
            reply.error(Errno::from_i32(unsafe { *libc::__errno_location() }));
        } else {
            self.record_op(OpType::Write, &real, req.pid(), AccessKind::Allowed);
            reply.written(n as u32);
        }
    }

    // ── release ─────────────────────────────────
    fn release(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        self.handles
            .lock()
            .expect("handles Mutex poisoned — fatal process state")
            .remove(fh);
        if let Some(ref stats) = self.stats {
            stats.record_handle_close();
        }
        reply.ok();
    }

    // ── readlink ────────────────────────────────
    fn readlink(&self, req: &Request, ino: INodeNo, reply: ReplyData) {
        self.ensure_policy_fresh();
        let Some(real) = self.real_path(ino) else {
            return reply.error(Errno::ENOENT);
        };
        if self.is_hidden_for_request(&real, Some(req)) {
            return reply.error(Errno::ENOENT);
        }

        let target = match std::fs::read_link(&real) {
            Ok(t) => t,
            Err(e) => return reply.error(Errno::from(e)),
        };

        let resolved = if target.is_absolute() {
            target.clone()
        } else {
            real.parent().unwrap_or(Path::new("/")).join(&target)
        };
        if let Ok(canonical) = std::fs::canonicalize(&resolved) {
            if !canonical.starts_with(&self.root) {
                warn!("DENY symlink escape: {real:?} → {canonical:?}");
                return reply.error(Errno::ENOENT);
            }
            if self.is_hidden_for_request(&canonical, Some(req)) {
                warn!("DENY symlink to hidden: {real:?} → {canonical:?}");
                return reply.error(Errno::ENOENT);
            }
        }

        use std::os::unix::ffi::OsStrExt;
        self.record_op(OpType::Readlink, &real, req.pid(), AccessKind::Allowed);
        reply.data(target.as_os_str().as_bytes());
    }

    // ── rename ──────────────────────────────────
    fn rename(
        &self,
        req: &Request,
        parent: INodeNo,
        name: &OsStr,
        newparent: INodeNo,
        newname: &OsStr,
        _flags: RenameFlags,
        reply: ReplyEmpty,
    ) {
        self.ensure_policy_fresh();
        let Some(src_parent_real) = self.real_path(parent) else {
            return reply.error(Errno::ENOENT);
        };
        let src_real = src_parent_real.join(name);
        let src_canonical = match std::fs::canonicalize(&src_real) {
            Ok(p) => p,
            Err(_) => return reply.error(Errno::ENOENT),
        };
        if self.is_hidden_for_request(&src_canonical, Some(req)) {
            warn!("DENY rename of hidden: {src_canonical:?}");
            return reply.error(Errno::ENOENT);
        }

        let Some(dst_parent_real) = self.real_path(newparent) else {
            return reply.error(Errno::ENOENT);
        };
        let dst_real = dst_parent_real.join(newname);
        if self.is_hidden_for_request(&dst_real, Some(req)) {
            warn!("DENY rename into hidden dest: {dst_real:?}");
            return reply.error(Errno::ENOENT);
        }

        self.guard.inodes_write().evict_prefix(&src_canonical);
        match std::fs::rename(&src_canonical, &dst_real) {
            Ok(_) => {
                self.record_op(
                    OpType::Rename,
                    &src_canonical,
                    req.pid(),
                    AccessKind::Allowed,
                );
                reply.ok();
            }
            Err(e) => reply.error(Errno::from(e)),
        }
    }

    // ── link ────────────────────────────────────
    fn link(
        &self,
        req: &Request,
        ino: INodeNo,
        newparent: INodeNo,
        newname: &OsStr,
        reply: ReplyEntry,
    ) {
        self.ensure_policy_fresh();
        let Some(real) = self.real_path(ino) else {
            return reply.error(Errno::ENOENT);
        };
        if self.is_hidden_for_request(&real, Some(req)) {
            warn!("DENY hard link to hidden inode: {real:?}");
            return reply.error(Errno::ENOENT);
        }
        let Some(newparent_real) = self.real_path(newparent) else {
            return reply.error(Errno::ENOENT);
        };
        let link_real = newparent_real.join(newname);
        if self.is_hidden_for_request(&link_real, Some(req)) {
            return reply.error(Errno::ENOENT);
        }
        match std::fs::hard_link(&real, &link_real) {
            Ok(_) => {
                let canonical = std::fs::canonicalize(&link_real).unwrap_or(link_real);
                let new_ino = self.guard.inodes_write().get_or_insert(&canonical);
                self.record_op(OpType::Link, &real, req.pid(), AccessKind::Allowed);
                match self.stat(fuser::INodeNo(new_ino), &canonical) {
                    Some(attr) => reply.entry(&TTL, &attr, fuser::Generation(0)),
                    None => reply.error(Errno::ENOENT),
                }
            }
            Err(e) => reply.error(Errno::from(e)),
        }
    }

    // ── symlink ─────────────────────────────────
    fn symlink(
        &self,
        req: &Request,
        parent: INodeNo,
        link_name: &OsStr,
        target: &Path,
        reply: ReplyEntry,
    ) {
        self.ensure_policy_fresh();
        let Some(parent_real) = self.real_path(parent) else {
            return reply.error(Errno::ENOENT);
        };
        let link_real = parent_real.join(link_name);
        if self.is_hidden_for_request(&link_real, Some(req)) {
            return reply.error(Errno::ENOENT);
        }

        let resolved = if target.is_absolute() {
            target.to_path_buf()
        } else {
            parent_real.join(target)
        };
        if let Ok(canonical) = std::fs::canonicalize(&resolved)
            && (!canonical.starts_with(&self.root)
                || self.is_hidden_for_request(&canonical, Some(req)))
        {
            warn!("DENY symlink to hidden/escaped target: {canonical:?}");
            return reply.error(Errno::ENOENT);
        }

        use std::os::unix::fs as unix_fs;
        match unix_fs::symlink(target, &link_real) {
            Ok(_) => {
                let new_ino = self.guard.inodes_write().get_or_insert(&link_real);
                self.record_op(OpType::Symlink, &link_real, req.pid(), AccessKind::Allowed);
                match self.stat(fuser::INodeNo(new_ino), &link_real) {
                    Some(attr) => reply.entry(&TTL, &attr, fuser::Generation(0)),
                    None => reply.error(Errno::ENOENT),
                }
            }
            Err(e) => reply.error(Errno::from(e)),
        }
    }

    // ── create ──────────────────────────────────
    fn create(
        &self,
        req: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        _flags: i32,
        reply: ReplyCreate,
    ) {
        self.ensure_policy_fresh();
        let Some(parent_real) = self.real_path(parent) else {
            return reply.error(Errno::ENOENT);
        };
        let child_real = parent_real.join(name);
        if self.is_hidden_for_request(&child_real, Some(req)) {
            warn!("DENY create in hidden path: {child_real:?}");
            return reply.error(Errno::ENOENT);
        }
        match std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .read(true)
            .open(&child_real)
        {
            Ok(f) => {
                let ino = self.guard.inodes_write().get_or_insert(&child_real);
                let fh = self
                    .handles
                    .lock()
                    .expect("handles Mutex poisoned — fatal process state")
                    .insert(f);
                self.record_op(OpType::Create, &child_real, req.pid(), AccessKind::Allowed);
                if let Some(ref stats) = self.stats {
                    stats.record_handle_open();
                }
                match self.stat(fuser::INodeNo(ino), &child_real) {
                    Some(attr) => {
                        reply.created(&TTL, &attr, fuser::Generation(0), fh, FopenFlags::empty())
                    }
                    None => reply.error(Errno::ENOENT),
                }
            }
            Err(e) => reply.error(Errno::from(e)),
        }
    }

    // ── mkdir ───────────────────────────────────
    fn mkdir(
        &self,
        req: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        self.ensure_policy_fresh();
        let Some(parent_real) = self.real_path(parent) else {
            return reply.error(Errno::ENOENT);
        };
        let child_real = parent_real.join(name);
        if self.is_hidden_for_request(&child_real, Some(req)) {
            return reply.error(Errno::ENOENT);
        }
        match std::fs::create_dir(&child_real) {
            Ok(_) => {
                let ino = self.guard.inodes_write().get_or_insert(&child_real);
                self.record_op(OpType::Mkdir, &child_real, req.pid(), AccessKind::Allowed);
                match self.stat(fuser::INodeNo(ino), &child_real) {
                    Some(attr) => reply.entry(&TTL, &attr, fuser::Generation(0)),
                    None => reply.error(Errno::ENOENT),
                }
            }
            Err(e) => reply.error(Errno::from(e)),
        }
    }

    // ── unlink ──────────────────────────────────
    fn unlink(&self, req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        self.ensure_policy_fresh();
        let Some(parent_real) = self.real_path(parent) else {
            return reply.error(Errno::ENOENT);
        };
        let child_real = parent_real.join(name);
        let canonical = match std::fs::canonicalize(&child_real) {
            Ok(p) => p,
            Err(_) => return reply.error(Errno::ENOENT),
        };
        if self.is_hidden_for_request(&canonical, Some(req)) {
            return reply.error(Errno::ENOENT);
        }
        self.guard.inodes_write().evict_prefix(&canonical);
        match std::fs::remove_file(&canonical) {
            Ok(_) => {
                self.record_op(OpType::Unlink, &canonical, req.pid(), AccessKind::Allowed);
                reply.ok();
            }
            Err(e) => reply.error(Errno::from(e)),
        }
    }

    // ── rmdir ───────────────────────────────────
    fn rmdir(&self, req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        self.ensure_policy_fresh();
        let Some(parent_real) = self.real_path(parent) else {
            return reply.error(Errno::ENOENT);
        };
        let child_real = parent_real.join(name);
        let canonical = match std::fs::canonicalize(&child_real) {
            Ok(p) => p,
            Err(_) => return reply.error(Errno::ENOENT),
        };
        if self.is_hidden_for_request(&canonical, Some(req)) {
            return reply.error(Errno::ENOENT);
        }
        self.guard.inodes_write().evict_prefix(&canonical);
        match std::fs::remove_dir(&canonical) {
            Ok(_) => {
                self.record_op(OpType::Rmdir, &canonical, req.pid(), AccessKind::Allowed);
                reply.ok();
            }
            Err(e) => reply.error(Errno::from(e)),
        }
    }

    // ── statfs ──────────────────────────────────
    fn statfs(&self, _req: &Request, _ino: INodeNo, reply: ReplyStatfs) {
        use nix::sys::statvfs::statvfs;
        self.record_op(OpType::Statfs, &self.root, 0, AccessKind::Allowed);
        match statvfs(&self.root) {
            Ok(s) => reply.statfs(
                s.blocks(),
                s.blocks_free(),
                s.blocks_available(),
                s.files(),
                s.files_free(),
                s.block_size() as u32,
                s.name_max() as u32,
                s.fragment_size() as u32,
            ),
            Err(_) => reply.statfs(0, 0, 0, 0, 0, 512, 255, 512),
        }
    }
}
