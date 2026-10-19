//! Aktualizacje z GitHub Releases: `latest.json` z ostatniego wydania, archiwa podpisane kluczem
//! minisign (klucz publiczny w `tauri.conf.json`), więc podmieniony plik nie zostanie zainstalowany.
//!
//! Sprawdzanie na żądanie (menu „Plik”) i jedno ciche kilka sekund po starcie – ono tylko pokazuje
//! przycisk na pasku narzędzi. Instalacja zawsze wymaga kliknięcia.

use crate::commands::AppState;
use parking_lot::Mutex;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Error as UpdaterError, Update, UpdaterExt};

/// Opóźnienie cichego sprawdzenia po starcie – okno i ewentualny skan z argumentów mają pierwszeństwo.
const STARTUP_DELAY: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct Updates {
    /// Ostatnio znalezione wydanie – instalacja bierze dokładnie to, co pokazano użytkownikowi.
    available: Mutex<Option<Update>>,
    installing: AtomicBool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    version: String,
    current_version: String,
    notes: Option<String>,
    date: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProgress {
    done: u64,
    /// 0, gdy serwer nie podał długości.
    total: u64,
    finished: bool,
    error: Option<String>,
}

fn info(u: &Update) -> UpdateInfo {
    UpdateInfo {
        version: u.version.clone(),
        current_version: u.current_version.clone(),
        notes: u.body.clone().filter(|b| !b.trim().is_empty()),
        date: u.date.map(|d| d.date().to_string()),
    }
}

/// Komunikaty wtyczki są pisane dla programistów („platform darwin-x86_64 was not found…”),
/// więc do okna trafia jedno z kilku zdań, a szczegół idzie do konsoli.
fn message(error: &UpdaterError) -> String {
    eprintln!("aktualizacja: {error}");
    match error {
        UpdaterError::Reqwest(_)
        | UpdaterError::Network(_)
        | UpdaterError::ReleaseNotFound
        | UpdaterError::Http(_)
        | UpdaterError::Serialization(_) => {
            "Nie udało się sprawdzić aktualizacji – brak połączenia z internetem albo serwer aktualizacji nie odpowiada.".into()
        }
        UpdaterError::TargetNotFound(_)
        | UpdaterError::TargetsNotFound(_)
        | UpdaterError::UnsupportedArch
        | UpdaterError::UnsupportedOs => "Nowa wersja nie ma jeszcze paczki dla tego systemu.".into(),
        UpdaterError::Minisign(_)
        | UpdaterError::Base64(_)
        | UpdaterError::SignatureUtf8(_)
        | UpdaterError::SignedVersionMismatch { .. }
        | UpdaterError::MissingSignedVersion => {
            "Podpis pobranej aktualizacji się nie zgadza – nic nie zostało zainstalowane.".into()
        }
        _ => format!("Nie udało się zainstalować aktualizacji: {error}"),
    }
}

/// Pyta serwer, zapamiętuje wynik i rozsyła go (`update://status`), żeby UI zobaczyło też wynik
/// cichego sprawdzenia przy starcie.
async fn check(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    let updater = app.updater().map_err(|e| message(&e))?;
    let found = updater.check().await.map_err(|e| message(&e))?;
    let info = found.as_ref().map(info);
    *app.state::<Updates>().available.lock() = found;
    let _ = app.emit("update://status", &info);
    Ok(info)
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    check(&app).await
}

/// Ostatnio znalezione wydanie, bez pytania serwera.
#[tauri::command]
pub fn known_update(updates: tauri::State<'_, Updates>) -> Option<UpdateInfo> {
    updates.available.lock().as_ref().map(info)
}

/// Instalacja zamyka albo restartuje aplikację – nie w trakcie skanu.
fn ensure_idle(app: &AppHandle) -> Result<(), String> {
    if app.state::<AppState>().scanning() {
        return Err("Poczekaj na koniec skanu albo go anuluj – instalacja uruchomi aplikację ponownie.".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    ensure_idle(&app)?;
    let updates = app.state::<Updates>();
    let Some(update) = updates.available.lock().clone() else {
        return Err("Brak znalezionej aktualizacji – sprawdź ponownie.".into());
    };
    if updates.installing.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let mut done = 0u64;
    let mut last_emit = Instant::now();
    let downloaded = update
        .download(
            |chunk, total| {
                done += chunk as u64;
                if last_emit.elapsed() > Duration::from_millis(150) {
                    last_emit = Instant::now();
                    let p = UpdateProgress { done, total: total.unwrap_or(0), finished: false, error: None };
                    let _ = app.emit("update://progress", p);
                }
            },
            || {},
        )
        .await
        .map_err(|e| message(&e));
    // W trakcie pobierania mógł ruszyć skan – sprawdzamy jeszcze raz tuż przed instalacją.
    let result = downloaded.and_then(|bytes| {
        ensure_idle(&app)?;
        update.install(bytes).map_err(|e| message(&e))
    });
    updates.installing.store(false, Ordering::SeqCst);
    let error = result.err();
    let _ = app.emit("update://progress", UpdateProgress { done, total: done, finished: true, error: error.clone() });
    if let Some(e) = error {
        return Err(e);
    }
    // Windows: instalator NSIS sam zamyka aplikację; na macOS i Linuxie uruchamiamy nową wersję.
    app.restart();
}

/// Ciche sprawdzenie po starcie; błędy (np. brak sieci) tylko w konsoli.
pub fn check_on_startup(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tauri::async_runtime::spawn_blocking(|| std::thread::sleep(STARTUP_DELAY)).await.ok();
        if let Ok(Some(u)) = check(&app).await {
            eprintln!("aktualizacja dostępna: {}", u.version);
        }
    });
}
