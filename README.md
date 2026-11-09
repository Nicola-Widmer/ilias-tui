# ilias-tui

A terminal UI for [ILIAS](https://github.com/ILIAS-eLearning/ILIAS) 10, built with
[ratatui](https://ratatui.rs). It is set up for the
[HSLU ILIAS](https://elearning.hslu.ch/ilias/) (Keycloak + Switch edu-ID login), but
works with any ILIAS 10 that logs in through OpenID Connect.

> **Note:** The code and documentation in this project were written with the help of AI
> (Claude by Anthropic). See [Disclaimer](#disclaimer).

```
  ILIAS   My courses  DEMO.101  ·  Introduction to Examples
  Inhalt   Info   Mitglieder   Lernfortschritt   ⇥ switch
╭───────────────────────────────────────────────────────────────────╮
│  Welcome to Introduction to Examples                              │
│  Room: B-042                                                      │
│  Time: Monday, 09:00 - 11:30                                      │
│                                                                   │
│ ▍Course material                                                  │
│▌   Slides                                                         │
│      Lecture slides and recordings                                │
│     Course information                                            │
│      Schedule and general information                             │
│                                                                   │
│ ▍Assignments and discussion                                       │
│     Submissions                                                   │
│      Exercise  ·  Weekly exercise sheets                          │
│     Questions & Answers                                           │
│      Forum  ·  Posts (unread): 2 (14)                             │
╰───────────────────────────────────────────────────────────────────╯
 ✓ Saved ~/Downloads/ILIAS/Lecture_01.pdf   / filter  s search  ? help  q quit
```

(In the terminal: an ILIAS-navy header bar, coloured icons, yellow section headings, the
selected row highlighted, and a spinner in the status bar while something loads.)

What it does:

- Shows your ILIAS **favourites** first, then your courses and groups ("Meine Kurse und
  Gruppen"), grouped by year like on ILIAS but with the newest year first, with each
  course's full name. Favourites are marked `★` everywhere.
- **Adds and removes favourites** with `f`. This changes your ILIAS account, so it always
  asks first (`y` / `n`).
- Shows a **To-Do** panel to the right of them (below them on narrow terminals): the
  assignments you still have to hand in, soonest first, with the course and how long you
  have left ("due in 3 days", yellow within 3 days, red within a day). `Tab` moves into the
  panel; `Enter` opens the assignment.
- **Reads forums**: the threads of a forum (author, number of posts, unread posts, last
  post), and in a thread all posts with author, date, replies indented and unread posts
  marked `● new`. Reading changes nothing on ILIAS; posts stay unread there.
- **Writes forum posts**, where ILIAS lets you: `n` in a forum starts a thread, `R` in a
  thread replies to the topmost post on screen (marked `↩ R reply`). You write in your
  editor (`$VISUAL` / `$EDITOR`, on Windows Notepad if neither is set; give the editor by a
  name on your `PATH`, e.g. `code -w`; without one, in one-line prompts); before anything is sent
  it shows the subject, the forum and how the text starts, and asks.
- Shows courses, groups and folders **the way ILIAS does**:
  - the page text (welcome note, room, Zoom link, …), with its links as entries right
    below it: select one with `j`/`k` and press `Enter` or `o` to open it,
  - the named sections (item groups),
  - the tabs (Inhalt, Info, Mitglieder, …; switch with `Tab`),
  - each item's description and properties (file type, size, date, unread posts, …),
  - every item of long lists, also the ones ILIAS hides behind its "Show more" button.
- **Shows what's new since your last visit**: files and other items that appeared or changed
  since you last opened the page are marked `● new`; forums and threads with new posts are
  marked `● new posts`, in another colour (reading posts elsewhere doesn't count). After a
  sync, each course on My courses and each folder on the way shows how much is new below
  it (`● 3 new  ● new posts in 1 forum`), so you can follow the counts down. The settings
  (`c`) choose which kinds of news show: files, folders, forum posts, everything else.
- **Nerd Font icons** per object type and file type: PDF, Word, Excel, PowerPoint,
  images, archives, code, …
- **Filters** any list as you type (`/`).
- **Searches** every file and folder below the current place (`s`), for one course or all
  of them.
- **Opens files** with their default app (Preview, Excel, …) on `Enter`. The first time,
  the file is downloaded into the same folders as on ILIAS, e.g.
  `~/Downloads/ILIAS/DEMO.101/Unterlagen/Woche 3/Folien.pdf`. After that, the saved copy
  opens right away; saved files are marked `✓ saved`. (`ilias-tui get` only knows the
  file, not its folders, so it saves directly into `~/Downloads/ILIAS`.)
- **Syncs whole courses** (`S`, mark several with `Space`, or all at once with `A`): every file up to 50 MB,
  in ILIAS's folder structure, and later only what changed. Files you edited (say, an
  annotated PDF) are never overwritten; see [Sync](#sync).
- **Reads exercises**: the assignments, ongoing or past, with deadline and whether you handed
  something in; instructions and attached files on the assignment's page. `Enter` on a To-Do
  entry goes straight there.
- **Hands in files** for an assignment (`H`): it asks for the file (`Tab` completes the
  path), then asks you to confirm.
  Its **Handed-in files** entry (in the Submission section, open it like any other entry)
  lists what you handed in; `X` deletes the selected file again (asks first), as long as
  ILIAS allows it, which is until the deadline.
- **Knows what you may change** in a course or folder: if ILIAS offers you "Add New
  Item", an upload form or an item's Delete (as a lecturer, tutor or administrator, or in a
  shared folder), ilias-tui reads that from the page and offers the matching keys there.
  Nothing new appears where you may only read. `ilias-tui show URL` lists them as `rights:`.
  The keys that work on a page are named at the bottom of its frame.
- **Uploads files** into the course or folder on screen (`u`), where ILIAS lets you (also
  shared folders): it asks for one or more files (drop them on the terminal, or type them
  with `Tab` completing the path as in a shell), then names
  the place and each file's size and asks you to confirm. Afterwards it checks that ILIAS
  lists them.
- **Creates folders** in the course or folder on screen (`n`), where ILIAS offers "Add New
  Item → Folder": it asks for the name, then to confirm, and shows the new folder.
- **Deletes items** into the ILIAS trash (`D`), where the item's ILIAS menu offers Delete: it
  names the item, its kind and the place, says it goes to the trash, and asks first. Where
  ILIAS has no trash (it would be deleted for good), it refuses. It never empties the trash.
- **Lists everyone's submissions** for those who grade: an exercise's "Submissions and
  Grades" tab shows each participant with how many files they handed in, when, and the
  grade, with one tab per assignment. `d` (or `Enter`) saves one participant's files, `a`
  saves everyone's, into `~/Downloads/ILIAS/<course>/<exercise>/<assignment>/<participant>/`
  (several files come as ILIAS' zip). `a` again only fetches what changed. Read-only.
- Opens everything else (tests, surveys, …) in your normal web browser, and copies any
  item's permalink with `y`.
- Logs in once through Switch edu-ID (2FA included) and keeps the session in a private
  file. Later runs refresh the session on their own, without a browser.

## Motivation

It's 07:58. The lecture starts at 08:00. You just need the slides.

You open ILIAS. You wait. A spinner thinks about life. You click "My courses". You wait.
You click the course. You wait. You click "Course material". You wait. Somewhere
a PHP process sighs. You click the folder you clicked yesterday, and the day before, and
every Thursday this semester. ILIAS has never seen it before. It renders it from scratch,
with love, for the 34th time.

We measured it: around **half a second to a whole second per page**, every time, even for
a page that hasn't changed since September. Then the semester update arrives, and with it
a fresh set of surprises.

ILIAS has had over twenty years to learn that a folder you visited yesterday probably looks
the same today. It hasn't. So we taught it ourselves:

- **Pages you've seen open instantly.** We cache them and quietly check for changes in the
  background, like it's 2010 and caching is a thing.
- **Files you've downloaded open instantly.** No eleventh copy of `Lecture_01 (10).pdf`.
- **Searching a whole course** fetches folders in parallel instead of politely queueing
  like it's the Mensa at 12:05.
- **No mouse marathon.** `/ demo` and Enter, and you're in the course.

ILIAS still needs its second per page the first time; we can't fix the server. But we can
make sure you only wait for it once.

(Nothing personal, ILIAS. You've stored our slides faithfully for years. We just want
them a bit faster.)

## Requirements

- macOS, Linux or Windows 10 or newer (in Windows Terminal or another modern terminal).
- Rust 1.88 or newer (edition 2024). For example `brew install rust` or
  [rustup](https://rustup.rs) (on Windows, rustup with the MSVC build tools).
- Google Chrome or Chromium, used only for the login window.
- A [Nerd Font](https://www.nerdfonts.com) in your terminal (for example
  `brew install --cask font-jetbrains-mono-nerd-font`) for the icons. Without one, set
  `ILIAS_NERD_FONT=0` to get plain Unicode symbols instead.

## Install

```bash
cargo install --path .
```

Or run it straight from the checkout:

```bash
cargo run --release
```

## Usage

```bash
ilias-tui
```

On the first start, a Chrome window opens on the HSLU login. Log in with Switch edu-ID as
usual. When the window lands back on ILIAS, the login is done: the window closes and the
page loads. Later starts show your cached courses at once while the session is checked in
the background; Chrome only opens again when the session has expired.

| Command               | What it does                                             |
| --------------------- | -------------------------------------------------------- |
| `ilias-tui`           | Start the TUI (logs in first if needed; offline if ILIAS can't be reached) |
| `ilias-tui --offline` | Start the TUI offline: cached pages and saved files only, nothing is sent |
| `ilias-tui login`     | Force a fresh browser login                              |
| `ilias-tui logout`    | Delete the saved session file                            |
| `ilias-tui ls [URL] [--deep]` | Print a container's items as `kind<TAB>ref_id<TAB>title` (default: my courses). With `--deep`, print everything below it |
| `ilias-tui get URL…`  | Download files, e.g. `goto.php/file/123`, to the download dir |
| `ilias-tui todo`      | Print the assignments still to hand in, soonest first    |
| `ilias-tui show URL`  | Print how a page is parsed: tabs, headings, text, items (for debugging) |
| `ilias-tui dump URL`  | Print the HTML ILIAS returns for `URL` (for debugging)   |
| `ilias-tui --version` | Print the version (also shown in the top bar and the help) |

`URL` can be absolute or relative to the ILIAS root, e.g. `ilias-tui ls goto.php/crs/123`.
`ls` also works for scripting:

```bash
ilias-tui ls goto.php/crs/123 --deep | grep -i folien
```

```bash
ilias-tui get goto.php/file/123456
```

### Searching

| Key | What it does |
| --- | ------------ |
| `/` | Opens the filter bar under the list. Typing narrows the **current list** right away. Every word must appear in the title, in any order, ignoring case: `folien 3` finds "Woche 3 › Folien". `↑`/`↓` move through the matches while you type. `Enter` opens the selected match right away. `Esc` clears the filter. A list keeps its filter while you open something in it: going back brings it back as you left it. |
| `s` | **Deep search.** Collects every file and folder below where you are and lists them with their path (`Unterlagen › Woche 3 › Folien.pdf`), then opens the filter bar. In a course it covers that course. On "My courses" it covers all your courses, which takes longer (progress shows in the status bar). `r` re-runs it, `Esc` goes back. |

### Keys

| Key             | Action                                          |
| --------------- | ----------------------------------------------- |
| **Navigate**    |                                                 |
| `↑ ↓` / `k j`   | Move selection; scroll in a forum thread        |
| `g` / `G`       | First / last item                               |
| `Enter` `→` `l` | Open folder, forum, thread or exercise; open file (downloads it the first time); open a link or anything else in the browser (asks first for links to other sites) |
| `Esc` `←` `h`   | Clear filter, then go back                      |
| `Tab` / `Shift-Tab` | Next / previous tab (Inhalt, Info, …); on My courses, in and out of the To-Do panel |
| **Search**      |                                                 |
| `/`             | Filter the current list                         |
| `s`             | Search everything below here                    |
| **Items**       |                                                 |
| `d`             | Download a fresh copy without opening it        |
| `o`             | Open selected item or link in your web browser (asks first for links to other sites) |
| `y`             | Copy the selected item's permalink (or the page's) to the clipboard |
| `f`             | Add to / remove from your ILIAS favourites (asks first) |
| `r`             | Reload current list                             |
| **Sync**        |                                                 |
| `Space`         | Mark a course or folder for sync                |
| `S`             | Sync the marked courses (or the selected one)   |
| `A`             | Sync all courses in My courses (asks first)     |
| **Exercises**   |                                                 |
| `H`             | On an assignment: hand in a file (asks first)   |
| `X`             | Handed-in files: delete the selected file (asks first) |
| `a`             | On an assignment's submissions page: download every participant's submission |
| **Edit**        | Only where ILIAS lets you; each asks first      |
| `u`             | Upload files into the course or folder on screen |
| `n`             | New folder in the course or folder on screen; in a forum, a new thread |
| `D`             | Move the selected item to the ILIAS trash       |
| `R`             | In a forum thread: reply to the topmost post on screen, marked `↩ R reply` |
| **App**         |                                                 |
| `L`             | Log in again / go online (opens Chrome if needed) |
| `c`             | Settings (download folder, sync, icons, theme, …) |
| `?`             | Toggle help                                     |
| `q` / `Ctrl-c`  | Quit                                            |

### Mouse

| Action                 | What it does                                          |
| ---------------------- | ----------------------------------------------------- |
| Click an item          | Select it                                             |
| Double-click an item   | Open it (same as `Enter`), also in the To-Do panel    |
| Scroll wheel           | Move the selection                                    |
| Click a tab            | Switch to that tab                                    |
| Click a breadcrumb     | Jump back to that level                               |
| Click anywhere         | Close the help overlay                                |

While the app has the mouse, selecting text with the mouse needs a modifier: hold
**Option** (iTerm2, Terminal.app) or **Shift** (most other terminals). `ILIAS_MOUSE=0`
turns mouse support off.

### Configuration

Press `c` for the settings page: every setting below, one per row (`↑`/`↓` choose, `↵`
toggles or edits, `r` resets to the default). Changes are saved right away in
`config.json` in the app's config folder. Mouse support, page cache, start offline and
the ILIAS URL apply at the next start; the rest at once. The settings page also has
**Sync missing files only** (see [Sync](#sync)).

Each setting can also be set with an environment variable, which wins over the settings
page:

| Environment variable  | Default                              |
| --------------------- | ------------------------------------ |
| `ILIAS_URL`           | `https://elearning.hslu.ch/ilias/`   |
| `ILIAS_DOWNLOAD_DIR`  | `~/Downloads/ILIAS`. A folder chosen in the app applies at once; the variable wins again at the next start |
| `ILIAS_NERD_FONT`     | `1`. Set to `0` for plain Unicode icons |
| `ILIAS_MOUSE`         | `1`. Set to `0` to turn off mouse support |
| `ILIAS_CACHE`         | `1`. Set to `0` to turn off the page cache |
| `ILIAS_SYNC_MAX_MB`   | `50`. Sync skips files bigger than this         |
| `ILIAS_OFFLINE`       | `0`. Set to `1` to always start offline (same as `--offline`) |
| `ILIAS_DRY_RUN`       | `0`. Set to `1` to never send changes to ILIAS (favourites, handing in); the app only says what it would have done |

### Themes

The settings (`c`) switch between **dark** (on your terminal's background), **light** and
**hub** (orange on black). All use 24-bit colour, which almost every modern terminal supports (older versions of macOS
Terminal.app don't; iTerm2, Ghostty, WezTerm, Kitty and Alacritty do).

For your own colours, add `colors` to `config.json` (the settings show where it is) and
restart. Set only the ones you want to change; the rest come from the theme:

```json
{
  "theme": "dark",
  "colors": { "bg": "#1e1e2e", "fg": "#cdd6f4", "accent": "#f5c2e7" }
}
```

A colour is `#rrggbb`, a name like `red`, or `reset` for the terminal's own. The names are
`bg`, `fg`, `muted`, `border`, `selected_bg`, `bar_bg`, `header_bg`, `header_fg`, `accent`, and
`blue`, `cyan`, `green`, `yellow`, `orange`, `red`, `purple`, `magenta`, `gray` (which also
colour the icons). The settings list any entry they can't use.

To use another school's ILIAS, point `ILIAS_URL` at the folder that contains `login.php`
and `openidconnect.php`.

For development there is a throwaway local ILIAS in Docker, see [dev/README.md](dev/README.md).

## Sync

`S` saves all files of the selected course or folder into the download folder, in the same
folders as on ILIAS (`~/Downloads/ILIAS/<course>/<folder>/…`). Mark several with `Space`
first (they show `◉ sync`), then `S` syncs all of them. `A` syncs every course in My
courses, after asking. Progress shows in the status bar,
and at the end a summary: new, updated, unchanged, skipped. If any files were skipped for
their size, saved beside your edited copy, or failed, a **sync report** opens and lists
them by name (`j`/`k` scrolls, any other key closes it).

- **Only reasonably sized files:** up to 50 MB (settings page or `ILIAS_SYNC_MAX_MB`). Lecture recordings
  and other big files are skipped and counted.
- **Only what changed:** ILIAS lists each file's size and date. A file is fetched again
  only if those changed, so syncing an unchanged course just reads its folder pages.
- **Your edits are safe:** sync remembers each file's size and modification time right
  after saving it. If they differ later, you changed it, and sync never overwrites it.
  When such a file changes on ILIAS, the new version is saved beside yours, e.g.
  `Blatt 3 (ILIAS 2026-11-02).pdf`. Files downloaded before sync existed count as edited.
- **Deleted is deleted:** a file you removed comes back only if it changes on ILIAS.
- **Sync missing files only** (settings page, off by default): sync downloads every file
  that isn't on disk, also the ones you deleted, and leaves every file you have exactly as
  it is: no updates, no new versions beside your copies.
- **Speed:** folders are read 6 at a time and files downloaded 3 at a time, so the server
  isn't flooded. Nothing is half-written: a download goes to a new file first and only
  then replaces an unedited copy.

- **Browsable offline:** every course and folder page sync reads is kept in the page
  cache, so you can walk through a synced course offline and open its files.

Sync only reads from ILIAS. It doesn't work offline.

## Offline

Without a connection (train, plane, ILIAS down), ilias-tui still works with what it has
seen before:

- **Starting:** if ILIAS can't be reached, it goes offline after a few seconds (your cached
  courses show meanwhile). `--offline` (or
  `ILIAS_OFFLINE=1`) starts offline on purpose. The status bar shows an `OFFLINE` badge.
- **Pages** you opened before, and every course and folder of a synced course, show from
  the cache as they were then, and are not refreshed. Other pages say "Not available
  offline". This covers tabs too: switching tabs online also shows the cached copy first.
- **Deep search** (`s`) searches the cached pages below the page on screen: a synced
  course completely, otherwise the folders you opened before.
- **Saved files** (`✓ saved`) open as usual; other files can't be downloaded.
- **Nothing is sent** to ILIAS, not even a session check. Favourites can't be changed and
  nothing can be handed in.
- **`L` goes online:** it checks the saved session (and only opens Chrome if that has
  expired). Then the page on screen reloads.

Cached pages are kept for 30 days, and only if the cache is on (`ILIAS_CACHE` not `0`).

## How long does a login last?

There are two sessions:

1. **The ILIAS session** (`PHPSESSID`). This one is short.
2. **The Keycloak SSO session** (`KEYCLOAK_IDENTITY`). This one usually lasts much longer.

When the ILIAS session expires, ilias-tui re-runs the OIDC redirect using the saved Keycloak
cookies and gets a new ILIAS session without any interaction. Only when the Keycloak session
has expired too do you see `session expired: press L to log in again`.

See [docs/architecture.md](docs/architecture.md) for the details.

## Speed

ILIAS itself needs about 0.5–1 s to render a page; ilias-tui keeps everything else out of
the way:

- **Instant revisits.** Every page you open is cached on disk. When you open it again, the
  cached copy shows immediately while the current version loads in the background
  ("Refreshing…" in the status bar). It is swapped in only if something changed.
- **Compressed transfers.** Pages come gzip/brotli-compressed, about 7× smaller.
- **Instant start.** My courses shows from the cache right away; the session check runs in
  the background, and the course list it reads is the one the refresh shows, so a start
  costs two requests (course list and dashboard). The To-Do remembers each exercise's
  course, so exercise pages are only read for new exercises.
- **Parallel deep search.** `s` fetches up to 6 folders at once, so whole courses (or all
  of them) are searched several times faster.

The cache lives in the system cache folder and cleans itself up after 30 days; see
[Security](#security). `ILIAS_CACHE=0` turns it off, and `ilias-tui logout` deletes it.

## Security

The same protections apply on macOS, Linux and Windows unless a point says otherwise.

**Your password never touches ilias-tui.** You log in yourself, in a real Chrome window,
on the normal HSLU / Switch edu-ID pages, 2FA included. Chrome runs with a fresh,
temporary profile that is deleted afterwards, so it doesn't see your normal browser data.

**Only the cookies it needs are kept.**

- After the login, ilias-tui keeps the cookies of ILIAS and of its login server (HSLU's
  Keycloak), nothing else.
- The Switch edu-ID cookies are dropped, because they could work for other edu-ID services.
  (A session saved by an older version may still contain them. Run `ilias-tui login` once
  to replace it.)

**The session file is as good as your login.** Anyone who gets a copy can use ILIAS as you
until the sessions expire. So:

- It's stored in your data folder, which only your user can open:
  - macOS: `~/Library/Application Support/ch.ilias-tui.ilias-tui/`, mode `0700`
  - Linux: `~/.local/share/ilias-tui/`, mode `0700`
  - Windows: `%APPDATA%\ilias-tui\ilias-tui\data\`, inside your user profile, which
    Windows already keeps to your account (ilias-tui sets no extra permissions there)
- The session file itself is mode `0600` (macOS, Linux). It is **not encrypted** (yet; see
  [docs/roadmap.md](docs/roadmap.md#security)), so treat it like a password. Don't commit,
  sync or share it, and remember that backups (e.g. Time Machine, Déjà Dup) include it.

**`ilias-tui logout` logs out on the server too.**

- It ends your ILIAS session through ILIAS's own logout, then deletes the local session
  and the page cache.
- It also checks whether HSLU's Keycloak login is still active. Keycloak can only be logged
  out through a confirmation page in the browser, so if it is still active, `logout` tells
  you and you can log out of ILIAS / Switch edu-ID in your browser.
- If you think the session file leaked: run `ilias-tui logout`. If in doubt, also change
  your edu-ID password.

**What else is stored locally:**

- the **page cache**: course pages you've opened, for instant revisits.
  - It lives in the system cache folder, `~/Library/Caches/ch.ilias-tui.ilias-tui/` on
    macOS, `~/.cache/ilias-tui/` on Linux (mode `0700`) or `%LOCALAPPDATA%\ilias-tui\ilias-tui\cache\`
    on Windows. Backup tools usually skip that
    folder.
  - Entries older than 30 days are deleted automatically.
  - `ILIAS_CACHE=0` turns the cache off.
- the **saved-files list**: which ILIAS file was saved to which path (in the private data
  folder).
- the **what's-new list** (`<host>.seen.json`, also in the private data folder): the ref ids
  of items a sync found new and the folders they are in, until you open those pages.

Downloaded files go to `~/Downloads/ILIAS` with normal permissions, like any browser
download.

**What it sends, and where:**

- Only to your ILIAS server (`ILIAS_URL`), over HTTPS with certificate checks. During the
  login, Chrome also talks to Keycloak and edu-ID. There is no telemetry and no third-party
  service.
- **Almost read-only.** ilias-tui reads with `GET` requests. The one exception that only
  reads is ILIAS's "Show more" button, which asks for the rest of a long list with a `POST`
  of the items already shown; ilias-tui sends it only to your ILIAS and only with that
  button's command (`renderBlockAsynch`). These change something on ILIAS:
  - `ilias-tui logout` ends your session.
  - `H` hands in a file for an assignment, and `X` deletes one you handed in. These are the
    only times it sends a `POST` that changes something. Both name the file and the assignment and ask first, check
    that ILIAS' form belongs to that assignment on your ILIAS, and do nothing with
    `ILIAS_DRY_RUN=1`. Your lecturer can see what you hand in. A deleted file can't be
    restored, and after the deadline ILIAS lets you delete nothing.
  - `u` uploads files into a course or folder. It names the place and the files with their
    sizes and asks first, and only uses that course's or folder's own upload form from a
    fresh copy of its page (checked to be for that very course or folder, on your ILIAS).
    Files bigger than ILIAS takes there are not sent. Everyone who can open the place sees
    the files. Nothing is sent with `ILIAS_DRY_RUN=1`.
  - `n` creates a folder. It names the folder and the place and asks first, and only follows
    that place's own "Add New Item → Folder" link and posts the form ILIAS shows for it
    (checked to create a folder in that very place, on your ILIAS). Nothing is sent with
    `ILIAS_DRY_RUN=1`.
  - `D` moves an item into the ILIAS trash. It names the item, its kind and the place and
    asks first, follows only the Delete in that item's own menu, and posts ILIAS' question
    only if it is for that item in that place, on your ILIAS. If ILIAS warns that the item
    would be deleted for good (no trash), nothing is sent. Nothing is sent with
    `ILIAS_DRY_RUN=1`, and the trash is never emptied.
  - `n` in a forum and `R` in a thread post to the forum. They show the subject, the forum
    and the start of the text and ask first, and only use that forum's own "New Thread"
    form or that post's own Reply form (checked to be for that forum, thread and post, on
    your ILIAS). Everyone who can read the forum sees the post. Nothing is sent with
    `ILIAS_DRY_RUN=1`.
  - `f` adds or removes a favourite. It asks first, only sends the favourites link from
    that item's own ILIAS menu (checked to be for that item, on your ILIAS), and does
    nothing with `ILIAS_DRY_RUN=1`.

  All of them live in `src/ilias/write.rs` and the files next to it (`src/ilias/write/`). Every other request passes a **write guard** that
  refuses anything that looks like a write (an ILIAS one-time `rtoken`, or a known write
  command such as `addToDesk`, `markPostRead` or `join`), so a bug elsewhere can't turn a
  link into a change. Tests check this against a fake ILIAS.

  Other features that would write are deliberately not built; see the rules in
  [docs/roadmap.md](docs/roadmap.md#rules-for-features-that-write).

**Opening things safely:**

- **Files that can run code are never opened.** Anyone who can upload to a shared folder or
  forum could place a `notes.command`, `setup.sh`, `.desktop`, `.app`, `.jar`, `.AppImage`,
  `.exe`, `.bat`, `.lnk`, …. For these, `Enter` only shows the file in your file manager
  (Finder, Files/Dolphin, Explorer) and says why. All other files open with their default app.
- **Downloads are never executable.** On macOS they also get the same quarantine mark
  browsers set, so Gatekeeper checks them before anything inside can run. On Windows they
  get the browsers' "Mark of the Web" (`Zone.Identifier`), so SmartScreen and Office's
  protected view treat them as downloaded. Linux has no equivalent mark; there, downloads
  simply aren't executable.
- **No shell in between.** Files and links are handed to `open` (macOS), `xdg-open`
  (Linux) or `rundll32 url.dll,FileProtocolHandler` (Windows) as one argument, never
  through `cmd`, which would read the `&` in ILIAS links as a command separator.
- **Only web links open in the browser, and other sites only after asking.** Before handing
  a link to `open` / `xdg-open`, ilias-tui checks that it is an `http(s)` link, never
  `file:` or app-specific URL schemes. Links to your ILIAS server open right away; a link
  in page text to any other site (a Zoom link, say) names the address and asks first.
  Links that would change something on ILIAS (the write guard's list) are not offered.
- File names from ILIAS are sanitised before saving, so a crafted name can't write outside
  the download folder, and every name is valid on Windows too (no `<>:"|?*`, no trailing
  dots, no device names like `CON`).

**AI-written code.** The code was written with an AI assistant and hasn't had a thorough
human security review (see [Disclaimer](#disclaimer)). The parts that matter most are
`src/auth.rs` (login), `src/session.rs` (session storage), `src/open.rs` (opening files and
links) and `src/ilias/mod.rs` (requests, downloads, logout). Read them before you trust the
tool with your account.

## Troubleshooting

| Problem | Fix |
| ------- | --- |
| `could not start Chrome/Chromium` | Install Google Chrome, or set `CHROME=/path/to/chrome`. |
| Can't select text with the mouse | Hold Option (iTerm2, Terminal.app) or Shift while dragging, or set `ILIAS_MOUSE=0`. |
| Icons show up as `?` or boxes | Your terminal font has no Nerd Font glyphs. Install one and select it in your terminal, or set `ILIAS_NERD_FONT=0`. |
| A list is empty that shouldn't be | Check with `ilias-tui show goto.php/fold/<id>`. If that's empty too, ILIAS changed its HTML. Run `ilias-tui dump goto.php/fold/<id> > page.html` and compare it with the selectors in `src/ilias/parse/`. |
| Want to log out / reset | Run `ilias-tui logout`. It removes the session and the page cache. The session is stored in `~/Library/Application Support/ch.ilias-tui.ilias-tui/` (macOS), `~/.local/share/ilias-tui/` (Linux) or `%APPDATA%\ilias-tui\ilias-tui\data\` (Windows), readable only by you. |
| `ILIAS returned a page instead of the file` | You don't have access, or the object isn't a plain file. Press `o` to open it in the browser. |
| Login window doesn't close | Make sure you end up on an ILIAS page (not an error page). The login times out after 5 minutes. |

## Development

```bash
cargo test
```

```bash
cargo clippy --all-targets
```

```bash
cargo fmt
```

CI (GitHub Actions) runs the tests and clippy on macOS, Linux and Windows, and `cargo audit`.

The parser tests run against real, public HSLU pages stored in `tests/fixtures/`.
[docs/architecture.md](docs/architecture.md) explains the code layout, the component
architecture and how to add a feature.
[docs/roadmap.md](docs/roadmap.md) lists the features that are missing, and the rules for
anything that would change data on ILIAS.

## Versions

ilias-tui uses [semantic versioning](https://semver.org). The version is in `Cargo.toml`;
`ilias-tui --version`, the top bar and the help (`?`) show it. Before 1.0, the middle number
goes up for new features and changes in behaviour (0.2.0 → 0.3.0), the last one for fixes
only (0.2.0 → 0.2.1). [CHANGELOG.md](CHANGELOG.md) lists what changed in each version.

To release a version:

1. Add what changed under "Unreleased" in `CHANGELOG.md` as you go.
2. Pick the new version, rename "Unreleased" to it with the date, and add a new empty
   "Unreleased" above.
3. Set the same version in `Cargo.toml` and run `cargo build` (it updates `Cargo.lock`).
   A test fails while the changelog has no section for the version in `Cargo.toml`.
4. Commit, then tag the commit, e.g. `git tag v0.3.0`, and push the tag.

## Disclaimer

**AI-generated code.** This project's code, tests and documentation were written with the
help of an AI assistant (Claude by Anthropic), with a human directing and testing it. It
has been tried against a real ILIAS installation, but it hasn't had a thorough human code
review. Read the code before you rely on it, especially the login and session handling
(`src/auth.rs`, `src/session.rs`), and use it at your own risk.

This is an unofficial personal tool. It reads the same pages your browser does, with your
own account. Keep the request rate reasonable and follow HSLU's IT usage rules.

## License

[MIT](LICENSE). You may use, change and share it freely. It comes **as is, without any
warranty**, and the authors are not liable for any damage or consequences of using it
(see the licence text).
