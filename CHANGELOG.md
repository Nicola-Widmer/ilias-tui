# Changelog

What changed in each version of ilias-tui. Versions follow
[semantic versioning](https://semver.org): before 1.0, the middle number goes up for new
features and changes in behaviour, the last one for fixes only. See
[Versions](README.md#versions) for how a release is made.

## Unreleased

### Added

- **`Tab` completes paths** in the file dialog of `u` (upload) and `H` (hand in), as a shell
  does: the only file or folder that fits, else as far as all of them agree, with the
  matches listed below the input.

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
