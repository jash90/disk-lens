//! System-dependent file metadata: allocated space, device, inode, modification time.

use std::fs::Metadata;

/// Space allocated on disk.
#[cfg(unix)]
pub fn alloc_size(meta: &Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.blocks() * 512
}

/// Windows does not report blocks in `Metadata`, so we round up to the usual NTFS cluster (4 KiB).
/// This ignores compressed and sparse files.
#[cfg(not(unix))]
pub fn alloc_size(meta: &Metadata) -> u64 {
    const CLUSTER: u64 = 4096;
    meta.len().div_ceil(CLUSTER) * CLUSTER
}

/// Device (volume) identifier, used to detect mount points.
#[cfg(unix)]
pub fn device(meta: &Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(meta.dev())
}

/// On Windows mount points are reparse points, which are not followed anyway (like symlinks).
#[cfg(not(unix))]
pub fn device(_meta: &Metadata) -> Option<u64> {
    None
}

/// (device, inode) key for files with more than one hard link.
#[cfg(unix)]
pub fn hardlink_key(meta: &Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    (!meta.is_dir() && meta.nlink() > 1).then(|| (meta.dev(), meta.ino()))
}

#[cfg(not(unix))]
pub fn hardlink_key(_meta: &Metadata) -> Option<(u64, u64)> {
    None
}

/// Seconds since the Unix epoch; 0 for times before it.
fn unix_secs(t: std::time::SystemTime) -> i64 {
    t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Current time as a Unix timestamp (s).
pub fn now_secs() -> i64 {
    unix_secs(std::time::SystemTime::now())
}

/// Modification time as a Unix timestamp (s); 0 when unknown.
pub fn mtime(meta: &Metadata) -> i64 {
    meta.modified().map(unix_secs).unwrap_or(0)
}

/// Clamps Unix seconds to the `u32` the tree stores for access and creation times (0 = unknown).
pub fn clamp_secs(secs: i64) -> u32 {
    secs.clamp(0, u32::MAX as i64) as u32
}

/// Last access time as a Unix timestamp (s); 0 when the system does not report it.
pub fn atime(meta: &Metadata) -> u32 {
    meta.accessed().map(unix_secs).map(clamp_secs).unwrap_or(0)
}

/// Creation (birth) time as a Unix timestamp (s); 0 when the file system does not record it.
pub fn created(meta: &Metadata) -> u32 {
    meta.created().map(unix_secs).map(clamp_secs).unwrap_or(0)
}

/// `st_flags` bit of an iCloud placeholder: the content is not on disk.
#[cfg(target_os = "macos")]
pub const SF_DATALESS: u32 = 0x4000_0000;

/// iCloud placeholder (SF_DATALESS); macOS only.
#[cfg(target_os = "macos")]
pub fn is_dataless(meta: &Metadata) -> bool {
    use std::os::macos::fs::MetadataExt;
    meta.st_flags() & SF_DATALESS != 0
}

#[cfg(not(target_os = "macos"))]
pub fn is_dataless(_meta: &Metadata) -> bool {
    false
}

/// For the duration of a scan, reports user-initiated work to the system so that macOS does not
/// put the app to sleep (App Nap) while its window is hidden. Ordinary system sleep is still
/// allowed. Released on `Drop`.
pub struct ScanActivity {
    #[cfg(target_os = "macos")]
    token: objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2::runtime::NSObjectProtocol>>,
}

impl ScanActivity {
    pub fn begin() -> Self {
        #[cfg(target_os = "macos")]
        {
            use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};
            let token = NSProcessInfo::processInfo().beginActivityWithOptions_reason(
                NSActivityOptions::UserInitiatedAllowingIdleSystemSleep,
                &NSString::from_str("Skanowanie dysku"),
            );
            ScanActivity { token }
        }
        #[cfg(not(target_os = "macos"))]
        ScanActivity {}
    }
}

impl Drop for ScanActivity {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        // SAFETY: `token` is the object returned by beginActivityWithOptions:reason:.
        unsafe {
            objc2_foundation::NSProcessInfo::processInfo().endActivity(&self.token)
        };
    }
}
