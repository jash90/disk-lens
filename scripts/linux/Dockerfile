# Środowisko do budowania Disk Lens na Linuxa (deb, rpm, AppImage).
# Obraz z gotowym Rustem – pobieranie toolchainu przez rustup w emulacji amd64 potrafi się zawiesić.
FROM rust:1.96-bookworm

RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential curl wget file rsync rpm patchelf xdg-utils ca-certificates gnupg \
      libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev \
    && curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/* \
    && corepack enable && corepack prepare pnpm@10.33.0 --activate

# linuxdeploy (AppImage) nie ma FUSE w kontenerze.
ENV APPIMAGE_EXTRACT_AND_RUN=1
WORKDIR /work
