//! Metadane plików zależne od systemu: zajęte miejsce, urządzenie, i-węzeł, czas modyfikacji.

use std::fs::Metadata;

/// Miejsce zajęte na dysku.
#[cfg(unix)]
pub fn alloc_size(meta: &Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.blocks() * 512
}

/// Windows nie podaje bloków w `Metadata` – zaokrąglamy do typowego klastra NTFS (4 KiB).
/// Plików skompresowanych i rzadkich to nie uwzględnia.
#[cfg(not(unix))]
pub fn alloc_size(meta: &Metadata) -> u64 {
    const CLUSTER: u64 = 4096;
    meta.len().div_ceil(CLUSTER) * CLUSTER
}

/// Identyfikator urządzenia (wolumenu) – do wykrywania punktów montowania.
#[cfg(unix)]
pub fn device(meta: &Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(meta.dev())
}

/// Na Windows punkty montowania to reparse pointy, a te i tak nie są śledzone (jak symlinki).
#[cfg(not(unix))]
pub fn device(_meta: &Metadata) -> Option<u64> {
    None
}

/// Klucz (urządzenie, i-węzeł) dla plików z więcej niż jednym twardym dowiązaniem.
#[cfg(unix)]
pub fn hardlink_key(meta: &Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    (!meta.is_dir() && meta.nlink() > 1).then(|| (meta.dev(), meta.ino()))
}

#[cfg(not(unix))]
pub fn hardlink_key(_meta: &Metadata) -> Option<(u64, u64)> {
    None
}

/// Czas modyfikacji jako Unix timestamp (s).
pub fn mtime(meta: &Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Placeholder iCloud (SF_DATALESS) – tylko macOS.
#[cfg(target_os = "macos")]
pub fn is_dataless(meta: &Metadata) -> bool {
    use std::os::macos::fs::MetadataExt;
    const SF_DATALESS: u32 = 0x4000_0000;
    meta.st_flags() & SF_DATALESS != 0
}

#[cfg(not(target_os = "macos"))]
pub fn is_dataless(_meta: &Metadata) -> bool {
    false
}

/// Na czas skanu zgłasza systemowi pracę zainicjowaną przez użytkownika, żeby macOS nie usypiał
/// aplikacji (App Nap), gdy okno jest schowane. Zwykły sen systemu nadal jest dozwolony.
/// Zwolnienie przy `Drop`.
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
        // SAFETY: `token` to obiekt zwrócony przez beginActivityWithOptions:reason:.
        unsafe {
            objc2_foundation::NSProcessInfo::processInfo().endActivity(&self.token)
        };
    }
}
