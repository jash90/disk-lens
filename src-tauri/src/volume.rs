//! Wolumeny: lista dysków do szybkiego wyboru oraz pojemność i wolne miejsce dysku, na którym leży skan.

use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Volume {
    pub name: String,
    pub path: String,
    pub total: u64,
    pub free: u64,
}

#[cfg(unix)]
pub fn space(path: &Path) -> Option<(u64, u64)> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` to poprawny napis zakończony zerem, `s` to zainicjalizowany bufor.
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let frsize = s.f_frsize as u64;
    Some((s.f_blocks as u64 * frsize, s.f_bavail as u64 * frsize))
}

#[cfg(windows)]
pub fn space(path: &Path) -> Option<(u64, u64)> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
    // SAFETY: `wide` jest zakończone zerem, wskaźniki wskazują na lokalne zmienne.
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut avail, &mut total, &mut free) };
    (ok != 0).then_some((total, avail))
}

pub fn list_volumes() -> Vec<Volume> {
    let mut out = Vec::new();
    let mut push = |name: String, path: PathBuf| {
        if let Some((total, free)) = space(&path) {
            if total > 0 {
                out.push(Volume { name, path: path.to_string_lossy().into_owned(), total, free });
            }
        }
    };

    #[cfg(target_os = "macos")]
    {
        push("Dysk systemowy (/)".into(), PathBuf::from("/"));
        if let Ok(rd) = std::fs::read_dir("/Volumes") {
            for e in rd.flatten() {
                // „/Volumes/Macintosh HD” to symlink do „/”.
                if e.file_type().map(|t| t.is_symlink()).unwrap_or(true) {
                    continue;
                }
                push(e.file_name().to_string_lossy().into_owned(), e.path());
            }
        }
    }

    #[cfg(windows)]
    {
        // SAFETY: funkcja bez argumentów, zwraca maskę bitową liter dysków.
        let mask = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
        for i in 0..26u32 {
            if mask & (1 << i) != 0 {
                let letter = (b'A' + i as u8) as char;
                push(format!("Dysk {letter}:"), PathBuf::from(format!("{letter}:\\")));
            }
        }
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    {
        push("System plików (/)".into(), PathBuf::from("/"));
        let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
        for line in mounts.lines() {
            let Some(mp) = line.split_whitespace().nth(1) else { continue };
            // Ścieżki w /proc/mounts mają spacje zakodowane jako \040.
            let mp = mp.replace("\\040", " ");
            let user_mount = ["/media/", "/mnt/", "/run/media/"].iter().any(|p| mp.starts_with(p)) || mp == "/home";
            if user_mount {
                let name = mp.rsplit('/').next().unwrap_or(&mp).to_string();
                push(name, PathBuf::from(&mp));
            }
        }
    }

    out
}

/// Wolumen, na którym leży ścieżka – do widoku „Dysk”.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VolumeInfo {
    pub name: String,
    pub mount_path: String,
    pub fs_type: String,
    pub total: u64,
    /// Wolne dla zwykłego zapisu (`statvfs` / `GetDiskFreeSpaceExW`).
    pub free: u64,
    /// macOS: wolne razem z miejscem, które system odzyska na żądanie (pamięć podręczna, kopie
    /// lokalne Time Machine) – to liczba, którą pokazuje Finder.
    pub available_important: Option<u64>,
}

/// Zestawienie: ile zajmuje wolumen, a ile znalazł skan.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    pub used: u64,
    pub scanned: u64,
    /// Zajęte, ale niewidoczne w skanie (inne foldery, pliki systemowe, migawki, pominięte ścieżki).
    pub unaccounted: u64,
    /// Skan większy niż zajętość wolumenu (klony APFS, skan z innymi wolumenami).
    pub overcounted: u64,
    /// Część zajętego miejsca, którą system zwolni sam (macOS); 0, gdy nieznana.
    pub purgeable: u64,
}

pub fn coverage(vol: &VolumeInfo, scanned: u64) -> Coverage {
    let used = vol.total.saturating_sub(vol.free);
    Coverage {
        used,
        scanned,
        unaccounted: used.saturating_sub(scanned),
        overcounted: scanned.saturating_sub(used),
        purgeable: vol.available_important.map_or(0, |a| a.saturating_sub(vol.free)),
    }
}

/// Nazwa do wyświetlenia, gdy system jej nie podaje.
fn fallback_name(mount: &str) -> String {
    let trimmed = mount.trim_end_matches(['/', '\\']);
    if trimmed.len() == 2 && trimmed.ends_with(':') {
        return format!("Dysk {trimmed}");
    }
    match trimmed.rsplit(['/', '\\']).next() {
        Some(last) if !last.is_empty() => last.to_string(),
        _ => "Dysk systemowy".into(),
    }
}

pub fn volume_for(path: &Path) -> Option<VolumeInfo> {
    let (mount_path, fs_type, sys_name) = mount_of(path)?;
    let (total, free) = space(path)?;
    if total == 0 {
        return None;
    }
    let (name, available_important) = macos::details(path);
    Some(VolumeInfo {
        name: name.or(sys_name).unwrap_or_else(|| fallback_name(&mount_path)),
        mount_path,
        fs_type,
        total,
        free,
        available_important,
    })
}

/// (punkt montowania, system plików, nazwa wolumenu, jeśli system ją zna).
#[cfg(target_os = "macos")]
fn mount_of(path: &Path) -> Option<(String, String, Option<String>)> {
    use std::ffi::{CStr, CString};
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` jest zakończone zerem, `s` to zainicjalizowany bufor.
    if unsafe { libc::statfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    // SAFETY: jądro zwraca napisy zakończone zerem w tablicach o stałym rozmiarze.
    let text = |a: &[libc::c_char]| unsafe { CStr::from_ptr(a.as_ptr()) }.to_string_lossy().into_owned();
    Some((text(&s.f_mntonname), text(&s.f_fstypename), None))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn mount_of(path: &Path) -> Option<(String, String, Option<String>)> {
    let canon = std::fs::canonicalize(path).ok()?;
    let info = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
    let (mount, fs) = mount_in(&info, &canon)?;
    Some((mount, fs, None))
}

/// Najdłuższy punkt montowania z `/proc/self/mountinfo`, pod którym leży `path` (porównanie
/// po składnikach ścieżki, więc `/home` nie pasuje do `/homework`).
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn mount_in(mountinfo: &str, path: &Path) -> Option<(String, String)> {
    let mut best: Option<(String, String)> = None;
    for line in mountinfo.lines() {
        // id rodzic major:minor korzeń PUNKT opcje [pola opcjonalne…] - TYP źródło superopcje
        let mut it = line.split(" - ");
        let (Some(left), Some(right)) = (it.next(), it.next()) else { continue };
        let Some(mp) = left.split(' ').nth(4) else { continue };
        let Some(fs) = right.split(' ').next() else { continue };
        let mp = mp.replace("\\040", " ").replace("\\011", "\t").replace("\\134", "\\");
        if path.starts_with(&mp) && best.as_ref().is_none_or(|(b, _)| mp.len() >= b.len()) {
            best = Some((mp, fs.to_string()));
        }
    }
    best
}

#[cfg(windows)]
fn mount_of(path: &Path) -> Option<(String, String, Option<String>)> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{GetVolumeInformationW, GetVolumePathNameW};
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut root = [0u16; 261];
    // SAFETY: `wide` jest zakończone zerem, bufor ma podaną długość.
    if unsafe { GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
        return None;
    }
    let (mut label, mut fs) = ([0u16; 261], [0u16; 261]);
    // SAFETY: `root` jest zakończone zerem, bufory mają podane długości, pozostałe wyjścia są opcjonalne.
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            label.as_mut_ptr(),
            label.len() as u32,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            fs.as_mut_ptr(),
            fs.len() as u32,
        )
    };
    let text = |b: &[u16]| String::from_utf16_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]);
    let mount = text(&root);
    if ok == 0 {
        return Some((mount, String::new(), None));
    }
    let label = text(&label);
    let name = (!label.is_empty()).then(|| format!("{label} ({})", mount.trim_end_matches('\\')));
    Some((mount, text(&fs), name))
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2_foundation::{
        NSArray, NSNumber, NSString, NSURLVolumeAvailableCapacityForImportantUsageKey, NSURLVolumeLocalizedNameKey, NSURL,
    };
    use std::path::Path;

    /// Nazwa wolumenu z Findera i miejsce dostępne „dla ważnych danych” (z odzyskiwalnym).
    pub fn details(path: &Path) -> (Option<String>, Option<u64>) {
        // Nowy NSURL przy każdym wywołaniu – wartości zasobów są buforowane w instancji.
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        // SAFETY: klucze to stałe Foundation, dostępne od macOS 10.13 (aplikacja wymaga 11+).
        let (name_key, important_key) = unsafe { (NSURLVolumeLocalizedNameKey, NSURLVolumeAvailableCapacityForImportantUsageKey) };
        let keys = NSArray::from_slice(&[name_key, important_key]);
        let Ok(values) = url.resourceValuesForKeys_error(&keys) else { return (None, None) };
        let name = values
            .objectForKey(name_key)
            .and_then(|v| v.downcast::<NSString>().ok())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());
        let important = values
            .objectForKey(important_key)
            .and_then(|v| v.downcast::<NSNumber>().ok())
            .map(|n| n.longLongValue())
            .filter(|&n| n > 0)
            .map(|n| n as u64);
        (name, important)
    }
}

#[cfg(not(target_os = "macos"))]
mod macos {
    pub fn details(_path: &std::path::Path) -> (Option<String>, Option<u64>) {
        (None, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vol(total: u64, free: u64, important: Option<u64>) -> VolumeInfo {
        VolumeInfo { name: "x".into(), mount_path: "/".into(), fs_type: "apfs".into(), total, free, available_important: important }
    }

    #[test]
    fn coverage_splits_used_space() {
        let c = coverage(&vol(1000, 300, Some(450)), 500);
        assert_eq!(c, Coverage { used: 700, scanned: 500, unaccounted: 200, overcounted: 0, purgeable: 150 });
    }

    #[test]
    fn coverage_never_goes_negative() {
        // Klony APFS / skan z innymi wolumenami: skan większy niż zajętość.
        let c = coverage(&vol(1000, 900, None), 250);
        assert_eq!((c.used, c.unaccounted, c.overcounted, c.purgeable), (100, 0, 150, 0));
        // Wolne większe niż „ważne” (nie powinno się zdarzyć, ale bez przepełnienia).
        assert_eq!(coverage(&vol(1000, 600, Some(500)), 0).purgeable, 0);
        assert_eq!(coverage(&vol(0, 0, None), 10).overcounted, 10);
    }

    #[test]
    fn mountinfo_longest_component_prefix() {
        let info = "\
22 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw
30 22 8:3 / /home rw,relatime shared:2 - xfs /dev/sda3 rw
31 22 0:40 / /mnt/My\\040Disk rw,nosuid - exfat /dev/sdb1 rw
32 30 0:41 / /home/me/net rw - nfs4 srv:/x rw";
        let m = |p: &str| mount_in(info, Path::new(p)).unwrap();
        assert_eq!(m("/etc/fstab"), ("/".into(), "ext4".into()));
        assert_eq!(m("/home/me/docs"), ("/home".into(), "xfs".into()));
        assert_eq!(m("/homework/x"), ("/".into(), "ext4".into()), "prefiks tylko po składnikach");
        assert_eq!(m("/home/me/net/a"), ("/home/me/net".into(), "nfs4".into()));
        assert_eq!(m("/mnt/My Disk/photos"), ("/mnt/My Disk".into(), "exfat".into()));
    }

    #[test]
    fn fallback_names() {
        assert_eq!(fallback_name("/"), "Dysk systemowy");
        assert_eq!(fallback_name("/mnt/backup/"), "backup");
        assert_eq!(fallback_name("D:\\"), "Dysk D:");
    }

    #[test]
    fn volume_for_temp_dir() {
        let d = tempfile::tempdir().unwrap();
        let v = volume_for(d.path()).expect("wolumen katalogu tymczasowego");
        assert!(v.total > 0 && v.free <= v.total);
        assert!(!v.mount_path.is_empty() && !v.fs_type.is_empty() && !v.name.is_empty());
        // Na macOS ścieżki z /private/var leżą na /System/Volumes/Data przez firmlinki.
        if cfg!(target_os = "linux") {
            assert!(d.path().canonicalize().unwrap().starts_with(&v.mount_path));
        }
        if cfg!(target_os = "macos") {
            assert!(v.available_important.is_some_and(|a| a >= v.free), "{v:?}");
        }
        assert!(volume_for(Path::new("/definitely/not/here")).is_none());
    }
}
