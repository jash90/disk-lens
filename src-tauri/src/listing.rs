//! Reading a folder's content together with the metadata of its entries.
//!
//! By default `read_dir` + `lstat` per entry. On macOS (APFS, HFS+) `getattrlistbulk(2)` returns
//! names and attributes in batches: one system call per few hundred entries instead of one per
//! entry, which is the main cost of a disk scan (time spent in the kernel). On SMB, NTFS or exFAT
//! `getattrlistbulk` can be slower than `readdir`, so the standard path stays in use there.

use crate::platform;
use crate::scanner::stats;
use std::ffi::OsStr;
use std::fs::{self, Metadata};
use std::io;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    File,
    Dir,
    Symlink,
    Other,
}

/// A folder entry with the metadata the scanner needs (the equivalent of `lstat`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawEntry<'a> {
    pub name: &'a OsStr,
    pub kind: FileKind,
    pub alloc: u64,
    pub logical: u64,
    pub mtime: i64,
    /// Last access, Unix seconds; 0 when unknown.
    pub atime: u32,
    /// Creation (birth) time, Unix seconds; 0 when unknown.
    pub btime: u32,
    /// When the entry was added to its folder (macOS bulk reads only), Unix seconds; 0 when unknown.
    pub added: u32,
    pub dev: Option<u64>,
    /// (device, inode): only files with more than one hard link.
    pub hardlink: Option<(u64, u64)>,
    pub dataless: bool,
}

impl<'a> RawEntry<'a> {
    pub fn from_meta(name: &'a OsStr, meta: &Metadata) -> Self {
        let ft = meta.file_type();
        let kind = if ft.is_dir() {
            FileKind::Dir
        } else if ft.is_symlink() {
            FileKind::Symlink
        } else if ft.is_file() {
            FileKind::File
        } else {
            FileKind::Other
        };
        RawEntry {
            name,
            kind,
            alloc: platform::alloc_size(meta),
            logical: meta.len(),
            mtime: platform::mtime(meta),
            atime: platform::atime(meta),
            btime: platform::created(meta),
            added: 0,
            dev: platform::device(meta),
            hardlink: platform::hardlink_key(meta),
            dataless: platform::is_dataless(meta),
        }
    }
}

/// A failure to read one entry (`name`) or the rest of the folder (`None`).
#[derive(Debug)]
pub struct EntryError<'a> {
    pub name: Option<&'a OsStr>,
    pub error: io::Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// `read_dir` + `lstat` per entry: works everywhere.
    Std,
    /// `getattrlistbulk`: macOS only, local APFS/HFS+.
    Bulk,
}

/// The fastest way to read on the volume `path` lives on.
pub fn backend_for(path: &Path) -> Backend {
    #[cfg(target_os = "macos")]
    if bulk::supported(path) {
        return Backend::Bulk;
    }
    let _ = path;
    Backend::Std
}

/// Calls `f` for every entry of the folder `path` (without "." and ".."); `f` returns `false` to stop.
/// An error is returned only when the folder cannot be opened.
///
/// The folder is read to the end and closed before returning, so the caller never holds several
/// open iterations at once (nested iterations lose their state on some SMB servers).
pub fn read_dir(path: &Path, backend: Backend, f: impl FnMut(Result<RawEntry<'_>, EntryError<'_>>) -> bool) -> io::Result<()> {
    match backend {
        #[cfg(target_os = "macos")]
        Backend::Bulk => bulk::read_dir(path, f),
        _ => read_dir_std(path, f),
    }
}

fn read_dir_std(path: &Path, mut f: impl FnMut(Result<RawEntry<'_>, EntryError<'_>>) -> bool) -> io::Result<()> {
    for item in fs::read_dir(path)? {
        let item = match item {
            Ok(i) => i,
            Err(error) => {
                if !f(Err(EntryError { name: None, error })) {
                    break;
                }
                continue;
            }
        };
        let name = item.file_name();
        // DirEntry::metadata does not follow symlinks on Unix (lstat); on Windows it costs no
        // extra call, the data comes with FindNextFileW.
        let timer = stats::Timer::start();
        let meta = item.metadata();
        timer.stop(&stats::METADATA);
        let res = match meta {
            Ok(m) => Ok(RawEntry::from_meta(&name, &m)),
            Err(error) => Err(EntryError { name: Some(&name), error }),
        };
        if !f(res) {
            break;
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
mod bulk {
    use super::{EntryError, FileKind, RawEntry};
    use crate::scanner::stats;
    use std::cell::RefCell;
    use std::ffi::{CStr, CString, OsStr};
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    /// Missing from `libc`: the error code of reading one entry's attributes.
    const ATTR_CMN_ERROR: u32 = 0x2000_0000;
    // Object types (`enum vtype` from <sys/vnode.h>).
    const VREG: u32 = 1;
    const VDIR: u32 = 2;
    const VLNK: u32 = 5;
    /// Buffer for a batch of entries; `fts` uses 32 KiB, a larger one means fewer calls.
    const BUF_SIZE: usize = 256 * 1024;

    thread_local! {
        static BUF: RefCell<Vec<u8>> = RefCell::new(vec![0u8; BUF_SIZE]);
    }

    /// File systems on which `getattrlistbulk` is faster than `readdir` + `lstat`.
    pub fn supported(path: &Path) -> bool {
        let Ok(c) = CString::new(path.as_os_str().as_bytes()) else { return false };
        let mut s: libc::statfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c` is NUL-terminated, `s` is an initialised buffer.
        if unsafe { libc::statfs(c.as_ptr(), &mut s) } != 0 {
            return false;
        }
        // SAFETY: the kernel returns a NUL-terminated name in a fixed-size array.
        let fs = unsafe { CStr::from_ptr(s.f_fstypename.as_ptr()) };
        matches!(fs.to_bytes(), b"apfs" | b"hfs")
    }

    fn attrlist() -> libc::attrlist {
        // SAFETY: a struct of plain numbers, zero is a valid value.
        let mut al: libc::attrlist = unsafe { std::mem::zeroed() };
        al.bitmapcount = libc::ATTR_BIT_MAP_COUNT;
        al.commonattr = libc::ATTR_CMN_RETURNED_ATTRS
            | ATTR_CMN_ERROR
            | libc::ATTR_CMN_NAME
            | libc::ATTR_CMN_DEVID
            | libc::ATTR_CMN_OBJTYPE
            | libc::ATTR_CMN_CRTIME
            | libc::ATTR_CMN_MODTIME
            | libc::ATTR_CMN_ACCTIME
            | libc::ATTR_CMN_FLAGS
            | libc::ATTR_CMN_FILEID
            | libc::ATTR_CMN_ADDEDTIME;
        al.dirattr = libc::ATTR_DIR_MOUNTSTATUS | libc::ATTR_DIR_ALLOCSIZE | libc::ATTR_DIR_DATALENGTH;
        al.fileattr = libc::ATTR_FILE_LINKCOUNT | libc::ATTR_FILE_ALLOCSIZE | libc::ATTR_FILE_DATALENGTH;
        al
    }

    /// Cursor over an entry record; attributes lie in sequence, aligned to 4 bytes, with no guarantee of 8-byte alignment.
    struct Cursor {
        p: *const u8,
        end: *const u8,
    }

    impl Cursor {
        fn take<T: Copy>(&mut self) -> Option<T> {
            let size = std::mem::size_of::<T>();
            if (self.end as usize).saturating_sub(self.p as usize) < size {
                return None;
            }
            // SAFETY: checked above that `size` bytes fit in the record.
            unsafe {
                let v = std::ptr::read_unaligned(self.p as *const T);
                self.p = self.p.add(size);
                Some(v)
            }
        }
    }

    enum Parsed<'a> {
        Entry(RawEntry<'a>),
        /// Incomplete record or a mount point: the data has to come from `lstat`.
        Fallback(&'a OsStr),
        Error(&'a OsStr, io::Error),
    }

    /// Decodes one record; `rec` covers exactly the `length` bytes of the record.
    ///
    /// Attributes are packed in the order of their bits in `<sys/attr.h>`, except the error code,
    /// which comes first.
    fn parse(rec: &[u8]) -> Option<Parsed<'_>> {
        let mut c = Cursor { p: rec.as_ptr(), end: rec.as_ptr_range().end };
        let _len: u32 = c.take()?;
        let ret: libc::attribute_set_t = c.take()?;
        let has = |group: u32, bit: u32| group & bit != 0;

        let err: u32 = if has(ret.commonattr, ATTR_CMN_ERROR) { c.take()? } else { 0 };
        if !has(ret.commonattr, libc::ATTR_CMN_NAME) {
            return None;
        }
        let name_at = c.p as usize - rec.as_ptr() as usize;
        let name_ref: libc::attrreference_t = c.take()?;
        let start = name_at.checked_add_signed(name_ref.attr_dataoffset as isize)?;
        let bytes = rec.get(start..start.checked_add(name_ref.attr_length as usize)?)?;
        // The length includes the terminating NUL.
        let name = OsStr::from_bytes(bytes.split(|&b| b == 0).next().unwrap_or(bytes));
        if err != 0 {
            return Some(Parsed::Error(name, io::Error::from_raw_os_error(err as i32)));
        }

        let common = [
            libc::ATTR_CMN_DEVID,
            libc::ATTR_CMN_OBJTYPE,
            libc::ATTR_CMN_MODTIME,
            libc::ATTR_CMN_FLAGS,
            libc::ATTR_CMN_FILEID,
        ];
        if common.iter().any(|&b| !has(ret.commonattr, b)) {
            return Some(Parsed::Fallback(name));
        }
        // Optional times: 0 ("unknown") when the file system did not return them.
        let secs = |t: libc::timespec| crate::platform::clamp_secs(t.tv_sec);
        // `dev_t` is i32; cast as `MetadataExt::dev` does, so that keys agree with `lstat`.
        let dev = c.take::<libc::dev_t>()? as u64;
        let objtype: u32 = c.take()?;
        let btime = if has(ret.commonattr, libc::ATTR_CMN_CRTIME) { secs(c.take()?) } else { 0 };
        let mtime: libc::timespec = c.take()?;
        let atime = if has(ret.commonattr, libc::ATTR_CMN_ACCTIME) { secs(c.take()?) } else { 0 };
        let fl: u32 = c.take()?;
        let fileid: u64 = c.take()?;
        let added = if has(ret.commonattr, libc::ATTR_CMN_ADDEDTIME) { secs(c.take()?) } else { 0 };

        let kind = match objtype {
            VREG => FileKind::File,
            VDIR => FileKind::Dir,
            VLNK => FileKind::Symlink,
            _ => FileKind::Other,
        };
        let (alloc, logical, nlink) = if kind == FileKind::Dir {
            let need = libc::ATTR_DIR_MOUNTSTATUS | libc::ATTR_DIR_ALLOCSIZE | libc::ATTR_DIR_DATALENGTH;
            if ret.dirattr & need != need {
                return Some(Parsed::Fallback(name));
            }
            let mount: u32 = c.take()?;
            if mount & libc::DIR_MNTSTATUS_MNTPOINT != 0 {
                return Some(Parsed::Fallback(name));
            }
            let alloc: libc::off_t = c.take()?;
            let len: libc::off_t = c.take()?;
            (alloc, len, 1)
        } else {
            let need = libc::ATTR_FILE_LINKCOUNT | libc::ATTR_FILE_ALLOCSIZE | libc::ATTR_FILE_DATALENGTH;
            if ret.fileattr & need != need {
                return Some(Parsed::Fallback(name));
            }
            let nlink: u32 = c.take()?;
            let alloc: libc::off_t = c.take()?;
            let len: libc::off_t = c.take()?;
            (alloc, len, nlink)
        };
        Some(Parsed::Entry(RawEntry {
            name,
            kind,
            alloc: alloc.max(0) as u64,
            logical: logical.max(0) as u64,
            // Like `platform::mtime`: times before 1970 become 0.
            mtime: mtime.tv_sec.max(0),
            atime,
            btime,
            added,
            dev: Some(dev),
            hardlink: (kind != FileKind::Dir && nlink > 1).then_some((dev, fileid)),
            dataless: fl & crate::platform::SF_DATALESS != 0,
        }))
    }

    pub fn read_dir(path: &Path, mut f: impl FnMut(Result<RawEntry<'_>, EntryError<'_>>) -> bool) -> io::Result<()> {
        let c = CString::new(path.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        // SAFETY: `c` is NUL-terminated.
        let raw = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` is a freshly opened descriptor nobody else owns.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let mut al = attrlist();

        BUF.with(|buf| {
            let mut buf = buf.borrow_mut();
            loop {
                // SAFETY: pointers to a local `attrlist` and a buffer of the given length.
                let n = unsafe {
                    libc::getattrlistbulk(fd.as_raw_fd(), (&mut al as *mut libc::attrlist).cast(), buf.as_mut_ptr().cast(), buf.len(), 0)
                };
                if n == 0 {
                    return Ok(());
                }
                if n < 0 {
                    let error = io::Error::last_os_error();
                    if error.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    f(Err(EntryError { name: None, error }));
                    return Ok(());
                }
                let mut off = 0usize;
                for _ in 0..n {
                    let Some(len_bytes) = buf.get(off..off + 4) else { break };
                    let len = u32::from_ne_bytes(len_bytes.try_into().expect("4 bytes")) as usize;
                    let Some(rec) = buf.get(off..off + len).filter(|_| len >= 4) else { break };
                    off += len;
                    let keep_going = match parse(rec) {
                        Some(Parsed::Entry(e)) => f(Ok(e)),
                        Some(Parsed::Fallback(name)) => {
                            let timer = stats::Timer::start();
                            let meta = std::fs::symlink_metadata(path.join(name));
                            timer.stop(&stats::METADATA);
                            match meta {
                                Ok(m) => f(Ok(RawEntry::from_meta(name, &m))),
                                Err(error) => f(Err(EntryError { name: Some(name), error })),
                            }
                        }
                        Some(Parsed::Error(name, error)) => f(Err(EntryError { name: Some(name), error })),
                        None => f(Err(EntryError {
                            name: None,
                            error: io::Error::new(io::ErrorKind::InvalidData, "malformed getattrlistbulk record"),
                        })),
                    };
                    if !keep_going {
                        return Ok(());
                    }
                }
            }
        })
    }
}
