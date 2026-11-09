# Roadmap

Features that ilias-tui does not have yet, grouped by risk. The first two groups only read
from ILIAS. The third group would change data on ILIAS and needs extra care (see
[Rules for features that write](#rules-for-features-that-write)).

When a feature is built, tick it off here in the same commit.

## Suggested next steps

## Reading content

Read-only; safe to build and to try against ILIAS.

- [x] **"Show more" lists.** Lists ILIAS cuts short behind "Show more" (a container's
  block limit) are loaded in full for browsing, sync and deep search. Tested on the local
  ILIAS; whether HSLU uses it is unchecked (`ilias-tui dump <url> | grep ilContainerShowMore`).
- [x] **What's new since my last visit.** Items new or changed since the cached copy are
  marked `● new`; after a sync, My courses counts them per course until you open it.
- [x] **Upcoming deadlines.** The To-Do list on "My courses" (from the ILIAS dashboard).
  Covers exercise assignments only.
- [ ] **Test and session dates** in the To-Do list. The dashboard block only lists
  exercises; tests and sessions would need the calendar.
- [x] **Forums.** Read threads and posts in the TUI.
- [ ] **Long forums and threads.** Check whether ILIAS splits long thread lists or threads
  into pages; only the first page is read today.
- [ ] **Links and attachments in forum posts.** Today only the text is shown.
- [x] **Exercises.** Assignments (ongoing, upcoming, past), deadlines, instructions, attached
  files and your own submission status.
- [ ] **Calendar and session dates.**
- [ ] **ILIAS mail.** Read the inbox.
- [ ] **Wikis, blogs, learning modules, content pages.** Read in the TUI (today: open in the
  browser).
- [ ] **Structured Info / Members / Learning progress tabs.** Today they are shown as plain
  text.
- [x] **Links in page text.** Each link becomes an entry below its paragraph (`j`/`k`,
  `Enter` / `o` open it; other sites than ILIAS ask first).
- [ ] **Images and tables in page text.** Today they are skipped or flattened into text.
- [ ] **Videos / Mediacast.** Hand off to a player such as `mpv` (today: open in the browser).
- [ ] **ILIAS server-side search.** Today there is only the local `/` filter and the `s`
  crawl.
- [x] **Favourites ("Favoriten")** on top of "My courses".
- [ ] **Recently visited items.**
- [ ] **Browse the whole repository.** All categories, not just your own courses.

## Downloads and files

Read-only; safe to build.

- [x] **Sync whole courses or folders** (`S`, `Space` to mark several): files up to 50 MB,
  only changed ones, your edits never overwritten.
- [x] **Sync report** listing the skipped big files, new versions saved beside your edits,
  and failures.
- [ ] **Sync from the command line** (`ilias-tui sync URL…`), e.g. for cron.
- [ ] **Sync through the fake ILIAS in tests.** Downloads bypass the `Pages` seam, so only
  sync's rules are tested, not a whole run.
- [ ] **Download progress bar and cancel.** Today there is only a spinner.
- [ ] **Download queue.** Several downloads at once, with a list of what is running.
- [x] **Mirror the course structure** in the download folder (`ILIAS/<course>/<folder>/…`).
- [ ] **Folders for `ilias-tui get`.** It only has the file's id, so it saves flat; it would
  need one more request (the file's info page) for the breadcrumb.
- [ ] **File versions.** List and download older versions.

## Changes data on ILIAS

**Not built on purpose.** Each of these needs the safeguards below before it is added.

- [x] Upload exercise submissions (`H`; asks first, tested only on the local test ILIAS)
- [x] Delete a handed-in file before the deadline (`X`; asks first, tested only on the local
      test ILIAS)
- [ ] Upload files to shared folders
- [ ] Write forum posts and replies
- [ ] Send ILIAS mail
- [ ] Join or leave courses and groups
- [x] Add or remove favourites (`f`; asks first, `ILIAS_DRY_RUN`, `src/ilias/write.rs`)
- [ ] Mark items as read, set learning progress
- [ ] Ratings, notes, tags
- [ ] Take tests and surveys (probably better left to the browser)

### Rules for features that write

- **Ask before every write.** A confirmation dialog shows exactly what will be sent where
  ("Upload `sheet-3.pdf` to *Submissions › Sheet 3*?").
- **Dry-run mode.** An environment variable (e.g. `ILIAS_DRY_RUN=1`) makes every write only
  log what it would do. Use it while developing and testing.
- **Keep writes separate.** Put write requests in their own module (e.g. `src/ilias/write.rs`)
  so every request that changes data is easy to review in one place.
- **No writes from the CLI without a flag.** Commands like `ilias-tui ls`, `show`, `dump` and
  `get` stay read-only.
- **Never test against real courses.** Use a test course or the dry-run mode. Exercise
  submissions and forum posts are visible to lecturers and classmates.

## Security

Done: owner-only data and cache folders, only ILIAS + Keycloak cookies kept, server-side
logout, no auto-opening of files that can run code, macOS quarantine on downloads, only
ILIAS links handed to the browser, cache opt-out and 30-day cleanup, a write guard in
front of every page request (only `src/ilias/write.rs` gets past it).

- [ ] **Encrypt the session file** with a key from the OS secret store. macOS: the Keychain
  via the `security` tool, which avoids losing access after every rebuild. Linux: Secret
  Service (GNOME Keyring / KWallet). Linux needs a fallback where no secret store exists
  (SSH, servers, minimal desktops): the current `0600` file with a warning, or a passphrase.
- [ ] **End the Keycloak session on logout.** Keycloak only logs out through a confirmation
  form (or with an ID token we don't have), so today `logout` only reports that it's still
  active.
- [ ] **Dependency checks in CI** (`cargo audit`, `cargo deny`) on macOS and Linux.
- [ ] **Human security review** of `src/auth.rs`, `src/session.rs`, `src/open.rs`,
  `src/ilias/mod.rs`.

## App and quality of life

- [x] **Copy permalink** of the selected item (`y`; `pbcopy` on macOS, else OSC 52).
- [ ] **Keep the filter after going back.** Today the filter resets on every navigation.
- [x] **Light theme / configurable colours.** Dark, light or hub in the settings, `colors` in
  `config.json` on top.
- [ ] **Config file** for keys, download folder and theme. Today there are only environment
  variables.
- [x] **Offline mode.** `--offline`, or automatic when ILIAS can't be reached: cached pages
  and saved files only, nothing is sent; `L` goes online.
- [x] **Offline pages for synced courses.** Sync caches every course and folder page it
  reads.
- [x] **Offline deep search** over the cached pages (complete for synced courses).
- [ ] **Offline tabs of synced courses** (Info, Members, …): sync caches only the content
  page of each course and folder.
- [ ] **Cache size limit / cleanup.** Today the cache only grows; only `ilias-tui logout`
  clears it.
- [ ] **Several accounts or ILIAS installations** side by side. Today: one at a time via
  `ILIAS_URL`.
- [ ] **Packaging.** Release binaries, a Homebrew formula, CI that runs tests and clippy.
- [x] **Tests for the client's logic.** `FakeIlias` (`src/ilias/pages.rs`) serves small,
  anonymised HTML snippets. Real logged-in pages contain personal data and must not be
  committed as fixtures.
- [ ] **Downloads through the fake ILIAS.** File downloads (and the Keycloak probe) still go
  to the network directly, so "session expired during a download" is untested.
