# Changelog

What changed in each version of ilias-tui. Versions follow
[semantic versioning](https://semver.org): before 1.0, the middle number goes up for new
features and changes in behaviour, the last one for fixes only. See
[Versions](CONTRIBUTING.md#versions) for how a release is made.

## Unreleased

### Changed

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
