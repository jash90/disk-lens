# Disk Lens

![Treemap with the selection synced to the folder tree](docs/screenshots/treemapa.jpg)

A disk usage analyzer for macOS, Windows and Linux in the spirit of GrandPerspective / WinDirStat. It shows the same data in two ways:

- **treemap**: every file is a rectangle whose area is proportional to its size (squarified algorithm, Canvas);
- **folder tree**: totals, percentage of the parent, file and folder counts, sortable by every column.

On top of that:
- tabs: largest files, breakdown by type, skipped paths, comparison of two scans, disk (capacity, free space, scan vs. disk usage);
- filters: name, minimum size, extensions, exclusions; applied without rescanning;
- actions: reveal in the file manager (Finder / Explorer), copy path, open a terminal, rescan a folder, move to Trash;
- export to CSV/JSON, and saving/loading a scan as a `.disklens` file;
- in-app updates from GitHub Releases (signed; installing always needs a click).

The app's interface is in Polish. Below, UI labels are quoted as they appear in the app, with an English translation.

## Installation

Download the package for your system from [Releases](../../releases):

| System | File |
|---|---|
| macOS 11+ (Apple Silicon and Intel) | `Disk.Lens_<version>_universal.dmg` |
| Windows 10/11 x64 | `Disk.Lens_<version>_x64-setup.exe` |
| Windows 11 ARM64 | `Disk.Lens_<version>_arm64-setup.exe` |
| Linux x86_64 | `.AppImage`, `.deb` (Debian/Ubuntu) or `.rpm` (Fedora/openSUSE), `amd64`/`x86_64` |
| Linux ARM64 | as above, `arm64`/`aarch64` |

Since 0.3.0 the app checks for a new version a few seconds after start and shows an **Aktualizacja X.Y.Z** (Update) button in the toolbar; **Plik → Sprawdź aktualizacje…** (File → Check for updates) checks on demand. The download is verified against a minisign key built into the app, then the app restarts as the new version. Version 0.2.0 has no updater, so 0.3.0 has to be installed once by hand.

- **macOS:** since 0.2.0 the app is signed with a Developer ID certificate and notarized by Apple, so it opens without warnings. To scan protected folders (Mail, Messages, app containers), grant Disk Lens **Full Disk Access** in System Settings → Privacy & Security.

The Windows and Linux packages are not code-signed:

- **Windows:** SmartScreen shows a warning; choose **More info → Run anyway**. The app uses WebView2 (the installer adds it if it is missing).
- **Linux:** an AppImage only needs to be made executable (`chmod +x`). The `.deb`/`.rpm` packages require WebKitGTK 4.1.

Stack: Tauri 2 + Rust (scanner, queries, treemap layout), React 19 + TypeScript + Vite (UI), pnpm.

## Running

```bash
pnpm install
pnpm tauri dev              # development mode
pnpm build:debug            # debug build with the MCP bridge for UI automation (no CSP)
```

All packages can be built locally on a Mac:

```bash
pnpm build:mac              # universal .app + .dmg (needs the aarch64- and x86_64-apple-darwin targets)
pnpm build:windows          # x64 NSIS installer, cross-compiled with cargo-xwin (brew install nsis llvm)
pnpm build:linux            # deb, rpm, AppImage for amd64 and arm64 in Docker → release-artifacts/
```

For Windows ARM64: `pnpm tauri build --runner cargo-xwin --target aarch64-pc-windows-msvc --bundles nsis`. When cross-compiling, `llvm-rc` and `lld-link` must be on `PATH` (`export PATH="$(brew --prefix llvm)/bin:$PATH"`).

`build:mac`, `build:windows` and `build:linux` also sign the updater artifacts (`.app.tar.gz`, `-setup.exe`, `.AppImage`, `.deb`, `.rpm` + `.sig`), so they need the updater key – see [Releasing](#releasing). `build:debug` skips them.

To sign and notarize the macOS build, set `APPLE_SIGNING_IDENTITY` (Developer ID Application certificate), `APPLE_ID`, `APPLE_PASSWORD` (app-specific password) and `APPLE_TEAM_ID` before `pnpm build:mac`. Tauri signs, notarizes and staples the `.app` and signs the `.dmg`; notarize the `.dmg` itself with `xcrun notarytool submit … --wait` and `xcrun stapler staple`.

A folder to scan can be passed as a launch argument: `disk-lens <folder>`.

Checking the scanner from the command line (comparable with `du -sk`):

```bash
cd src-tauri && cargo run --release --example scan -- <folder>
```

| Coloring by file age | Breakdown by type |
|---|---|
| ![Treemap colored by file age](docs/screenshots/wiek.jpg) | ![Breakdown by category and extension](docs/screenshots/wg-typu.jpg) |

## Performance

Measured on 2026-10-10 on a MacBook (10 cores, APFS on SSD) with a warm metadata cache. The machine was busy with other processes (load average 17–100), so absolute times are inflated. Every tool was measured on the same three folders (small, medium, large), one after another.

| Folder | Files | **Disk Lens** | GrandPerspective 2.6.3 | gdu 5.38 | du (BSD) |
|---|---|---|---|---|---|
| small | 34k | **0.10 s** | 1.22 s | 0.09 s | 0.17 s |
| medium | 366k | **3.2 s** | 24.5 s | 3.9 s | 4.0 s |
| large | 6.9M | **1.1 min**, 393 MB | 7.1 min, 891 MB | 1.5 min, 132 MB | 3.2 min |

- **Disk Lens:** the `examples/scan` CLI (the same scanner as in the app), hyperfine, 5 runs (3 for the large folder).
- **gdu and du:** hyperfine `-n -p -s -x` / `-skx`, number of runs as above (du on the large folder: 1).
- **GrandPerspective:** fresh start with App Nap disabled, time from handing over the folder to the result window appearing (±0.1 s accuracy, includes drawing the window). Median of 3 runs, 1 run for the large folder.
- **Result agreement:** Disk Lens total = `du -sk` to the KiB (small and medium folder). gdu reports ~1.2% less (it counts folders and hard links differently).
- **WinDirStat:** skipped, because it only runs on Windows, and comparing times from another computer would say nothing.

The table refers to version 0.1.0. Since 0.2.0 the scanner reads folders on APFS and HFS+ with `getattrlistbulk` (names and metadata in batches instead of one `lstat` per file). Measured on 2026-10-19 on the same Mac, warm cache, `examples/scan` with and without `--no-bulk`:

| Tree | Entries | 0.1.0 path (`read_dir` + `lstat`) | 0.2.0 (`getattrlistbulk`) | Speed-up |
|---|---|---|---|---|
| `~/Projects` | 8.7M | 66.5 s / 67.1 s | 36.6 s / 36.2 s | 1.83× |
| whole home folder | 16.4M | ~121 s | 78–81 s | ~1.55× |

Totals are identical in both modes. On SMB, NTFS, exFAT and other file systems, and on Windows and Linux, the previous path is used.

View queries on a tree of 7.8M nodes (release): children < 1 ms, treemap 14 ms, filter 54 ms, largest files 0.5 s, breakdown by type 0.7 s (`cargo run --release --example scan -- <path> --queries`).

## Releasing

1. Bump the version in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`, commit, push `main`.
2. Build everything (the updater key is loaded by `scripts/with-updater-key.sh`): `pnpm build:mac` with the Apple variables above, `pnpm build:windows`, the ARM64 Windows command, `pnpm build:linux`. Notarize and staple the `.dmg`.
3. Copy the release files into one folder with dots instead of spaces in the names (`Disk.Lens_…`, as GitHub stores them) – the installers plus the updater artifacts and their `.sig` files – and run `node scripts/latest-json.mjs <folder> [notes.txt]`. It verifies every signature against the public key in `tauri.conf.json`, checks that each was signed for this version, and writes `latest.json` (platform keys `darwin-*`, `windows-x86_64|aarch64[-nsis]`, `linux-x86_64|aarch64[-appimage|-deb|-rpm]`).
4. Tag `vX.Y.Z`, push it, and create the release with all files including `latest.json` and the `.sig` files. The app reads `releases/latest/download/latest.json`, so a draft or pre-release is not offered as an update.

| What | Where |
|---|---|
| Updater signing key (minisign) | `~/.tauri/disk-lens.key` (+ `.password`); path in `DISK_LENS_UPDATER_KEY` in `~/.config/local-release/tauri-updater.env`. The public key is `plugins.updater.pubkey` in `src-tauri/tauri.conf.json`. **Losing the private key means existing installs can no longer update.** |
| Developer ID certificate | login keychain; two certificates share the name, so sign by SHA-1 (`APPLE_SIGNING_IDENTITY`) |

## Tests and quality

```bash
pnpm test        # cargo test: model, filters, treemap (proptest), save/diff, scanner on a tempdir, volumes
pnpm typecheck   # tsc
pnpm lint        # eslint + cargo clippy -D warnings
```

The scanner tests (Unix) build a fixture tree in a temporary directory and check:
- folder totals;
- hard links counted once;
- symlinks not being followed;
- a `chmod 000` folder (skipped, the scan continues);
- a sparse file (logical size > allocated);
- an `.app` package as a single node;
- cancellation and rescanning a subtree;
- the total matching `du -sk`;
- on macOS, `getattrlistbulk` and `read_dir` + `lstat` building identical trees (edge-case fixture, unreadable folders, and a proptest on random trees).

## Default decisions

| Topic | Decision |
|---|---|
| Size | **Allocated on disk** by default (macOS/Linux: `st_blocks × 512`, like `du`; Windows: size rounded up to a 4 KiB cluster). The **Logiczny** (Logical) toggle shows the file size instead. |
| Units | Decimal (1 KB = 1000 B), as in Finder. |
| A folder's own blocks | Included in the folder total (matches `du`). |
| Hard links | macOS/Linux: counted once (deduplicated by `dev + inode` for files with `nlink > 1`); further copies have size 0 and a 🔗 icon. Windows: no deduplication. |
| Symlinks | Not followed; only the link itself is counted. |
| Other volumes | macOS/Linux: mount points are not entered by default (⛁ icon); the **Inne wolumeny** (Other volumes) option changes that. Windows: mount points and junctions are reparse points, which (like symlinks) are not followed. |
| Scanning "/" on macOS | Data on the Data volume is counted through firmlinks (`/Users`, `/Applications`…), and `/System/Volumes/Data` is skipped so nothing is counted twice. |
| Reading folders | macOS on APFS/HFS+: `getattrlistbulk` (one call returns a batch of entries with their metadata). Elsewhere: `read_dir` + `lstat`. Each folder is read to the end before its subfolders are queued, so no directory iterations are nested. |
| Packages | `.app`, `.photoslibrary`, `.framework` etc. are scanned in full but shown as a single tile/row. Double-click or **Wejdź do środka pakietu** (Show package contents) opens them. |
| iCloud | Placeholders (`SF_DATALESS`) are only marked ☁, never downloaded. |
| Missing permissions | The scan continues; the paths go to the **Pominięte** (Skipped) tab with a hint about Full Disk Access. |
| Disk tab | Capacity and free space of the volume the scan root is on (`statvfs` / `GetDiskFreeSpaceExW`; on macOS also the space available for important usage, which is what Finder shows). "Scanned vs. used" lists what the rest is made of: folders outside the scan, skipped paths, other APFS volumes in the container, snapshots, system files. |
| Trash | Only moving to the Trash, never permanent deletion, always with confirmation. On macOS via `NSFileManager.trashItemAtURL` (no AppleScript, so no prompt to control Finder); downside: Finder does not offer "Put Back". On Windows it goes to the Recycle Bin, on Linux to the freedesktop trash. Always blocked: the scan root, the disk root, the home folder and the users folder as a whole. Additionally per system (`src-tauri/src/actions.rs`): macOS – `/System`, `/usr` (except `/usr/local`), `/bin`, `/sbin`, `/private` (except tmp); Linux – `/usr`, `/etc`, `/boot`, `/var/lib`, `/proc`, `/sys`…; Windows – `C:\Windows`, `C:\Program Files`, `C:\ProgramData`, `$Recycle.Bin`…. |
| Trash and rescan during a scan | Blocked, because the scanner would keep writing results into detached nodes. |
| Treemap | Up to 50,000 tiles, BFS layout (the budget is spread evenly across levels). Items smaller than 1 px² are merged into grey "small items". A 1 px gap inside folders shows the structure. |
| Filters | Computed in a single O(n) pass over the arena and cached per (filter, scan, tree generation). Name, size and extension apply to files; exclusions skip files and whole folders with the given name. |
| Scan comparison | Matched by paths relative to the root; a list of the 1000 largest changes. |
| `.disklens` format | `DLENS` header + postcard + zstd. The file is not a stable exchange format across versions. |

## Limitations

- Windows: "allocated" is an approximation (4 KiB cluster) that ignores NTFS compression and sparse files.
- The total can differ from "Used" in Finder. APFS clones share blocks but are counted for every copy here. Sparse files take less space than their size. Local snapshots and system data do not live in folders. The **Dysk** (Disk) tab shows how large that gap is.
- File names that are not valid UTF-8 are stored in a lossy form. Actions on such files (Finder, Trash) may miss the path.
- Rescanning a subtree deduplicates hard links only within that subtree.

## Architecture

```
src-tauri/src/
  model.rs      node arena (Vec<Node>, u32 indexes), total propagation, detach/clear
  scanner.rs    parallel walker on rayon::scope, hard links, packages, cancellation, progress
  listing.rs    reading a folder with metadata: read_dir + lstat, or getattrlistbulk on macOS
  query.rs      children (sorting/paging), top files, breakdown by type, filters
  treemap.rs    squarified layout + binary tile encoding (24 B/tile)
  category.rs   extension → category
  actions.rs    path protection (separate rules for macOS/Linux/Windows), Trash, terminal
  updater.rs    update check/install (tauri-plugin-updater), startup check, readable errors
  volume.rs     volume list, capacity/free space of the scan's volume, scan vs. usage
  platform.rs   OS-specific metadata: allocated size, device, inode, iCloud
  persist.rs    saving/loading a scan, CSV/JSON export, diff
  commands.rs   Tauri commands (async, so they don't block the window)
src/
  api.ts        typed command and event calls
  store.ts      UI state (zustand)
  components/   Toolbar, Breadcrumb, FilterBar, FolderTree, Treemap, Panels, DiskPanel, Volumes, Update, Overlays
scripts/
  build-linux.sh        Linux packages in Docker
  with-updater-key.sh   runs a build with the updater signing key in the environment
  latest-json.mjs       latest.json for the updater + signature checks (lib/verify-minisign.mjs)
```

Each scan has its own tree (`Arc<RwLock<Tree>>`), so a cancelled scan that is still finishing does not corrupt a new one. Progress is emitted every 250 ms (`scan://progress`). The UI refreshes the tree on every event and the treemap less often (every 1.2 s), because the layout is more expensive.
