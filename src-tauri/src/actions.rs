//! Actions on files: protection of system paths, Trash, terminal; separately for each system.

use crate::model::{Tree, ROOT};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Rules protecting paths from being moved to the Trash.
pub struct Rules {
    /// Protected together with their content.
    pub prefixes: &'static [&'static str],
    /// Exceptions to `prefixes`: the user may clean up here.
    pub allowed: &'static [&'static str],
    /// Protected only as a whole (the content may be removed).
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

/// The path as a string with `/`, without a trailing separator, lower-cased when the system ignores case.
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

/// Returns why the path is blocked, or `None` when it may be moved to the Trash.
pub fn protection_reason(path: &Path, scan_root: &Path, home: Option<&Path>, rules: &Rules) -> Option<String> {
    let p = norm(path, rules);
    let pn = |x: &str| norm(Path::new(x), rules);
    if p == norm(scan_root, rules) {
        return Some("To korzeń bieżącego skanu.".into());
    }
    // File system or drive root: "/", "C:/".
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

/// The protection rules resolved to nodes of one tree, so that a walk over the tree can tell
/// protected items without building a path for each of them. Agrees with [`protection_reason`].
#[derive(Default, Debug)]
pub struct Protected {
    /// Whether the children of the scan root lie in a protected zone.
    root_zone: bool,
    /// Nodes protected together with their content.
    blocked: Vec<u32>,
    /// Nodes whose content is free again (the node itself stays protected).
    allowed: Vec<u32>,
    /// Nodes protected only as a whole.
    exact: Vec<u32>,
}

impl Protected {
    pub fn resolve(tree: &Tree, home: Option<&Path>, rules: &Rules) -> Self {
        let root = norm(&tree.root_path, rules);
        // All nodes whose path is `target`: several when names differ only by case on a
        // case-sensitive volume, none when the path is outside the scan.
        let nodes_of = |target: &str| -> Vec<u32> {
            let rest = if root == "/" { target.strip_prefix('/') } else { target.strip_prefix(root.as_str()).and_then(|r| r.strip_prefix('/')) };
            let Some(rest) = rest else { return Vec::new() };
            rest.split('/').filter(|c| !c.is_empty()).fold(vec![ROOT], |ids, comp| {
                let matches = |c: &u32| {
                    let name = tree.name(*c);
                    if rules.case_insensitive {
                        name.to_lowercase() == comp
                    } else {
                        name == comp
                    }
                };
                ids.iter().flat_map(|&id| tree.children(id).filter(matches)).collect()
            })
        };
        let pn = |x: &str| norm(Path::new(x), rules);
        let all = |list: &[&str]| -> Vec<u32> { list.iter().flat_map(|x| nodes_of(&pn(x))).collect() };
        let mut exact = all(rules.exact);
        if let Some(h) = home.map(|h| norm(h, rules)) {
            exact.extend(nodes_of(&h));
            if let Some((parent, _)) = h.rsplit_once('/') {
                exact.extend(nodes_of(parent));
            }
        }
        Protected {
            root_zone: rules.prefixes.iter().any(|x| under(&root, &pn(x))) && !rules.allowed.iter().any(|a| under(&root, &pn(a))),
            blocked: all(rules.prefixes),
            allowed: all(rules.allowed),
            exact,
        }
    }

    /// Whether the children of `id` lie in a protected zone, given the zone `id` itself lies in.
    pub fn zone_below(&self, id: u32, zone: bool) -> bool {
        if self.blocked.contains(&id) {
            true
        } else if self.allowed.contains(&id) {
            false
        } else {
            zone
        }
    }

    /// The zone the children of `id` lie in, found by walking down from the scan root.
    pub fn zone_of_children(&self, tree: &Tree, id: u32) -> bool {
        tree.ancestors(id).into_iter().skip(1).fold(self.root_zone, |zone, a| self.zone_below(a, zone))
    }

    /// Whether `id` itself must not be trashed, given the zone its parent's children lie in.
    pub fn is_protected(&self, id: u32, zone: bool) -> bool {
        id == ROOT || zone || self.blocked.contains(&id) || self.exact.contains(&id)
    }
}

/// Moves to the Trash. On macOS through `NSFileManager.trashItemAtURL`: no AppleScript, so no
/// prompt for permission to control Finder (at the cost of losing "Put Back").
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
        // Windows Terminal, and when it is missing, classic cmd in a new window.
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
        assert!(p("/Users/me/Dokumenty").is_some(), "scan root");
        assert!(p("/Users/me/Dokumenty/old").is_none());
        assert!(p("/Applications/Foo.app").is_none());
        assert!(p("/private/var/db/x").is_some());
        assert!(p("/private/tmp/x").is_none());
        assert!(p("/usrx/file").is_none(), "a prefix has to end at a component boundary");
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
        assert!(p(r"c:\PROGRAM FILES\App").is_some(), "case-insensitive");
        assert!(p(r"C:\Users").is_some());
        assert!(p(r"C:\Users\Me").is_some());
        assert!(p(r"C:\Users\Me\Downloads\x.zip").is_none());
        assert!(p(r"d:\dane").is_some(), "scan root");
        assert!(p(r"D:\Dane\stare").is_none());
    }

    /// Every node of a realistic tree gets the same verdict from the resolved rules as from the
    /// path-based check.
    #[test]
    fn resolved_rules_agree_with_path_check() {
        use crate::model::testutil::entry;
        use crate::model::Kind;
        for (root, rules, home) in [("/", &MACOS, "/Users/me"), ("/usr", &MACOS, "/Users/me"), ("/Users/me", &MACOS, "/Users/me"), ("/", &LINUX, "/home/me")] {
            let mut t = Tree::new(root.into(), entry("", Kind::Dir, 0), 0);
            let paths = [
                "System/Library/x", "usr/local/bin/f", "usr/bin/f", "local/bin/f", "bin/f", "Users/me/Documents/f", "Users/other/f",
                "private/tmp/f", "private/var/folders/x", "private/var/db/x", "Applications/Foo.app/c", "Library/Apple/x",
                "Library/Caches/x", "opt/homebrew/bin/f", "home/me/data/f", "var/lib/docker/x", "var/log/old.log", "USR/share/f",
            ];
            for p in paths {
                let mut id = ROOT;
                for comp in p.split('/') {
                    id = match t.children(id).find(|&c| t.name(c) == comp) {
                        Some(c) => c,
                        None => t.add_children(id, vec![entry(comp, Kind::Dir, 0)])[0],
                    };
                }
            }
            let prot = Protected::resolve(&t, Some(Path::new(home)), rules);
            let mut stack = vec![(ROOT, false)];
            while let Some((id, zone)) = stack.pop() {
                let expected = protection_reason(&t.path(id), &t.root_path, Some(Path::new(home)), rules).is_some();
                let zone = if id == ROOT { zone } else { prot.zone_of_children(&t, t.node(id).parent) };
                assert_eq!(prot.is_protected(id, zone), expected, "{:?} under root {root}", t.path(id));
                stack.extend(t.children(id).map(|c| (c, false)));
            }
        }
    }
}
