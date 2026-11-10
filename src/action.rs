use std::path::PathBuf;

use crate::config::Config;
use crate::ilias::{Item, Page};

/// A forum post being written.
#[derive(Debug, Clone)]
pub enum Draft {
    /// A new thread in `forum`, which is at `place`.
    Thread { forum: Item, place: String },
    /// A reply in `thread` to its post `post`, written by `author`.
    Reply {
        thread: Item,
        post: u64,
        author: String,
    },
}

/// Everything that can happen in the app. Components emit actions from key events,
/// the [`App`](crate::app::App) runs the side effects, and every action is then broadcast
/// to all components via [`Component::update`](crate::components::Component::update).
#[derive(Debug, Clone)]
pub enum Action {
    Quit,
    /// A key or click was used up by an overlay; nothing else happens.
    Handled,
    ToggleHelp,
    ToggleSettings,
    /// Save these settings and apply them (changed on the settings page).
    SetConfig(Config),
    /// The folder files are saved in now.
    DownloadDirSet(PathBuf),
    /// Fetch the page of `item` (`None` means "My courses"). `url` selects another tab of
    /// it. With `deep`, fetch everything below it recursively (the deep search).
    Load {
        item: Option<Item>,
        url: Option<String>,
        refresh: bool,
        deep: bool,
    },
    Loaded {
        item: Option<Item>,
        page: Page,
        refresh: bool,
        deep: bool,
    },
    /// These items are new since your last visit (by ref id): mark them.
    NewItems(Vec<u64>),
    /// How much is new below each course and folder (by key) that you haven't opened since.
    NewCounts(std::collections::HashMap<u64, crate::seen::Counts>),
    /// Show only items whose title or description matches the query (empty = show all).
    Filter(String),
    /// Save a file; with `open`, open it afterwards (reusing an earlier copy if there is one).
    Download {
        item: Item,
        open: bool,
    },
    Downloaded {
        item: Item,
        path: PathBuf,
        open: bool,
    },
    /// Open a local file with its default app.
    OpenFile(PathBuf),
    /// Open the item's link in the web browser: ILIAS links at once, others after asking.
    OpenInBrowser(Item),
    /// Copy the item's permalink to the clipboard.
    CopyLink(Item),
    /// This address is in the clipboard now.
    Copied(String),
    /// Open a web link now: an ILIAS link, or another site after asking.
    OpenLink(String),
    /// Ask which file to hand in for `assignment`; `url` is its submission page.
    AskHandIn {
        assignment: Item,
        url: String,
    },
    HandIn {
        assignment: Item,
        url: String,
        path: PathBuf,
    },
    /// `file` is `None` after a dry run.
    HandedIn {
        assignment: Item,
        file: Option<String>,
    },
    /// Delete `file`, handed in for `assignment`, from the submission.
    DeleteSubmission {
        assignment: Item,
        file: Item,
    },
    /// `deleted` is `false` after a dry run.
    SubmissionDeleted {
        assignment: Item,
        file: Item,
        deleted: bool,
    },
    /// Ask which files to upload into `container` (a course or folder), which is at `place`.
    AskUpload {
        container: Item,
        place: String,
    },
    Upload {
        container: Item,
        paths: Vec<PathBuf>,
    },
    /// `files` (as ILIAS lists them) is `None` after a dry run.
    Uploaded {
        container: Item,
        files: Option<Vec<String>>,
    },
    /// Ask for the name of a new folder in `container`, which is at `place`.
    AskFolderName {
        container: Item,
        place: String,
    },
    NewFolder {
        container: Item,
        name: String,
    },
    /// `created` is `false` after a dry run.
    FolderCreated {
        container: Item,
        name: String,
        created: bool,
    },
    /// Move `item` from `container` into the ILIAS trash.
    Delete {
        container: Item,
        item: Item,
    },
    /// `deleted` is `false` after a dry run.
    Deleted {
        container: Item,
        item: Item,
        deleted: bool,
    },
    /// Write a forum post: in `$EDITOR` if there is one, else in one-line prompts.
    Compose(Draft),
    /// Ask for a line of the post: its title if `title` is `None` (a new thread), else its
    /// text.
    AskPostText {
        draft: Draft,
        title: Option<String>,
    },
    /// `title` is empty for a reply (ILIAS keeps its subject).
    SendPost {
        draft: Draft,
        title: String,
        text: String,
    },
    /// `sent` is `false` after a dry run.
    Posted {
        draft: Draft,
        sent: bool,
    },
    /// Save what these participants handed in (an assignment's submissions page), each into
    /// its own folder; unchanged ones saved before are skipped.
    DownloadSubmissions(Vec<Item>),
    /// `folder` is where they went; `saved` the keys of the participants saved now.
    SubmissionsSaved {
        folder: PathBuf,
        saved: Vec<u64>,
        unchanged: usize,
        failed: Vec<String>,
    },
    /// Ask the user first; `then` only runs on "yes". Everything that changes data on ILIAS
    /// goes through this.
    Ask {
        question: String,
        then: Box<Action>,
    },
    /// The user said no to an [`Action::Ask`].
    Cancelled,
    /// Add the item to the ILIAS favourites, or remove it (whichever its menu offers).
    ToggleFavorite(Item),
    /// `dry_run`: dry run is on (settings or `ILIAS_DRY_RUN`), so nothing was sent.
    FavoriteToggled {
        item: Item,
        dry_run: bool,
    },
    /// Save the files of these courses or folders (see `sync.rs`).
    Sync(Vec<Item>),
    /// A sync finished: what happened (one line), the files worth a look (by section),
    /// the ref ids of all files saved now and (two-way) what it would send to ILIAS.
    Synced {
        summary: String,
        report: Vec<(String, Vec<String>)>,
        saved: Vec<u64>,
        review: Box<crate::sync::Review>,
    },
    /// Two-way sync: send what the review sends (uploads, new folders, replaced files, the
    /// ticked deletions); asks once more first if it is a lot at once.
    AskPush(Vec<crate::sync::Change>),
    /// Move these saved files, gone on ILIAS and untouched, to your computer's trash.
    TrashLocal(Vec<PathBuf>),
    /// How many went to the trash, and the ones kept with why.
    LocalTrashed {
        moved: usize,
        kept: Vec<String>,
    },
    /// Two-way sync: send these changes to ILIAS (you agreed to them).
    Push(Vec<crate::sync::Change>),
    /// What a push sent (one line) and what deserves a look (by section).
    Pushed {
        summary: String,
        report: Vec<(String, Vec<String>)>,
    },
    /// The app runs offline: only cached pages and saved files, nothing is sent.
    Offline,
    /// Run the interactive browser login (offline: first try the saved session).
    Login,
    LoggedIn,
    /// Intermediate status of a running job.
    Progress(String),
    Error(String),
}
