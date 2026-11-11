//! The file-system event log of macOS (FSEvents): which folders changed since a given moment.
//! The system keeps this log per volume, also while the app is not running, so a tree saved
//! earlier can be brought up to date by listing only those folders (see `refresh`).
//!
//! Everything here can fail or be unavailable (another system, a volume without a log, a log
//! that was reset, too much history). The caller then falls back to a full scan.

use crate::refresh::Change;
use std::path::Path;
use std::time::Duration;

/// A point in a volume's event log.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Position {
    pub event_id: u64,
    /// Identity of the log; a different value means the ids cannot be compared.
    pub uuid: String,
}

/// More ids than this between two points and the replay takes longer than it saves
/// (about 3 s per million on the machine it was measured on).
pub const MAX_ID_DISTANCE: u64 = 4_000_000;
/// More changed folders than this and a full scan is the safer choice.
const MAX_CHANGES: usize = 150_000;

/// Where the log of the volume holding `path` stands now. `None`: no usable log.
pub fn position(path: &Path) -> Option<Position> {
    imp::position(path)
}

/// Folders under `root` whose content changed after `since`. `Err` says why the log cannot
/// answer; nothing can be concluded from the tree then. `give_up` is asked several times a
/// second whether to stop waiting.
pub fn changes_since(root: &Path, since: &Position, timeout: Duration, give_up: &dyn Fn() -> bool) -> Result<Vec<Change>, String> {
    let now = position(root).ok_or("no event log for this volume")?;
    if since.event_id == 0 || since.uuid.is_empty() || now.uuid != since.uuid {
        return Err("the event log was reset since the last scan".into());
    }
    if now.event_id < since.event_id {
        return Err("event ids went backwards".into());
    }
    if now.event_id - since.event_id > MAX_ID_DISTANCE {
        return Err("too much happened since the last scan".into());
    }
    imp::changes_since(root, since.event_id, timeout, give_up)
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::*;

    pub fn position(_: &Path) -> Option<Position> {
        None
    }

    pub fn changes_since(_: &Path, _: u64, _: Duration, _: &dyn Fn() -> bool) -> Result<Vec<Change>, String> {
        Err("no event log on this system".into())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use std::collections::HashMap;
    use std::ffi::{c_char, c_void, CStr, OsStr};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::path::PathBuf;
    use std::sync::mpsc::{sync_channel, SyncSender};
    use std::sync::Mutex;

    type CfRef = *const c_void;
    type Callback = extern "C" fn(CfRef, *mut c_void, usize, *mut c_void, *const u32, *const u64);

    #[repr(C)]
    struct StreamContext {
        version: isize,
        info: *mut c_void,
        retain: *const c_void,
        release: *const c_void,
        copy_description: *const c_void,
    }

    const UTF8: u32 = 0x0800_0100;
    const MUST_SCAN_SUBDIRS: u32 = 0x01;
    const USER_DROPPED: u32 = 0x02;
    const KERNEL_DROPPED: u32 = 0x04;
    const IDS_WRAPPED: u32 = 0x08;
    const HISTORY_DONE: u32 = 0x10;
    const ROOT_CHANGED: u32 = 0x20;
    const MOUNT: u32 = 0x40;
    const UNMOUNT: u32 = 0x80;

    #[link(name = "CoreServices", kind = "framework")]
    extern "C" {
        fn FSEventsGetCurrentEventId() -> u64;
        fn FSEventsCopyUUIDForDevice(dev: i32) -> CfRef;
        fn FSEventStreamCreate(alloc: CfRef, cb: Callback, ctx: *const StreamContext, paths: CfRef, since: u64, latency: f64, flags: u32) -> CfRef;
        fn FSEventStreamSetDispatchQueue(stream: CfRef, queue: *mut c_void);
        fn FSEventStreamStart(stream: CfRef) -> u8;
        fn FSEventStreamFlushSync(stream: CfRef);
        fn FSEventStreamStop(stream: CfRef);
        fn FSEventStreamInvalidate(stream: CfRef);
        fn FSEventStreamRelease(stream: CfRef);
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFTypeArrayCallBacks: c_void;
        fn CFUUIDCreateString(alloc: CfRef, uuid: CfRef) -> CfRef;
        fn CFStringCreateWithBytes(alloc: CfRef, bytes: *const u8, len: isize, encoding: u32, external: u8) -> CfRef;
        fn CFStringGetCString(s: CfRef, buf: *mut c_char, size: isize, encoding: u32) -> u8;
        fn CFArrayCreate(alloc: CfRef, values: *const CfRef, count: isize, callbacks: *const c_void) -> CfRef;
        fn CFRelease(obj: CfRef);
    }

    extern "C" {
        fn dispatch_queue_create(label: *const c_char, attr: *const c_void) -> *mut c_void;
        fn dispatch_sync_f(queue: *mut c_void, ctx: *mut c_void, work: extern "C" fn(*mut c_void));
        fn dispatch_release(obj: *mut c_void);
    }

    pub fn position(path: &Path) -> Option<Position> {
        let dev = std::fs::metadata(path).ok()?.dev();
        // SAFETY: plain C calls; the two objects created here are released before returning.
        unsafe {
            // Taken before the UUID: if the log is reset in between, the pair fails to match later.
            let event_id = FSEventsGetCurrentEventId();
            let uuid_ref = FSEventsCopyUUIDForDevice(dev as i32);
            if uuid_ref.is_null() {
                return None;
            }
            let text = CFUUIDCreateString(std::ptr::null(), uuid_ref);
            CFRelease(uuid_ref);
            if text.is_null() {
                return None;
            }
            let mut buf = [0 as c_char; 64];
            let ok = CFStringGetCString(text, buf.as_mut_ptr(), buf.len() as isize, UTF8);
            CFRelease(text);
            if ok == 0 || event_id == 0 {
                return None;
            }
            let uuid = CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned();
            Some(Position { event_id, uuid })
        }
    }

    #[derive(Default)]
    struct Collected {
        /// folder → changed in depth
        dirs: HashMap<PathBuf, bool>,
        failed: Option<&'static str>,
    }

    struct Shared {
        collected: Mutex<Collected>,
        history_done: SyncSender<()>,
    }

    extern "C" fn on_events(_: CfRef, info: *mut c_void, count: usize, paths: *mut c_void, flags: *const u32, _: *const u64) {
        // SAFETY: `info` is the `Shared` that outlives the stream (see `changes_since`); the
        // arrays hold `count` items for the duration of the call, paths being C strings.
        let (shared, paths, flags) = unsafe {
            (&*(info as *const Shared), std::slice::from_raw_parts(paths as *const *const c_char, count), std::slice::from_raw_parts(flags, count))
        };
        let mut c = shared.collected.lock().unwrap_or_else(|e| e.into_inner());
        for (&path, &flag) in paths.iter().zip(flags) {
            if flag & HISTORY_DONE != 0 {
                let _ = shared.history_done.try_send(());
                continue;
            }
            if flag & (USER_DROPPED | KERNEL_DROPPED) != 0 {
                c.failed = Some("the system dropped events");
            } else if flag & IDS_WRAPPED != 0 {
                c.failed = Some("event ids wrapped");
            } else if flag & ROOT_CHANGED != 0 {
                c.failed = Some("the scanned folder was moved");
            }
            if c.failed.is_some() || c.dirs.len() > MAX_CHANGES {
                c.failed.get_or_insert("too many changed folders");
                continue;
            }
            // SAFETY: see above.
            let bytes = unsafe { CStr::from_ptr(path) }.to_bytes();
            let mut dir = Path::new(OsStr::from_bytes(bytes));
            // A volume that came or went changes what its mount point's parent shows.
            if flag & (MOUNT | UNMOUNT) != 0 {
                dir = dir.parent().unwrap_or(dir);
            }
            // `Path` ignores the trailing slash the log puts on folders.
            let deep = flag & MUST_SCAN_SUBDIRS != 0;
            match c.dirs.get_mut(dir) {
                Some(d) => *d |= deep,
                None => {
                    c.dirs.insert(dir.components().collect(), deep);
                }
            }
        }
    }

    extern "C" fn nothing(_: *mut c_void) {}

    pub fn changes_since(root: &Path, since: u64, timeout: Duration, give_up: &dyn Fn() -> bool) -> Result<Vec<Change>, String> {
        // The log speaks in resolved paths (/private/var for /var).
        let watched = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
        let bytes = watched.as_os_str().as_bytes();
        let (done_tx, done_rx) = sync_channel(1);
        let shared = Box::new(Shared { collected: Mutex::default(), history_done: done_tx });

        // SAFETY: every object created is released below; `shared` outlives the stream because
        // the queue is drained (`dispatch_sync_f`) after the stream is invalidated and before
        // `shared` is dropped, so no callback can still be running.
        let finished = unsafe {
            let path = CFStringCreateWithBytes(std::ptr::null(), bytes.as_ptr(), bytes.len() as isize, UTF8, 0);
            if path.is_null() {
                return Err("path is not valid text".into());
            }
            let paths = CFArrayCreate(std::ptr::null(), &path, 1, &kCFTypeArrayCallBacks);
            let ctx = StreamContext {
                version: 0,
                info: &*shared as *const Shared as *mut c_void,
                retain: std::ptr::null(),
                release: std::ptr::null(),
                copy_description: std::ptr::null(),
            };
            let stream = FSEventStreamCreate(std::ptr::null(), on_events, &ctx, paths, since, 0.0, 0);
            CFRelease(paths);
            CFRelease(path);
            if stream.is_null() {
                return Err("could not open the event log".into());
            }
            let queue = dispatch_queue_create(c"disk-lens.fsevents".as_ptr(), std::ptr::null());
            FSEventStreamSetDispatchQueue(stream, queue);
            let finished = if FSEventStreamStart(stream) != 0 {
                let deadline = std::time::Instant::now() + timeout;
                let mut finished = false;
                while !finished && std::time::Instant::now() < deadline && !give_up() {
                    finished = done_rx.recv_timeout(Duration::from_millis(100)).is_ok();
                }
                if finished {
                    // Events newer than the stored history but not handed over yet.
                    FSEventStreamFlushSync(stream);
                }
                FSEventStreamStop(stream);
                finished
            } else {
                false
            };
            FSEventStreamInvalidate(stream);
            FSEventStreamRelease(stream);
            dispatch_sync_f(queue, std::ptr::null_mut(), nothing);
            dispatch_release(queue);
            finished
        };

        let collected = std::mem::take(&mut *shared.collected.lock().unwrap_or_else(|e| e.into_inner()));
        if let Some(reason) = collected.failed {
            return Err(reason.into());
        }
        if !finished {
            return Err("reading the event log was stopped or took too long".into());
        }
        let mut changes = Vec::with_capacity(collected.dirs.len());
        for (dir, recursive) in collected.dirs {
            // Back to the spelling of the root the tree was scanned with.
            let Ok(rel) = dir.strip_prefix(&watched) else { continue };
            changes.push(Change { path: root.join(rel), recursive });
        }
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(changes)
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::fs;

    fn wait_for(root: &Path, since: &Position, wanted: &Path) -> Vec<Change> {
        // The log is written with a short delay; give it a few tries.
        let mut last = Vec::new();
        for _ in 0..20 {
            last = changes_since(root, since, Duration::from_secs(20), &|| false).expect("event log readable");
            if last.iter().any(|c| c.path == wanted) {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        last
    }

    #[test]
    fn reports_folders_changed_after_a_position_and_only_those() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("quiet")).unwrap();
        fs::create_dir_all(root.join("busy/inner")).unwrap();
        fs::write(root.join("quiet/old.txt"), b"x").unwrap();
        std::thread::sleep(Duration::from_millis(1500));

        let Some(since) = position(root) else { return };
        fs::write(root.join("busy/inner/new.txt"), b"hello").unwrap();
        let changes = wait_for(root, &since, &root.join("busy/inner"));
        let paths: Vec<&Path> = changes.iter().map(|c| c.path.as_path()).collect();
        assert!(paths.contains(&root.join("busy/inner").as_path()), "{paths:?}");
        assert!(!paths.contains(&root.join("quiet").as_path()), "{paths:?}");
        assert!(changes.iter().all(|c| c.path.starts_with(root)), "paths use the root's own spelling: {paths:?}");
    }

    #[test]
    fn a_position_from_another_log_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let Some(mut since) = position(dir.path()) else { return };
        since.uuid = "00000000-0000-0000-0000-000000000000".into();
        assert!(changes_since(dir.path(), &since, Duration::from_secs(5), &|| false).is_err());
        let far = Position { event_id: 1, uuid: position(dir.path()).unwrap().uuid };
        assert!(changes_since(dir.path(), &far, Duration::from_secs(5), &|| false).is_err());
        assert!(changes_since(dir.path(), &Position::default(), Duration::from_secs(5), &|| false).is_err());
    }
}
