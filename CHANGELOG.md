# Changelog

What changed in each version of ilias-tui. Versions follow
[semantic versioning](https://semver.org): before 1.0, the middle number goes up for new
features and changes in behaviour, the last one for fixes only. See
[Versions](CONTRIBUTING.md#versions) for how a release is made.

## Unreleased

### Fixed

- **Scrolling with the mouse support off** (`ILIAS_MOUSE=0` or the settings) no longer
  scrolls the terminal's history over the TUI: the terminal sends the wheel and trackpad
  as arrow keys instead, and text selection keeps working. Writing a forum post no longer
  turns the mouse support back on when it is off in the settings.
- **Fast scrolling** doesn't fall behind: everything waiting is read before the screen is
  drawn again.

## 0.5.0 - 2026-11-19

### Added

- **Accordions** in page text: each pane is a box with its caption, `▸` closed or `▾`
  open as on ILIAS, and `Enter` opens or closes it. A short text beside a pane in a
  table-like row, such as the date of an agenda entry, goes into the box's top border.
  Files and links in closed panes are still synced, searched and found by the `/` filter.
- **Links in page text to ILIAS files and folders** show and open as those: a file link
  has its file icon, `Enter` downloads and opens it (or your saved copy, marked `✓ saved`)
  and `d` downloads it; a folder or course link opens in ilias-tui. Sync still leaves them
  alone. A line that is only a label and its links (`Material: CBL26 | EBNF`) keeps just
  the label above the links, labels such as `Inhalt:` are bold, and web links show their
  address without `https://`.
- **Files deleted on ILIAS can go to your Trash**: after a two-way sync, a dialog lists the
  files ILIAS no longer has that you left untouched, and `y` moves them to your computer's
  trash (macOS Trash, Windows Recycle Bin, Linux desktop trash). Edited ones are kept.

### Changed

- **Sync rebuilt on one model**: the base (what sync recorded), ILIAS now and your disk
  now go into one pure diff with one decision per path, which gives a list of ops
  (download, upload, replace, rename, new folder, ILIAS trash, your Trash, conflict,
  keep); the review, the report, the status line and the brake are built from those ops.
  The saved files move to `<host>.base.json`, with paths relative to the download folder
  and the folders sync saved or created; `<host>.downloads.json` is taken over the first
  time, and entries go once a file is gone on both sides. Behaviour stays as it was, with
  these differences: a saved folder that is gone on ILIAS is no longer offered for upload
  as a new folder (its untouched files are offered for your Trash instead), a file
  downloaded with `d` counts as saved by sync, a rename where ILIAS offers no Versions is
  uploaded as new with the old file proposed for the trash, and files saved into an earlier
  download folder no longer count as saved: if you changed the download folder, the first
  sync downloads them again into the current one (the copies in the earlier folder stay),
  and opening one downloads it again too.
- **`S` on a course's page syncs the whole course**, even when a folder in it is selected.
  Inside a folder it still syncs the selected subfolder (or the folder on screen).
- **The status line names what was synced** ("Synced Test: …"), so it shows when `S`
  synced the selected folder rather than the course on screen.
- **A folder you delete goes to the ILIAS trash as a whole** (where ILIAS lets you delete
  it, and only if none of its files changed on ILIAS since): one proposal in the review
  instead of one per file, and always asked about once more. Before, its files went but
  the empty folder stayed.
- **The TUI shows ILIAS as it is after a sync, a two-way push or the trash on your disk**:
  the page on screen is read again (the result stays in the status line), the others once
  you go back to them, and the "saved" marks follow.
- **Dialogs and the sync review look better**: an icon and colour per kind of question
  and per section (Nerd Font icons, plain ones without), the question as a headline with
  its files listed below, `y`/`n` as buttons, count badges and checkboxes.
- **Two-way sync mirrors your folder**: a file renamed in the same folder is renamed on
  ILIAS (through "Replace File" with the new name) instead of uploaded again; a file
  moved to another folder is uploaded there and the old one goes to the trash; files you
  deleted are ticked for the ILIAS trash in the review (`Space` unticks). The status line
  says "to the ILIAS trash" instead of "(left out)".
- **The sync review is the question**: `Enter` sends what it lists and the ticked
  deletions at once; it asks again only for a lot at once.
- **The mass-change brake counts files, not percent**: it asks again for more than 20
  files at once, or more than 5 files of one folder replaced or moved to the trash. A
  single edit in a small folder no longer asks twice.
- **Two-way sync overwrites edited files on ILIAS** through the file's "Replace File"
  instead of adding a version with "Create New Version". ILIAS keeps only your copy;
  its earlier versions are deleted (the review says so). As before only
  where the file's menu offers Versions, and only if ILIAS still has the version you
  edited.

### Fixed

- **A rename that only changes the case is a rename**: on a disk that ignores case (macOS
  and Windows by default), renaming `skript.pdf` to `Skript.pdf` made sync see both names
  and report the new one as a copy of the old one. It now renames ILIAS' file (or, without
  Versions there, uploads it and proposes the old one for the trash), as for any rename.
  Folders renamed only in case still keep their ILIAS name on your disk.
- **Large syncs record their downloads in one write**: the saved-files list was rewritten
  once per file downloaded, which took several seconds on a first sync of a few thousand
  files; the diff also looks ILIAS' files up by id instead of searching the list for each.
- **Your older edit no longer replaces ILIAS' newer file after a conflict**: once ILIAS'
  copy was saved beside your edited one, the next sync proposed replacing ILIAS' file with
  yours (deleting its earlier versions). Now yours waits as a conflict until you delete the
  copy beside it, the sign that you merged what you need.
- **An upload whose recording was lost no longer leaves a duplicate**: if sync stopped
  after ILIAS took an upload, the next sync saved ILIAS' file again as `… (1)` and called
  yours a conflict. Now a file of yours with the same content is taken as that file.
- **A move where ILIAS doesn't let you delete the old file** is a conflict instead of an
  upload, which left the file on ILIAS twice.
- **Idle CPU**: the TUI drew the whole screen 20 times a second even when nothing
  changed (about 12% CPU in a debug build). It now draws only after a key, a click, a
  resize or a result, and while the spinner turns; idle it uses next to none.
- **Renames among files with the same content**: a saved file already gone from ILIAS
  made a rename look unclear, so it was uploaded again and the old one trashed. Only files
  proposed for the trash now count as what you renamed.
- **The status line after deleting a folder** counted its files as ones ILIAS doesn't let
  you delete.
- **Sync after changing the download folder** kept updating files in the old folder, and
  two-way sync proposed them for the ILIAS trash once that folder was gone. Now only copies
  in the current download folder count: files are saved there again, and old copies are
  left alone.
- **Sync after midnight**: ILIAS lists recent dates as "Today, 12:41", which becomes
  "Yesterday, 12:41" the next day, so every such file looked changed on ILIAS. Sync
  fetched untouched files again, and two-way sync took your edits for conflicts. Sync
  now records dates in full and reads older entries against the day they were saved.

## 0.4.0 - 2026-11-09

### Added

- **Two-way sync** (settings, `ILIAS_TWO_WAY=1`; off by default), first part: sync compares
  your copy of each synced course with what it last saved and with ILIAS, and lists in the
  report what it would send: new files, edited files (as new versions), files you deleted
  (for the trash), new folders, conflicts and files gone on ILIAS.
- **Two-way sync uploads new files** after naming each one and asking (a dry run stops at
  the plan). Each folder is read again right before; a file of the same name that appeared
  since is a conflict. More than 20 files, or more than 30 % of a folder, asks once more.
  Uploaded files count as in sync afterwards, and every write goes into a journal
  (`<host>.journal.jsonl` next to the session).
- **Two-way sync uploads your edits as new versions** through the file's own "Create New
  Version" (never "Replace File", so ILIAS keeps the older versions), only if the file is
  still at the version you edited, and checks that ILIAS lists the new version.
- **The sync review**: after a two-way sync the report shows what Enter sends and lists
  the files you deleted as proposals for the ILIAS trash, unticked until you tick them
  (`Space`); `Esc` sends nothing. Ticked ones go to the trash only if ILIAS has one and
  still has the version you deleted.
- **Two-way sync creates your new folders** on ILIAS (where "Add New Item" offers a
  folder), with the folders and files in them, and uploads those files.
- **Content hashes** in the saved-files list: a file whose date changed but whose content
  didn't is no longer "edited", so sync can update it.

- **`Tab` completes paths** in the file dialog of `u` (upload) and `H` (hand in), as a shell
  does: the only file or folder that fits, else as far as all of them agree, with the
  matches listed below the input.

### Fixed

- **`D` in lists cut short behind "Show more"**: an item after the block limit could not be
  moved to the trash, because its menu only comes with its batch. Its batch is read now.

### Changed

- **The sync review is the question**: `Enter` sends what it lists and the ticked
  deletions at once; it asks again only for a lot at once.
- **The mass-change brake counts files, not percent**: it asks again for more than 20
  files at once, or more than 5 files of one folder replaced or moved to the trash. A
  single edit in a small folder no longer asks twice.
- **A shorter README** (about half as long): the details of what is stored and sent moved
  to `docs/security.md`, development and releases to `CONTRIBUTING.md`.
- **The help (`?`) is grouped** (Items, Sync, Exercises, Edit, …) with shorter texts, wraps
  lines that don't fit instead of cutting them off, and uses two columns on wide terminals.

## 0.3.0 - 2026-11-09

### Added

- **Rights**: ilias-tui reads from each course and folder page what ILIAS lets you change
  there (upload, "Add New Item", Delete), to offer the editing keys only where they work.
  `ilias-tui show` prints them. Nothing changes for students.
- **Upload files** (`u`) into the course or folder on screen, where ILIAS lets you (also
  shared folders): one or more at once, with each file's size in the question. ilias-tui
  checks that ILIAS lists them afterwards.
- **New folder** (`n`) in the course or folder on screen, where ILIAS offers it.
- **Delete into the trash** (`D`): moves the selected item to the ILIAS trash, where its menu
  offers Delete. Refuses where ILIAS has no trash.
- **Forum posts**: `n` in a forum starts a thread, `R` replies to the topmost post on screen
  in a thread. The text is written in `$VISUAL` / `$EDITOR` (or one-line prompts without
  one), and the question names subject, forum and how the text starts.
- **Submissions of an assignment** (for those who grade): the "Submissions and Grades" tab
  lists every participant with what they handed in, one tab per assignment; `a` saves them
  all into `<course>/<exercise>/<assignment>/<participant>/`, and only what changed the
  next time.
- **Windows support** (Windows 10 and newer; so far checked by CI only, not by hand).
  Files and links open without going through `cmd`, downloads get the Mark of the Web,
  `y` copies with `clip`, and posts are written in Notepad when no `$EDITOR` is set.

### Changed

- **The sync review is the question**: `Enter` sends what it lists and the ticked
  deletions at once; it asks again only for a lot at once.
- **The mass-change brake counts files, not percent**: it asks again for more than 20
  files at once, or more than 5 files of one folder replaced or moved to the trash. A
  single edit in a small folder no longer asks twice.
- Saved file and folder names are valid on Windows too: `<>:"|?*` become `_`, trailing dots
  and spaces are dropped, device names such as `CON` get a `_` in front.

### Fixed

- Forum threads scroll again with `j` / `k`; each frame put them back at the top.

## 0.2.0 - 2026-11-09

### Added

- **"Show more" lists**: lists ILIAS cuts short behind its "Show more" button are loaded in
  full, for browsing, sync and the deep search (`s`).
- **What's new since your last visit**: new and changed items are marked `● new`; forums and
  threads with new posts `● new posts`. After a sync, courses and every folder on the way
  show how much is new below them. The settings choose which kinds are shown (files,
  folders, forum posts, other).
- **Links in page text** (e.g. a Zoom link) are entries you can select and open (`Enter`,
  `o`); links to other sites than ILIAS ask first.
- **`y`** copies the selected item's permalink.
- **The `/` filter is kept** when you go back to the list you filtered.
- **`ilias-tui --version`**, and the version in the top bar and the help.
- CI on GitHub Actions: formatting, tests and clippy on macOS and Linux, and `cargo audit`.

### Changed

- **The sync review is the question**: `Enter` sends what it lists and the ticked
  deletions at once; it asks again only for a lot at once.
- **The mass-change brake counts files, not percent**: it asks again for more than 20
  files at once, or more than 5 files of one folder replaced or moved to the trash. A
  single edit in a small folder no longer asks twice.
- **Faster start**: My courses shows from the cache at once while the session is checked
  in the background, and a start reads the course list once instead of twice. The To-Do
  remembers each exercise's course, so exercise pages are only read for new exercises.
- Reading posts elsewhere no longer marks a forum as new.

### Fixed

- Typing `c` or `?` into the filter no longer opens the settings or the help.
- The first frame no longer says "My courses is not available" before the cached page.
- Favourites are no longer marked as new on every refresh.

## 0.1.0

The first version: My courses with favourites and To-Do, browsing courses, folders,
forums and exercises, opening and downloading files, sync, the `/` filter and the deep
search, offline mode, handing in and deleting files, favourites, themes and settings.
