//! Odczyt zawartości folderu razem z metadanymi wpisów.
//!
//! Domyślnie `read_dir` + `lstat` na każdy wpis. Na macOS (APFS, HFS+) `getattrlistbulk(2)` zwraca
//! nazwy i atrybuty paczkami – jedno wywołanie systemowe na kilkaset wpisów zamiast jednego na wpis,
//! co przy skanie dysku jest głównym kosztem (czas spędzony w jądrze). Na SMB, NTFS czy exFAT
//! `getattrlistbulk` bywa wolniejszy od `readdir`, więc tam zostaje ścieżka standardowa.

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

/// Wpis folderu z metadanymi potrzebnymi skanerowi (odpowiednik `lstat`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawEntry<'a> {
    pub name: &'a OsStr,
    pub kind: FileKind,
    pub alloc: u64,
    pub logical: u64,
    pub mtime: i64,
    pub dev: Option<u64>,
    /// (urządzenie, i-węzeł) – tylko pliki z więcej niż jednym twardym dowiązaniem.
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
            dev: platform::device(meta),
            hardlink: platform::hardlink_key(meta),
            dataless: platform::is_dataless(meta),
        }
    }
}

/// Błąd odczytu pojedynczego wpisu (`name`) albo dalszej części folderu (`None`).
#[derive(Debug)]
pub struct EntryError<'a> {
    pub name: Option<&'a OsStr>,
    pub error: io::Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// `read_dir` + `lstat` na każdy wpis – działa wszędzie.
    Std,
    /// `getattrlistbulk` – tylko macOS, lokalne APFS/HFS+.
    Bulk,
}

/// Najszybszy sposób odczytu dla wolumenu, na którym leży `path`.
pub fn backend_for(path: &Path) -> Backend {
    #[cfg(target_os = "macos")]
    if bulk::supported(path) {
        return Backend::Bulk;
    }
    let _ = path;
    Backend::Std
}

/// Wywołuje `f` dla każdego wpisu folderu `path` (bez „.” i „..”); `f` zwraca `false`, żeby przerwać.
/// Błąd zwracany jest tylko wtedy, gdy folderu nie da się otworzyć.
///
/// Folder jest czytany do końca i zamykany przed powrotem – wywołujący nie trzyma kilku otwartych
/// iteracji naraz (zagnieżdżone iteracje gubią stan na niektórych serwerach SMB).
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
        // DirEntry::metadata na Unixie nie podąża za symlinkami (lstat); na Windows nie kosztuje
        // dodatkowego wywołania – dane przychodzą z FindNextFileW.
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

    /// Brak w `libc`: kod błędu odczytu atrybutów konkretnego wpisu.
    const ATTR_CMN_ERROR: u32 = 0x2000_0000;
    const SF_DATALESS: u32 = 0x4000_0000;
    // Typy obiektów (`enum vtype` z <sys/vnode.h>).
    const VREG: u32 = 1;
    const VDIR: u32 = 2;
    const VLNK: u32 = 5;
    /// Bufor na paczkę wpisów; `fts` używa 32 KiB, większy oznacza mniej wywołań.
    const BUF_SIZE: usize = 256 * 1024;

    thread_local! {
        static BUF: RefCell<Vec<u8>> = RefCell::new(vec![0u8; BUF_SIZE]);
    }

    /// Systemy plików, na których `getattrlistbulk` jest szybszy niż `readdir` + `lstat`.
    pub fn supported(path: &Path) -> bool {
        let Ok(c) = CString::new(path.as_os_str().as_bytes()) else { return false };
        let mut s: libc::statfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c` jest zakończone zerem, `s` to zainicjalizowany bufor.
        if unsafe { libc::statfs(c.as_ptr(), &mut s) } != 0 {
            return false;
        }
        // SAFETY: jądro zwraca nazwę zakończoną zerem w tablicy o stałym rozmiarze.
        let fs = unsafe { CStr::from_ptr(s.f_fstypename.as_ptr()) };
        matches!(fs.to_bytes(), b"apfs" | b"hfs")
    }

    fn attrlist() -> libc::attrlist {
        // SAFETY: struktura z samych liczb, zero to poprawna wartość.
        let mut al: libc::attrlist = unsafe { std::mem::zeroed() };
        al.bitmapcount = libc::ATTR_BIT_MAP_COUNT;
        al.commonattr = libc::ATTR_CMN_RETURNED_ATTRS
            | ATTR_CMN_ERROR
            | libc::ATTR_CMN_NAME
            | libc::ATTR_CMN_DEVID
            | libc::ATTR_CMN_OBJTYPE
            | libc::ATTR_CMN_MODTIME
            | libc::ATTR_CMN_FLAGS
            | libc::ATTR_CMN_FILEID;
        al.dirattr = libc::ATTR_DIR_MOUNTSTATUS | libc::ATTR_DIR_ALLOCSIZE | libc::ATTR_DIR_DATALENGTH;
        al.fileattr = libc::ATTR_FILE_LINKCOUNT | libc::ATTR_FILE_ALLOCSIZE | libc::ATTR_FILE_DATALENGTH;
        al
    }

    /// Kursor po rekordzie wpisu; atrybuty leżą kolejno, wyrównane do 4 bajtów, bez gwarancji wyrównania do 8.
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
            // SAFETY: sprawdzone wyżej, że `size` bajtów mieści się w rekordzie.
            unsafe {
                let v = std::ptr::read_unaligned(self.p as *const T);
                self.p = self.p.add(size);
                Some(v)
            }
        }
    }

    enum Parsed<'a> {
        Entry(RawEntry<'a>),
        /// Rekord niepełny albo punkt montowania – dane trzeba wziąć z `lstat`.
        Fallback(&'a OsStr),
        Error(&'a OsStr, io::Error),
    }

    /// Dekoduje jeden rekord; `rec` obejmuje dokładnie `length` bajtów rekordu.
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
        // Długość obejmuje kończące zero.
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
        // `dev_t` to i32; rzutowanie jak w `MetadataExt::dev`, żeby klucze były zgodne z `lstat`.
        let dev = c.take::<libc::dev_t>()? as u64;
        let objtype: u32 = c.take()?;
        let mtime: libc::timespec = c.take()?;
        let fl: u32 = c.take()?;
        let fileid: u64 = c.take()?;

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
            // Jak `platform::mtime`: czasy sprzed 1970 jako 0.
            mtime: mtime.tv_sec.max(0),
            dev: Some(dev),
            hardlink: (kind != FileKind::Dir && nlink > 1).then_some((dev, fileid)),
            dataless: fl & SF_DATALESS != 0,
        }))
    }

    pub fn read_dir(path: &Path, mut f: impl FnMut(Result<RawEntry<'_>, EntryError<'_>>) -> bool) -> io::Result<()> {
        let c = CString::new(path.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        // SAFETY: `c` jest zakończone zerem.
        let raw = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` to świeżo otwarty deskryptor, którego nikt inny nie posiada.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let mut al = attrlist();

        BUF.with(|buf| {
            let mut buf = buf.borrow_mut();
            loop {
                // SAFETY: wskaźniki na lokalną `attrlist` i bufor o podanej długości.
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
                    let len = u32::from_ne_bytes(len_bytes.try_into().expect("4 bajty")) as usize;
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
                            error: io::Error::new(io::ErrorKind::InvalidData, "niepoprawny rekord getattrlistbulk"),
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
