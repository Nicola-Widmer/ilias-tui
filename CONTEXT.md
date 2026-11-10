# Context

The words this project uses for ILIAS and for its own parts. Code, docs and reviews use
them as defined here.

## ILIAS

**Course list**
Your courses and groups, grouped by year, as ILIAS lists them on "Meine Kurse und Gruppen"
(`ilmembershipoverviewgui`). ilias-tui shows the newest year first.
_Avoid_: memberships (only in code that talks to that ILIAS page).

**Favourites**
The objects you marked on ILIAS ("Favoriten"), listed on the ILIAS dashboard. Adding or
removing one changes your ILIAS account.

**To-Do**
Assignments you still have to hand in, with their deadline and course. ILIAS lists them in
the dashboard's "To-Do" block; it only covers exercise assignments.
_Avoid_: tasks, deadlines (a deadline is one property of a To-Do entry).

**Dashboard**
The ILIAS page (`ilDashboardGUI`) that ilias-tui reads favourites and To-Do from. Not shown
as a page of its own.

## ilias-tui

**My courses**
The start page: favourites first, then the course list, with the To-Do beside them. Always
built complete by one module; it loads like any other page (cached copy first, then a
refresh).

**Page**
What ilias-tui shows for one ILIAS object (or My courses): its tabs and blocks, as parsed
from ILIAS.

**Out of date**
A page that something (e.g. a favourites change) has made wrong. It is reloaded the next
time it is on screen, not while it is hidden under other pages.

**Session check**
Checking at startup that the saved ILIAS session still works, renewing it silently if
possible. In the TUI it runs in the background while My courses shows from the cache. It
reads the course list to check, and the next My courses uses that page instead of asking
again.

**Reference id**
ILIAS' number for an object in the repository (`ref_id`, as in `goto.php/crs/123`). Only
these have permalinks and favourites. Assignments and forum threads have numbers of their
own, and ilias-tui gives things it only knows by a link (attachments, text links) an id made
from that link; in code these are the variants of `Id`.

**Write**
A request that changes something on ILIAS: your account, your favourites, read marks,
memberships, notifications or the session (logout). ilias-tui sends writes only from
the `ilias/write` module (`write.rs` and `write/`): favourites changes, handing in a file
and deleting a handed-in one, uploads, new folders, moving items to the **trash**, forum
posts (each after asking), and the logout
(which you ask for).
_Avoid_: action, command (ILIAS's `cmd=` parameter is a command; only some commands are
writes).

**Rights**
What ILIAS lets you change on a page besides reading it: upload files, create items ("Add
New Item"), delete items. Read from the page's own menus and forms for that page (its
reference id), never guessed; students usually have none. They decide which editing keys
work.
_Avoid_: permissions (ILIAS' own tab for setting them), capabilities.

**Reply** (in a forum thread)
A post answering another post, through that post's Reply form. `R` replies to the topmost
post on screen whose header is visible (marked `↩ R reply`); threads scroll, they have no
selection.

**Trash**
Where ILIAS keeps deleted items until someone with the rights restores them or empties it.
ilias-tui's `D` only moves items there, never empties it, and refuses where ILIAS has no
trash.

**Write guard**
The check every page request passes: a request that looks like a write (it carries an
`rtoken`, or its command is a known write such as `addToDesk`) is refused unless it comes
through the write path. A parser bug can therefore never turn a link into a write.

**Link** (in page text)
A web link in a page's text, such as the Zoom link on a course page. ilias-tui lists it as an
entry below its paragraph. ILIAS links open in the browser at once, others after asking.
_Avoid_: weblink (that is the ILIAS object `webr`).

**Permalink**
An object's lasting address on ILIAS 10, `goto.php/<kind>/<ref_id>` (what ILIAS shows as
"Permanent Link"). `y` copies it; things without a reference id (assignments, threads,
links) copy their own link instead.

**Show more list**
A list ILIAS cuts short after a container's block limit, with a "Show more" button
(`ilContainerShowMore`) that loads the next batch. ilias-tui loads every batch, so a page
always has all its items; if it can't, the page says so.
_Avoid_: pagination (ILIAS pages forums differently).

**New** (what's new since your last visit)
An item of a course or folder that appeared, or whose properties (size, date, …) changed,
since the cached copy of that page. Countdowns ("5 Hours, 8 Minutes") don't count. Marked
`● new` while ilias-tui runs. Forums and threads are kept apart as **new posts**: they
count only when one of their numbers went up (posts, unread, the last post's date), so
reading posts is no news. Every new item belongs to one **group** (files, folders, forum
posts, other); the settings switch each group's marks and counts on or off.

**Unseen**
New items a sync found on a page (a course or folder) you haven't opened since. Kept per
ILIAS host in `<host>.seen.json`, with the folders on the way down from the course. The
course and every folder on the way count them (`● 3 new`), so the count leads to the new
files. Opening a page makes the items on that page seen; folders below keep theirs.

**Deep search**
Collecting everything below a place (`s`). On My courses it covers the course list only,
not favourites or To-Do.

**Assignment**:
One task of an exercise, with a deadline, instructions, attached files and your submission. It has its own number, which is not a reference id.

**Submission**:
The files you handed in for an assignment, listed on its Submission tab; ilias-tui shows that
tab as the assignment's "Handed-in files" entry. Until the deadline you can add files (`H`)
and delete them (`X`).
_Avoid_: upload, delivery (ILIAS' own word in its code: `delivered`).

**Submissions page**:
An assignment's "Submissions and Grades" (`ilExerciseManagementGUI`, `cmd=members`), seen
only by those who grade: every participant with what they handed in, one assignment at a
time (ilias-tui shows the assignments as tabs). Each participant is an entry of kind
`Participant`; `a` saves all of their submissions.
_Avoid_: members page (ILIAS' command name), deliveries.

**Two-way sync**:
A sync that also sends your side to ILIAS: new and edited files, and deletions you choose.
Off by default (settings). It decides each file three-way, from the base, your disk now and
ILIAS now.

**Base**:
What the saved-files list recorded for a file when sync last saved it (or sent it): its
version on ILIAS (size and date in the listing), and the size, date and content hash of
the saved copy. "Edited" means the content differs from the base; "changed on ILIAS" means
the version does.

**Conflict**:
A file sync can't decide about safely: edited by you and changed on ILIAS, or something
unclear (a new file named like an ILIAS file sync didn't save, a copy of a saved file).
Nothing is sent for it and both copies stay.
_Avoid_: clash, collision.

**Proposal**:
A deletion two-way sync suggests because you deleted a file it saved and ILIAS still has
it as saved: moving that file to the ILIAS trash, unticked until you tick it.

**Review**:
What a two-way sync shows before it sends anything: grouped, what `Enter` sends (uploads,
replaced files), the proposals to tick, and what stays as it is (conflicts, files gone on
ILIAS, files ILIAS doesn't let you upload). In a dry run it is a plain report.
_Avoid_: preview, diff.

**Journal**:
The log of every write two-way sync sent or tried (time, action, file, ILIAS object,
result), `<host>.journal.jsonl` next to the session. One sync's writes share a `run`.

**Upload**:
Putting files into a course, group or folder (`u`), as its "Upload Files" form does. Everyone
who can open the place sees them. Only where the page's rights include it.
_Avoid_: hand in (that is a Submission).
