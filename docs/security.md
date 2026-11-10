# Security

What ilias-tui keeps, what it sends, and how it opens things. The same protections apply on
macOS, Linux and Windows unless a point says otherwise. The short version is in the
[README](../README.md#security).

## Login

**Your password never touches ilias-tui.** You log in yourself, in a real Chrome window,
on the normal HSLU / Switch edu-ID pages, 2FA included. Chrome runs with a fresh,
temporary profile that is deleted afterwards, so it doesn't see your normal browser data.

**Only the cookies it needs are kept**: those of ILIAS and of its login server (HSLU's
Keycloak). The Switch edu-ID cookies are dropped, because they could work for other edu-ID
services. (A session saved by an older version may still contain them; run
`ilias-tui login` once to replace it.)

**How long a login lasts.** There are two sessions: ILIAS' own (`PHPSESSID`, short) and
Keycloak's (`KEYCLOAK_IDENTITY`, usually much longer). When the ILIAS session expires,
ilias-tui runs the OpenID Connect redirect again with the saved Keycloak cookies and gets a
new ILIAS session without asking you. Only when the Keycloak session has expired too do you
see `session expired: press L to log in again`. See [architecture.md](architecture.md) for
the details.

## The session file

**It is as good as your login.** Anyone who gets a copy can use ILIAS as you until the
sessions expire. So:

- It's stored in your data folder, which only your user can open:
  - macOS: `~/Library/Application Support/ch.ilias-tui.ilias-tui/`, mode `0700`
  - Linux: `~/.local/share/ilias-tui/`, mode `0700`
  - Windows: `%APPDATA%\ilias-tui\ilias-tui\data\`, inside your user profile, which
    Windows already keeps to your account (ilias-tui sets no extra permissions there)
- The file itself is mode `0600` (macOS, Linux). It is **not encrypted** (yet; see
  [roadmap.md](roadmap.md#security)), so treat it like a password. Don't commit, sync or
  share it, and remember that backups (e.g. Time Machine, Déjà Dup) include it.

**`ilias-tui logout` logs out on the server too.** It ends your ILIAS session through
ILIAS' own logout, then deletes the local session and the page cache. It also checks
whether HSLU's Keycloak login is still active; Keycloak can only be logged out through a
confirmation page in the browser, so if it is, `logout` tells you to log out there. If you
think the session file leaked: run `ilias-tui logout`, and if in doubt change your edu-ID
password.

## What else is stored locally

- **The page cache**: pages you've opened, for instant revisits. It lives in the system
  cache folder (`~/Library/Caches/ch.ilias-tui.ilias-tui/` on macOS, `~/.cache/ilias-tui/`
  on Linux with mode `0700`, `%LOCALAPPDATA%\ilias-tui\ilias-tui\cache\` on Windows), which
  backup tools usually skip. Entries older than 30 days are deleted automatically;
  `ILIAS_CACHE=0` turns it off.
- **The saved-files list**: which ILIAS file was saved to which path, with its version
  on ILIAS and the size, date and SHA-256 hash of the saved copy (in the data folder).
  Two-way sync reads your synced course folders to compare them with it.
- **The what's-new list** (`<host>.seen.json`, in the data folder): items a sync found new
  and the folders they are in, until you open those pages.
- **The journal** (`<host>.journal.jsonl`, in the data folder): every write two-way sync
  sent or tried, with the local path of the file.

Downloaded files go to `~/Downloads/ILIAS` with normal permissions, like any browser
download.

## What it sends, and where

Only to your ILIAS server (`ILIAS_URL`), over HTTPS with certificate checks. During the
login, Chrome also talks to Keycloak and edu-ID. There is no telemetry and no third-party
service.

ilias-tui reads with `GET` requests. The one `POST` that only reads is ILIAS' "Show more"
button, which asks for the rest of a long list; it goes only to your ILIAS and only with
that button's command (`renderBlockAsynch`).

These change something on ILIAS. Each one names what it sends where and asks first (except
the logout you ask for), sends nothing with `ILIAS_DRY_RUN=1`, and only uses a link or form
that ILIAS shows on the page, after checking that it is on your ILIAS and for that very
object:

| Key | What it changes | Checked before sending |
| --- | --- | --- |
| `f` | Adds or removes a favourite | The favourites link from that item's own menu |
| `H` | Hands in a file for an assignment; your lecturer sees it | The assignment's own upload form |
| `X` | Deletes a file you handed in, for good; ILIAS allows it only until the deadline | The Delete of that file on the assignment's page |
| `u` | Uploads files into a course or folder; everyone who can open it sees them | That place's own upload form; files over ILIAS' size limit are not sent |
| `S` / `A` with two-way sync | Uploads your new files of synced courses into their ILIAS folders | The same upload form, read again right before; nothing if a file of that name appeared since. Asks once more for more than 20 files, or more than 5 of one folder replaced or trashed; every write goes into the journal |
| `S` / `A` with two-way sync | Creates your new local folders (and the folders in them) and uploads their files | As `n` and `u`: each place's own "Add New Item → Folder" and its form, then the new folder's own upload form; nothing if something of that name appeared since the plan |
| `S` / `A` with two-way sync, unless unticked in the review | Moves a file you deleted (or moved to another folder) into the ILIAS trash (never empties it) | As `D`: that file's Delete from its folder's menu (also behind "Show more") and ILIAS' question; refused without a trash, or if the file changed on ILIAS or is back on your disk |
| `S` / `A` with two-way sync | Replaces a file with the copy you edited; ILIAS deletes the file's earlier versions for good | The file's Versions entry in its menu, that tab's "Replace File" and its form (never "Create New Version"), all for that file; nothing if the file changed on ILIAS since the plan |
| `S` / `A` with two-way sync | Renames a file you renamed in the same folder; ILIAS deletes the file's earlier versions for good | The same "Replace File" form, with your new name as the title; nothing if the file changed on ILIAS since the plan or the old name is back on your disk |
| `n` | Creates a folder in a course or folder | That place's own "Add New Item → Folder" and the form it leads to |
| `D` | Moves an item into the ILIAS trash (never empties it) | The Delete in that item's own menu and ILIAS' question for it; refused where ILIAS has no trash |
| `n` in a forum, `R` | Starts a thread or replies to a post; everyone in the forum sees it | That forum's "New Thread" form, or that post's Reply form |
| `ilias-tui logout` | Ends your session | The logout link of the page |

All of them live in `src/ilias/write.rs` and `src/ilias/write/`. Every other request passes
a **write guard** that refuses anything that looks like a write (an ILIAS one-time `rtoken`,
or a known write command such as `addToDesk`, `markPostRead` or `join`), so a bug elsewhere
can't turn a link into a change. Tests check this against a fake ILIAS. Other features that
would write are deliberately not built; see the
[rules for features that write](roadmap.md#rules-for-features-that-write).

## Opening things safely

- **Files that can run code are never opened.** Anyone who can upload to a shared folder or
  forum could place a `notes.command`, `setup.sh`, `.desktop`, `.app`, `.jar`, `.AppImage`,
  `.exe`, `.bat`, `.lnk`, …. For these, `Enter` only shows the file in your file manager
  (Finder, Files/Dolphin, Explorer) and says why. All other files open with their default
  app.
- **Downloads are never executable.** On macOS they also get the quarantine mark browsers
  set, so Gatekeeper checks them before anything inside can run. On Windows they get the
  browsers' "Mark of the Web" (`Zone.Identifier`), so SmartScreen and Office's protected
  view treat them as downloaded. Linux has no equivalent mark.
- **No shell in between.** Files and links are handed to `open` (macOS), `xdg-open` (Linux)
  or `rundll32 url.dll,FileProtocolHandler` (Windows) as one argument, never through `cmd`,
  which would read the `&` in ILIAS links as a command separator.
- **Only web links open in the browser, and other sites only after asking.** A link must be
  `http(s)`, never `file:` or an app's URL scheme. Links to your ILIAS open right away; a
  link in page text to any other site (a Zoom link, say) names the address and asks first.
  Links that would change something on ILIAS (the write guard's list) are not offered.
- **File names from ILIAS are sanitised** before saving, so a crafted name can't write
  outside the download folder, and every name is valid on Windows too (no `<>:"|?*`, no
  trailing dots, no device names like `CON`).

## AI-written code

The code was written with an AI assistant and hasn't had a thorough human security review.
The parts that matter most are `src/auth.rs` (login), `src/session.rs` (session storage),
`src/open.rs` (opening files and links), `src/ilias/mod.rs` (requests, downloads, logout)
and `src/ilias/write.rs` with `src/ilias/write/` (everything that changes ILIAS). Read them
before you trust the tool with your account.
