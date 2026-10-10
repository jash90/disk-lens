//! Przypisanie pliku do kategorii (kolor w treemapie, podział wg typu).

use crate::model::{extension_of, Kind};

pub const OTHER: u8 = 0;
pub const VIDEO: u8 = 1;
pub const IMAGE: u8 = 2;
pub const AUDIO: u8 = 3;
pub const ARCHIVE: u8 = 4;
pub const DOCUMENT: u8 = 5;
pub const CODE: u8 = 6;
pub const APP: u8 = 7;
pub const FOLDER: u8 = 8;

pub const NAMES: [&str; 9] = [
    "Inne",
    "Wideo",
    "Obrazy",
    "Audio",
    "Archiwa i obrazy dysków",
    "Dokumenty",
    "Kod i dane",
    "Aplikacje i biblioteki",
    "Foldery",
];

pub fn of(name: &str, kind: Kind) -> u8 {
    match kind {
        Kind::Dir => return FOLDER,
        Kind::Package => {
            return match extension_of(name).as_str() {
                "app" | "framework" | "bundle" | "plugin" | "kext" | "appex" | "xpc" => APP,
                "photoslibrary" => IMAGE,
                _ => FOLDER,
            }
        }
        Kind::Symlink | Kind::Other => return OTHER,
        Kind::File => {}
    }
    by_extension(&extension_of(name))
}

pub fn by_extension(ext: &str) -> u8 {
    match ext {
        "mp4" | "mov" | "m4v" | "mkv" | "avi" | "webm" | "wmv" | "flv" | "mpg" | "mpeg" | "3gp"
        | "mts" | "m2ts" | "prores" | "braw" | "r3d" => VIDEO,
        "jpg" | "jpeg" | "png" | "gif" | "heic" | "heif" | "webp" | "tif" | "tiff" | "bmp"
        | "raw" | "cr2" | "cr3" | "nef" | "arw" | "dng" | "psd" | "svg" | "ico" | "icns"
        | "avif" | "exr" | "hdr" | "raf" | "orf" => IMAGE,
        "mp3" | "m4a" | "aac" | "wav" | "flac" | "aiff" | "aif" | "ogg" | "opus" | "wma"
        | "alac" | "caf" | "mid" | "midi" | "logicx" => AUDIO,
        "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "lz4" | "dmg"
        | "iso" | "img" | "pkg" | "sparseimage" | "sparsebundle" | "vmdk" | "qcow2" | "vdi"
        | "ipsw" | "xip" | "apk" | "aab" | "ipa" | "jar" | "whl" => ARCHIVE,
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "pages" | "numbers"
        | "key" | "odt" | "ods" | "odp" | "rtf" | "txt" | "md" | "epub" | "csv" | "tex" => DOCUMENT,
        "rs" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "py" | "rb" | "go" | "java"
        | "kt" | "swift" | "c" | "h" | "cpp" | "hpp" | "cc" | "m" | "mm" | "cs" | "php"
        | "html" | "css" | "scss" | "json" | "yaml" | "yml" | "toml" | "xml" | "sql" | "sh"
        | "zsh" | "lock" | "map" | "wasm" | "rlib" | "rmeta" | "o" | "a" | "pyc" | "class"
        | "db" | "sqlite" | "sqlite3" | "parquet" | "npy" | "pt" | "safetensors" | "gguf"
        | "bin" | "onnx" | "ckpt" | "h5" | "log" => CODE,
        "dylib" | "so" | "dll" | "exe" | "msi" | "app" | "framework" => APP,
        _ => OTHER,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories() {
        assert_eq!(of("film.MOV", Kind::File), VIDEO);
        assert_eq!(of("Safari.app", Kind::Package), APP);
        assert_eq!(of("src", Kind::Dir), FOLDER);
        assert_eq!(of("x.unknown", Kind::File), OTHER);
    }
}
