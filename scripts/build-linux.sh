#!/usr/bin/env bash
# Buduje paczki Linuxa w Dockerze: ./scripts/build-linux.sh [amd64|arm64]...
# Wynik: release-artifacts/linux-<arch>/ (deb, rpm, AppImage + podpisy .sig dla aktualizacji w aplikacji).
set -euo pipefail
cd "$(dirname "$0")/.."
# Klucz podpisujący aktualizacje – bez niego `tauri build` się nie uda (createUpdaterArtifacts).
[[ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]] || exec scripts/with-updater-key.sh "$0" "$@"
ARCHES=("${@:-amd64 arm64}")
[ $# -eq 0 ] && ARCHES=(amd64 arm64)

for arch in "${ARCHES[@]}"; do
  image="disk-lens-linux-builder:$arch"
  docker build --platform "linux/$arch" -t "$image" scripts/linux
  out="release-artifacts/linux-$arch"
  mkdir -p "$out"
  # Źródła kopiujemy do kontenera (bez node_modules/target z macOS – mają natywne binarki darwin).
  docker run --rm --platform "linux/$arch" \
    -e TAURI_SIGNING_PRIVATE_KEY -e TAURI_SIGNING_PRIVATE_KEY_PASSWORD \
    -v "$PWD":/src:ro \
    -v "$PWD/$out":/out \
    -v "disk-lens-cargo-$arch":/usr/local/cargo/registry \
    -v "disk-lens-target-$arch":/work/src-tauri/target \
    -v "disk-lens-tauri-cache-$arch":/root/.cache/tauri \
    "$image" bash -euo pipefail -c '
      rsync -a --exclude node_modules --exclude "src-tauri/target" --exclude release-artifacts --exclude .git /src/ /work/
      pnpm install --frozen-lockfile
      # Wtyczka linuxdeploy to AppImage z sygnaturą w wypełnieniu nagłówka ELF; emulacja amd64
      # w Dockerze (Rosetta/qemu) odrzuca takie pliki („Exec format error”). Zerujemy 3 bajty kopii.
      plugin=/root/.cache/tauri/linuxdeploy-plugin-appimage.AppImage
      if [ ! -s "$plugin" ]; then
        a=$(uname -m)
        curl -fsSL -o "$plugin" "https://github.com/linuxdeploy/linuxdeploy-plugin-appimage/releases/download/continuous/linuxdeploy-plugin-appimage-$a.AppImage"
        chmod +x "$plugin"
      fi
      dd if=/dev/zero of="$plugin" bs=1 seek=8 count=3 conv=notrunc 2>/dev/null
      pnpm tauri build --bundles deb,rpm,appimage
      find src-tauri/target/release/bundle \( -name "*.deb" -o -name "*.rpm" -o -name "*.AppImage" -o -name "*.sig" \) -exec cp {} /out/ \;
    '
  ls -la "$out"
done
