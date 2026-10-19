#!/usr/bin/env bash
# Runs a command with the updater signing key in the environment (TAURI_SIGNING_PRIVATE_KEY[_PASSWORD]),
# so `tauri build` signs the updater artifacts. Example: scripts/with-updater-key.sh pnpm build:mac
# The key path comes from DISK_LENS_UPDATER_KEY (environment or ~/.config/local-release/tauri-updater.env);
# the password, if any, from the file <key>.password next to it.
set -euo pipefail
if [[ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
  env_file="$HOME/.config/local-release/tauri-updater.env"
  # shellcheck disable=SC1090
  [[ -z "${DISK_LENS_UPDATER_KEY:-}" && -f "$env_file" ]] && source "$env_file"
  key="${DISK_LENS_UPDATER_KEY:-}"
  [[ -f "$key" ]] || { echo "DISK_LENS_UPDATER_KEY does not point to a key file (see README → Releasing)" >&2; exit 1; }
  TAURI_SIGNING_PRIVATE_KEY="$(cat "$key")"
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(cat "$key.password" 2>/dev/null || true)"
  export TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD
fi
exec "$@"
