//! Akcje na plikach: blokady ścieżek systemowych, Kosz, terminal – osobno dla każdego systemu.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Reguły ochrony ścieżek przed przeniesieniem do Kosza.
pub struct Rules {
    /// Chronione razem z zawartością.
    pub prefixes: &'static [&'static str],
    /// Wyjątki od `prefixes` – tu użytkownik może sprzątać.
    pub allowed: &'static [&'static str],
    /// Chronione tylko jako całość (zawartość wolno usuwać).
    pub exact: &'static [&'static str],
    pub case_insensitive: bool,
    pub system_name: &'static str,
}

pub const MACOS: Rules = Rules {
    prefixes: &["/System", "/usr", "/bin", "/sbin", "/private", "/Library/Apple", "/cores", "/dev"],
    allowed: &["/usr/local", "/private/tmp", "/private/var/folders"],
    exact: &["/Users", "/Applications", "/Library", "/Volumes", "/opt", "/opt/homebrew"],
    case_insensitive: true,
    system_name: "macOS",
};

pub const LINUX: Rules = Rules {
    prefixes: &[
        "/usr", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/etc", "/boot", "/proc", "/sys", "/dev", "/run",
        "/var/lib", "/snap", "/srv",
    ],
    allowed: &["/usr/local"],
    exact: &["/home", "/var", "/opt", "/media", "/mnt", "/root", "/tmp"],
    case_insensitive: false,
    system_name: "Linux",
};

pub const WINDOWS: Rules = Rules {
    prefixes: &[
        "c:/windows",
        "c:/program files",
        "c:/program files (x86)",
        "c:/programdata",
        "c:/$recycle.bin",
        "c:/system volume information",
        "c:/recovery",
        "c:/boot",
    ],
    allowed: &[],
    exact: &["c:/users"],
    case_insensitive: true,
    system_name: "Windows",
};

#[cfg(target_os = "macos")]
pub const RULES: &Rules = &MACOS;
#[cfg(windows)]
pub const RULES: &Rules = &WINDOWS;
#[cfg(not(any(target_os = "macos", windows)))]
pub const RULES: &Rules = &LINUX;

/// Ścieżka jako napis z `/`, bez końcowego separatora, małymi literami gdy system ich nie rozróżnia.
fn norm(path: &Path, rules: &Rules) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let s = if s.len() > 1 { s.trim_end_matches('/').to_string() } else { s };
    if rules.case_insensitive {
        s.to_lowercase()
    } else {
        s
    }
}

fn under(path: &str, prefix: &str) -> bool {
    path == prefix || path.strip_prefix(prefix).is_some_and(|rest| rest.starts_with('/'))
}

/// Zwraca powód blokady albo `None`, gdy ścieżkę wolno przenieść do Kosza.
pub fn protection_reason(path: &Path, scan_root: &Path, home: Option<&Path>, rules: &Rules) -> Option<String> {
    let p = norm(path, rules);
    let pn = |x: &str| norm(Path::new(x), rules);
    if p == norm(scan_root, rules) {
        return Some("To korzeń bieżącego skanu.".into());
    }
    // Korzeń systemu plików lub dysku: „/”, „C:/”.
    if p == "/" || (p.len() == 2 && p.ends_with(':')) {
        return Some("To korzeń dysku.".into());
    }
    if let Some(h) = home {
        let h = norm(h, rules);
        if p == h {
            return Some("To cały katalog domowy.".into());
        }
        if h.rsplit_once('/').is_some_and(|(parent, _)| p == parent) {
            return Some("To folder ze wszystkimi kontami użytkowników.".into());
        }
    }
    if rules.exact.iter().any(|e| p == pn(e)) {
        return Some("To folder systemowy najwyższego poziomu.".into());
    }
    let allowed = rules.allowed.iter().any(|a| under(&p, &pn(a)) && p != pn(a));
    if !allowed && rules.prefixes.iter().any(|x| under(&p, &pn(x))) {
        return Some(format!("Ścieżka należy do systemu {}.", rules.system_name));
    }
    None
}

/// Przenosi do Kosza. Na macOS przez `NSFileManager.trashItemAtURL` – bez AppleScriptu,
/// więc bez pytania o zgodę na sterowanie Finderem (kosztem braku „Odłóż z powrotem”).
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    #[allow(unused_mut)]
    let mut ctx = trash::TrashContext::default();
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        ctx.set_delete_method(DeleteMethod::NsFileManager);
    }
    ctx.delete(path).map_err(|e| e.to_string())
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

pub fn open_terminal(dir: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg("-a").arg("Terminal").arg(dir).spawn().map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(windows)]
    {
        // Windows Terminal, a gdy go nie ma – klasyczny cmd w nowym oknie.
        if Command::new("wt.exe").arg("-d").arg(dir).spawn().is_ok() {
            return Ok(());
        }
        Command::new("cmd")
            .args(["/C", "start", "", "cmd.exe"])
            .current_dir(dir)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        for term in ["x-terminal-emulator", "gnome-terminal", "konsole", "xfce4-terminal", "kitty", "alacritty", "xterm"] {
            if Command::new(term).current_dir(dir).spawn().is_ok() {
                return Ok(());
            }
        }
        Err("Nie znaleziono emulatora terminala.".into())
    }
}

pub fn open_privacy_settings() -> Result<(), String> {
    if cfg!(target_os = "macos") {
        Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    } else {
        Err("Dostępne tylko na macOS.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protection_macos() {
        let root = Path::new("/Users/me/Dokumenty");
        let home = Some(Path::new("/Users/me"));
        let p = |s: &str| protection_reason(Path::new(s), root, home, &MACOS);
        assert!(p("/").is_some());
        assert!(p("/System/Library").is_some());
        assert!(p("/usr/bin/ls").is_some());
        assert!(p("/usr/local/lib/x").is_none());
        assert!(p("/Users/me").is_some());
        assert!(p("/Users").is_some());
        assert!(p("/Users/me/Dokumenty").is_some(), "korzeń skanu");
        assert!(p("/Users/me/Dokumenty/old").is_none());
        assert!(p("/Applications/Foo.app").is_none());
        assert!(p("/private/var/db/x").is_some());
        assert!(p("/private/tmp/x").is_none());
        assert!(p("/usrx/file").is_none(), "prefiks musi kończyć się na granicy komponentu");
    }

    #[test]
    fn protection_linux() {
        let root = Path::new("/home/me/dane");
        let home = Some(Path::new("/home/me"));
        let p = |s: &str| protection_reason(Path::new(s), root, home, &LINUX);
        assert!(p("/").is_some());
        assert!(p("/etc/fstab").is_some());
        assert!(p("/home").is_some());
        assert!(p("/home/me").is_some());
        assert!(p("/home/me/dane/stare").is_none());
        assert!(p("/var/lib/docker").is_some());
        assert!(p("/var/log/old.log").is_none());
        assert!(p("/usr/local/share/x").is_none());
    }

    #[test]
    fn protection_windows() {
        let root = Path::new(r"D:\Dane");
        let home = Some(Path::new(r"C:\Users\Me"));
        let p = |s: &str| protection_reason(Path::new(s), root, home, &WINDOWS);
        assert!(p(r"C:\").is_some());
        assert!(p(r"D:\").is_some());
        assert!(p(r"C:\Windows\System32").is_some());
        assert!(p(r"c:\PROGRAM FILES\App").is_some(), "bez rozróżniania wielkości liter");
        assert!(p(r"C:\Users").is_some());
        assert!(p(r"C:\Users\Me").is_some());
        assert!(p(r"C:\Users\Me\Downloads\x.zip").is_none());
        assert!(p(r"d:\dane").is_some(), "korzeń skanu");
        assert!(p(r"D:\Dane\stare").is_none());
    }
}
