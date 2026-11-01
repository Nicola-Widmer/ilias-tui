# Roadmap

Features that ilias-tui does not have yet, grouped by risk. The first two groups only read
from ILIAS. The third group would change data on ILIAS and needs extra care (see
[Rules for features that write](#rules-for-features-that-write)).

When a feature is built, tick it off here in the same commit.

## Suggested next steps

1. **Check the "show more" gap**: ILIAS can truncate long lists behind a "show more"
   button, and items behind it may currently be missing. See [Reading content](#reading-content).
2. **What's new since my last visit**
3. **Download / sync a whole folder or course**

## Reading content

Read-only; safe to build and to try against ILIAS.

- [ ] **"Show more" lists.** ILIAS can truncate long item lists behind a "show more"
  button. The parser skips that button (`ilContainerShowMore` in `src/ilias/parse.rs`), so
  items behind it may be **silently missing**. Check first whether HSLU uses it.
- [ ] **What's new since my last visit.** Show new or changed files per course. The page
  cache (`src/cache.rs`) already holds the previous version to compare against.
- [x] **Upcoming deadlines.** The To-Do list on "My courses" (from the ILIAS dashboard).
  Covers exercise assignments only.
- [ ] **Test and session dates** in the To-Do list. The dashboard block only lists
  exercises; tests and sessions would need the calendar.
- [x] **Forums.** Read threads and posts in the TUI.
- [ ] **Long forums and threads.** Check whether ILIAS splits long thread lists or threads
  into pages; only the first page is read today.
- [ ] **Links and attachments in forum posts.** Today only the text is shown.
- [ ] **Exercises.** Show assignments, deadlines and your own submission status (today:
  opens in the browser).
- [ ] **Calendar and session dates.**
- [ ] **ILIAS mail.** Read the inbox.
- [ ] **Wikis, blogs, learning modules, content pages.** Read in the TUI (today: open in the
  browser).
- [ ] **Structured Info / Members / Learning progress tabs.** Today they are shown as plain
  text.
- [ ] **Links in page text.** Make them selectable and openable (e.g. the Zoom link). Today
  they are only highlighted.
- [ ] **Images and tables in page text.** Today they are skipped or flattened into text.
- [ ] **Videos / Mediacast.** Hand off to a player such as `mpv` (today: open in the browser).
- [ ] **ILIAS server-side search.** Today there is only the local `/` filter and the `s`
  crawl.
- [ ] **Favourites ("Favoriten") and recently visited items.** Today only "My courses" is
  shown.
- [ ] **Browse the whole repository.** All categories, not just your own courses.

## Downloads and files

Read-only; safe to build.

- [ ] **Download / sync a whole folder or course** to disk, like
  [PFERD](https://github.com/Garmelon/PFERD). Today: one file at a time.
- [ ] **Detect updated files.** Re-download when the date or size in ILIAS changed. Today `d`
  always fetches a fresh copy without comparing.
- [ ] **Download progress bar and cancel.** Today there is only a spinner.
- [ ] **Download queue.** Several downloads at once, with a list of what is running.
- [ ] **Mirror the course structure** in the download folder (`ILIAS/<course>/<folder>/…`).
  Today everything lands flat in `~/Downloads/ILIAS`.
- [ ] **File versions.** List and download older versions.

## Changes data on ILIAS

**Not built on purpose.** Each of these needs the safeguards below before it is added.

- [ ] Upload exercise submissions
- [ ] Upload files to shared folders
- [ ] Write forum posts and replies
- [ ] Send ILIAS mail
- [ ] Join or leave courses and groups
- [ ] Add or remove favourites
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
ILIAS links handed to the browser, cache opt-out and 30-day cleanup.

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

- [ ] **Copy permalink** of the selected item (e.g. `y`).
- [ ] **Keep the filter after going back.** Today the filter resets on every navigation.
- [ ] **Light theme / configurable colours.** Today the colours are tuned for dark
  terminals (`src/theme.rs`).
- [ ] **Config file** for keys, download folder and theme. Today there are only environment
  variables.
- [ ] **Offline mode.** Browse cached pages without a connection. Today they are shown, but
  the background refresh fails with an error.
- [ ] **Cache size limit / cleanup.** Today the cache only grows; only `ilias-tui logout`
  clears it.
- [ ] **Several accounts or ILIAS installations** side by side. Today: one at a time via
  `ILIAS_URL`.
- [ ] **Packaging.** Release binaries, a Homebrew formula, CI that runs tests and clippy.
- [ ] **Tests for logged-in pages.** Today the parser tests use small HTML snippets modelled
  on real pages. Real logged-in pages contain personal data and must not be committed as
  fixtures.
