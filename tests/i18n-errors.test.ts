import assert from 'node:assert/strict';
import test from 'node:test';
import { translateError } from '../src/i18n/errors.ts';

test('open failures translate the wrapper and preserve paths and OS details', () => {
  const input = 'Nie można otworzyć C:\\Users\\Ścieżka należy do systemu Windows.\\$& {path}: a: Access denied (os error 5)';
  assert.equal(translateError(input, 'en'), 'Cannot open C:\\Users\\Ścieżka należy do systemu Windows.\\$& {path}: a: Access denied (os error 5)');
  assert.equal(translateError(input, 'de'), 'Kann nicht geöffnet werden: C:\\Users\\Ścieżka należy do systemu Windows.\\$& {path}: a: Access denied (os error 5)');
});

test('unknown errors and filenames remain unchanged in every locale', () => {
  for (const locale of ['pl', 'de', 'en'] as const) {
    for (const input of ['', 'Access denied (os error 5)', '/tmp/Dysk systemowy (from backup)', 'file: Nie udało się przenieść do Kosza: report.txt', 'To korzeń dysku. backup.txt', '__proto__', 'constructor']) {
      assert.equal(translateError(input, locale), input);
    }
  }
});

test('Polish errors pass through byte for byte', () => {
  assert.equal(translateError('Zablokowane: Ścieżka należy do systemu Windows.', 'pl'), 'Zablokowane: Ścieżka należy do systemu Windows.');
  assert.equal(translateError('Nie można otworzyć /tmp/źródło: Wybrana ścieżka nie jest folderem', 'pl'), 'Nie można otworzyć /tmp/źródło: Wybrana ścieżka nie jest folderem');
});

test('all fixed application errors and generated labels have German and English text', () => {
  const cases = [
    ['Najpierw wybierz folder do skanowania.', 'Wähle zuerst einen Ordner zum Scannen.', 'First select a folder to scan.'],
    ['Poczekaj na koniec skanu albo go anuluj.', 'Warte, bis der Scan abgeschlossen ist, oder brich ihn ab.', 'Wait for the scan to finish or cancel it.'],
    ['Element nie istnieje (mógł zostać usunięty lub przeskanowany ponownie).', 'Das Element existiert nicht (es wurde möglicherweise gelöscht oder erneut gescannt).', 'The item does not exist (it may have been deleted or rescanned).'],
    ['Nie można przenieść do Kosza korzenia skanu.', 'Der Scan-Stammordner kann nicht in den Papierkorb verschoben werden.', 'The scan root cannot be moved to the Trash.'],
    ['Nie znaleziono emulatora terminala.', 'Kein Terminal-Emulator gefunden.', 'No terminal emulator found.'],
    ['Dostępne tylko na macOS.', 'Nur unter macOS verfügbar.', 'Only available on macOS.'],
    ['To nie jest plik skanu disk-lens (.disklens) albo pochodzi z innej wersji.', 'Dies ist keine disk-lens-Scandatei (.disklens), oder sie stammt aus einer anderen Version.', 'This is not a disk-lens scan file (.disklens), or it comes from another version.'],
    ['Wybrana ścieżka nie jest folderem', 'Der ausgewählte Pfad ist kein Ordner', 'The selected path is not a folder'],
    ['niepoprawny rekord getattrlistbulk', 'Ungültiger getattrlistbulk-Datensatz', 'Invalid getattrlistbulk record'],
    ['(bez rozszerzenia)', '(ohne Erweiterung)', '(no extension)'],
    ['Dysk systemowy (/)', 'Systemlaufwerk (/)', 'System drive (/)'],
    ['System plików (/)', 'Dateisystem (/)', 'Filesystem (/)'],
    ['Dysk systemowy', 'Systemlaufwerk', 'System drive'],
  ] as const;
  for (const [input, de, en] of cases) {
    assert.equal(translateError(input, 'de'), de);
    assert.equal(translateError(input, 'en'), en);
  }
});

test('protection reasons translate both standalone and inside a blocked error', () => {
  const cases = [
    ['To korzeń bieżącego skanu.', 'Dies ist der Stammordner des aktuellen Scans.', 'This is the root of the current scan.'],
    ['To korzeń dysku.', 'Dies ist das Stammverzeichnis des Laufwerks.', 'This is the root of the drive.'],
    ['To cały katalog domowy.', 'Dies ist der gesamte Benutzerordner.', 'This is the entire home folder.'],
    ['To folder ze wszystkimi kontami użytkowników.', 'Dieser Ordner enthält alle Benutzerkonten.', 'This folder contains all user accounts.'],
    ['To folder systemowy najwyższego poziomu.', 'Dies ist ein Systemordner auf oberster Ebene.', 'This is a top-level system folder.'],
    ['Ścieżka należy do systemu Windows.', 'Der Pfad gehört zum System Windows.', 'The path belongs to the Windows system.'],
    ['Ścieżka należy do systemu macOS.', 'Der Pfad gehört zum System macOS.', 'The path belongs to the macOS system.'],
    ['Ścieżka należy do systemu Linux.', 'Der Pfad gehört zum System Linux.', 'The path belongs to the Linux system.'],
  ] as const;
  for (const [input, de, en] of cases) {
    assert.equal(translateError(input, 'de'), de);
    assert.equal(translateError(input, 'en'), en);
    assert.equal(translateError(`Zablokowane: ${input}`, 'de'), `Blockiert: ${de}`);
    assert.equal(translateError(`Zablokowane: ${input}`, 'en'), `Blocked: ${en}`);
  }
});

test('updater errors translate every user-facing failure class', () => {
  const cases = [
    ['Nie udało się sprawdzić aktualizacji – brak połączenia z internetem albo serwer aktualizacji nie odpowiada.', 'Updates konnten nicht geprüft werden – keine Internetverbindung oder der Update-Server antwortet nicht.', 'Could not check for updates – no internet connection or the update server is not responding.'],
    ['Nowa wersja nie ma jeszcze paczki dla tego systemu.', 'Für dieses System ist noch kein Paket der neuen Version verfügbar.', 'The new version does not have a package for this system yet.'],
    ['Podpis pobranej aktualizacji się nie zgadza – nic nie zostało zainstalowane.', 'Die Signatur des heruntergeladenen Updates stimmt nicht überein – es wurde nichts installiert.', 'The downloaded update signature does not match – nothing was installed.'],
    ['Poczekaj na koniec skanu albo go anuluj – instalacja uruchomi aplikację ponownie.', 'Warte, bis der Scan abgeschlossen ist, oder brich ihn ab – die Installation startet die Anwendung neu.', 'Wait for the scan to finish or cancel it – installation will restart the application.'],
    ['Brak znalezionej aktualizacji – sprawdź ponownie.', 'Kein Update gefunden – prüfe erneut.', 'No update found – check again.'],
  ] as const;
  for (const [input, de, en] of cases) {
    assert.equal(translateError(input, 'de'), de);
    assert.equal(translateError(input, 'en'), en);
  }
});

test('templated failures preserve opaque details including multiline text and replacement tokens', () => {
  const cases = [
    ['Nie udało się przenieść do Kosza: ', 'Verschieben in den Papierkorb fehlgeschlagen: ', 'Could not move to the Trash: '],
    ['Uszkodzony plik skanu: ', 'Beschädigte Scandatei: ', 'Corrupted scan file: '],
    ['Nieznany format eksportu: ', 'Unbekanntes Exportformat: ', 'Unknown export format: '],
    ['Nie udało się zainstalować aktualizacji: ', 'Update konnte nicht installiert werden: ', 'Could not install the update: '],
    ['Zablokowane: ', 'Blockiert: ', 'Blocked: '],
  ] as const;
  const details = 'C:\\Dysk systemowy\\$&{name}: To korzeń dysku.\nraw detail';
  for (const [prefix, de, en] of cases) {
    assert.equal(translateError(prefix + details, 'de'), de + details);
    assert.equal(translateError(prefix + details, 'en'), en + details);
  }
});

test('known scanner validation translates inside an open error while its path stays untouched', () => {
  assert.equal(translateError('Nie można otworzyć /tmp/To korzeń dysku.: Wybrana ścieżka nie jest folderem', 'de'), 'Kann nicht geöffnet werden: /tmp/To korzeń dysku.: Der ausgewählte Pfad ist kein Ordner');
  assert.equal(translateError('Nie można otworzyć C:\\a: Wybrana ścieżka nie jest folderem', 'en'), 'Cannot open C:\\a: The selected path is not a folder');
});

test('drive labels only translate complete generated drive names', () => {
  assert.equal(translateError('Dysk C:', 'de'), 'Laufwerk C:');
  assert.equal(translateError('Dysk D:', 'en'), 'Drive D:');
  assert.equal(translateError('Dysk D: backup', 'en'), 'Dysk D: backup');
  assert.equal(translateError('Dysk D:\n', 'en'), 'Dysk D:\n');
  assert.equal(translateError('Ścieżka należy do systemu Windows.\n', 'de'), 'Ścieżka należy do systemu Windows.\n');
});

test('a blocked wrapper only translates known protection reasons and preserves opaque details', () => {
  assert.equal(translateError('Zablokowane: Nie udało się przenieść do Kosza: C:\\x', 'en'), 'Blocked: Nie udało się przenieść do Kosza: C:\\x');
  assert.equal(translateError('Zablokowane: Dysk C:', 'de'), 'Blockiert: Dysk C:');
});
