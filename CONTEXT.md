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
`ilias/write.rs`: favourites changes, handing in a file and deleting a handed-in one (after
asking), and the logout
(which you ask for).
_Avoid_: action, command (ILIAS's `cmd=` parameter is a command; only some commands are
writes).

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
reading posts is no news.

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
