use std::path::PathBuf;

use crate::ilias::{Item, Page};

/// Everything that can happen in the app. Components emit actions from key events,
/// the [`App`](crate::app::App) runs the side effects, and every action is then broadcast
/// to all components via [`Component::update`](crate::components::Component::update).
#[derive(Debug, Clone)]
pub enum Action {
    Quit,
    ToggleHelp,
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
    OpenInBrowser(Item),
    /// Run the interactive browser login.
    Login,
    LoggedIn,
    /// Intermediate status of a running job.
    Progress(String),
    Error(String),
}
