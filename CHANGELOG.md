# Changelog

What changed in each release of Vestigo. The newest version is on top.

## [0.2.0] — 2026-11-24

- The search window and the bar in Explorer and file dialogs open faster both when idle and on a busy computer: Vestigo locks its working set against Windows trimming (up to a safe share of RAM), opts the whole process out of background throttling and raises its CPU and GPU priority ahead of background work, skips redundant repaints and duplicate shell/shortcut lookups on open, and catches new file dialogs sooner.
- The search window opens from the hotkey or double Ctrl right after it was closed by a click outside it; before, it ignored them for half a second.
- Search lifts what you use in Windows, below what you open from Vestigo: files recently opened in any program, the folders you opened them from, and the programs you work in, wherever they were started. Folders you only pass through while browsing get nothing. Turn it off in *Settings → Index → System*.

## [0.1.2] — 2026-11-17

- A maximized Open or Save dialog, or one with no room above or below it, now gets the search bar right under its bottom edge and the results over the dialog, instead of a bar pushed into the dialog with no results.
- The search bar under dialogs and in Explorer is a little lower, as tall as its result rows.

## [0.1.1] — 2026-11-14

- The search window and the bar in Explorer open as fast after Vestigo has been idle for a while as they do right after the last use: Windows no longer pages out its memory while it waits.

## [0.1.0] — 2026-11-10

The first public release.

- Instant search for files, folders and apps from the NTFS file table, kept current by the change journal.
- Typo-tolerant matching, initials, and queries typed in the wrong keyboard layout.
- Filters in the query: kinds, extensions, dates, sizes, paths.
- Folder navigation from the keyboard, with folder marks.
- Search inside documents with `full:` (off by default).
- Preview card, actions with the Windows context menu, custom commands and web searches.
- The search bar inside Explorer windows, on the desktop and under Open and Save dialogs.
- The Vestigo window: kinds with counts, date filter, sortable columns, preview pane.
- Five built-in themes and a theme editor.
- English and Russian interface.
- Updates from GitHub: checked at startup and once a day, installed with one click, for the installed and the portable
  copy alike.
- Builds for x64, x86 and ARM64.
