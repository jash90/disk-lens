export const errors: Record<string, { de: string; en: string }> = {
  'Najpierw wybierz folder do skanowania.': {
    de: 'Wähle zuerst einen Ordner zum Scannen.',
    en: 'First select a folder to scan.',
  },
  'Poczekaj na koniec skanu albo go anuluj.': {
    de: 'Warte, bis der Scan abgeschlossen ist, oder brich ihn ab.',
    en: 'Wait for the scan to finish or cancel it.',
  },
  'Element nie istnieje (mógł zostać usunięty lub przeskanowany ponownie).': {
    de: 'Das Element existiert nicht (es wurde möglicherweise gelöscht oder erneut gescannt).',
    en: 'The item does not exist (it may have been deleted or rescanned).',
  },
  'Nie można przenieść do Kosza korzenia skanu.': {
    de: 'Der Scan-Stammordner kann nicht in den Papierkorb verschoben werden.',
    en: 'The scan root cannot be moved to the Trash.',
  },
  'Nie znaleziono emulatora terminala.': {
    de: 'Kein Terminal-Emulator gefunden.',
    en: 'No terminal emulator found.',
  },
  'Dostępne tylko na macOS.': {
    de: 'Nur unter macOS verfügbar.',
    en: 'Only available on macOS.',
  },
  'To nie jest plik skanu disk-lens (.disklens) albo pochodzi z innej wersji.': {
    de: 'Dies ist keine disk-lens-Scandatei (.disklens), oder sie stammt aus einer anderen Version.',
    en: 'This is not a disk-lens scan file (.disklens), or it comes from another version.',
  },
  'Wybrana ścieżka nie jest folderem': {
    de: 'Der ausgewählte Pfad ist kein Ordner',
    en: 'The selected path is not a folder',
  },
  'niepoprawny rekord getattrlistbulk': {
    de: 'Ungültiger getattrlistbulk-Datensatz',
    en: 'Invalid getattrlistbulk record',
  },
  'To korzeń bieżącego skanu.': {
    de: 'Dies ist der Stammordner des aktuellen Scans.',
    en: 'This is the root of the current scan.',
  },
  'To korzeń dysku.': {
    de: 'Dies ist das Stammverzeichnis des Laufwerks.',
    en: 'This is the root of the drive.',
  },
  'To cały katalog domowy.': {
    de: 'Dies ist der gesamte Benutzerordner.',
    en: 'This is the entire home folder.',
  },
  'To folder ze wszystkimi kontami użytkowników.': {
    de: 'Dieser Ordner enthält alle Benutzerkonten.',
    en: 'This folder contains all user accounts.',
  },
  'To folder systemowy najwyższego poziomu.': {
    de: 'Dies ist ein Systemordner auf oberster Ebene.',
    en: 'This is a top-level system folder.',
  },
  'Nie udało się sprawdzić aktualizacji – brak połączenia z internetem albo serwer aktualizacji nie odpowiada.': {
    de: 'Updates konnten nicht geprüft werden – keine Internetverbindung oder der Update-Server antwortet nicht.',
    en: 'Could not check for updates – no internet connection or the update server is not responding.',
  },
  'Nowa wersja nie ma jeszcze paczki dla tego systemu.': {
    de: 'Für dieses System ist noch kein Paket der neuen Version verfügbar.',
    en: 'The new version does not have a package for this system yet.',
  },
  'Podpis pobranej aktualizacji się nie zgadza – nic nie zostało zainstalowane.': {
    de: 'Die Signatur des heruntergeladenen Updates stimmt nicht überein – es wurde nichts installiert.',
    en: 'The downloaded update signature does not match – nothing was installed.',
  },
  'Poczekaj na koniec skanu albo go anuluj – instalacja uruchomi aplikację ponownie.': {
    de: 'Warte, bis der Scan abgeschlossen ist, oder brich ihn ab – die Installation startet die Anwendung neu.',
    en: 'Wait for the scan to finish or cancel it – installation will restart the application.',
  },
  'Brak znalezionej aktualizacji – sprawdź ponownie.': {
    de: 'Kein Update gefunden – prüfe erneut.',
    en: 'No update found – check again.',
  },
  '(bez rozszerzenia)': { de: '(ohne Erweiterung)', en: '(no extension)' },
  'Dysk systemowy (/)': { de: 'Systemlaufwerk (/)', en: 'System drive (/)' },
  'System plików (/)': { de: 'Dateisystem (/)', en: 'Filesystem (/)' },
  'Dysk systemowy': { de: 'Systemlaufwerk', en: 'System drive' },
};

type Locale = 'pl' | 'de' | 'en';
type Template = {
  pattern: RegExp;
  de: (value: string) => string;
  en: (value: string) => string;
};

const systemPath = /^Ścieżka należy do systemu (macOS|Linux|Windows)\.$/;
const protectionReasons = new Set([
  'To korzeń bieżącego skanu.',
  'To korzeń dysku.',
  'To cały katalog domowy.',
  'To folder ze wszystkimi kontami użytkowników.',
  'To folder systemowy najwyższego poziomu.',
]);

const templates: Template[] = [
  {
    pattern: systemPath,
    de: value => `Der Pfad gehört zum System ${value}.`,
    en: value => `The path belongs to the ${value} system.`,
  },
  {
    pattern: /^Nie udało się przenieść do Kosza: ([\s\S]*)$/,
    de: value => `Verschieben in den Papierkorb fehlgeschlagen: ${value}`,
    en: value => `Could not move to the Trash: ${value}`,
  },
  {
    pattern: /^Uszkodzony plik skanu: ([\s\S]*)$/,
    de: value => `Beschädigte Scandatei: ${value}`,
    en: value => `Corrupted scan file: ${value}`,
  },
  {
    pattern: /^Nieznany format eksportu: ([\s\S]*)$/,
    de: value => `Unbekanntes Exportformat: ${value}`,
    en: value => `Unknown export format: ${value}`,
  },
  {
    pattern: /^Nie udało się zainstalować aktualizacji: ([\s\S]*)$/,
    de: value => `Update konnte nicht installiert werden: ${value}`,
    en: value => `Could not install the update: ${value}`,
  },
  {
    pattern: /^Dysk ([A-Za-z]:)$/,
    de: value => `Laufwerk ${value}`,
    en: value => `Drive ${value}`,
  },
];

// Only this known scanner validation can be localized inside an open failure.
// Other OS details and the complete path stay opaque, including embedded colons.
const invalidFolder = /^Nie można otworzyć ([\s\S]*): Wybrana ścieżka nie jest folderem$/;
const openFailure = /^Nie można otworzyć ([\s\S]*)$/;
const blocked = /^Zablokowane: ([\s\S]*)$/;

export function translateError(message: string, locale: Locale): string {
  if (locale === 'pl') return message;
  if (Object.hasOwn(errors, message)) return errors[message][locale];

  const invalid = invalidFolder.exec(message);
  if (invalid) {
    const detail = errors['Wybrana ścieżka nie jest folderem'][locale];
    return locale === 'de'
      ? `Kann nicht geöffnet werden: ${invalid[1]}: ${detail}`
      : `Cannot open ${invalid[1]}: ${detail}`;
  }
  const open = openFailure.exec(message);
  if (open) return locale === 'de' ? `Kann nicht geöffnet werden: ${open[1]}` : `Cannot open ${open[1]}`;

  const reason = blocked.exec(message);
  if (reason) {
    // Known reasons are fixed application messages; unknown details are preserved.
    const known = protectionReasons.has(reason[1]) || systemPath.exec(reason[1])?.[0] === reason[1];
    const translated = known ? translateError(reason[1], locale) : reason[1];
    return locale === 'de' ? `Blockiert: ${translated}` : `Blocked: ${translated}`;
  }

  for (const template of templates) {
    const match = template.pattern.exec(message);
    if (match && match[0] === message) return template[locale](match[1]);
  }
  return message;
}
