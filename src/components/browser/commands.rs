//! What each [`Command`] does in the browser (keys come in through [`crate::keymap`]).

use super::*;
use crate::ilias::Kind;

impl Browser {
    /// Runs `command`; `None` if it doesn't apply here (the key then goes on).
    pub(super) fn run(&mut self, command: Command) -> Option<Action> {
        if self.todo_focus
            && let Some(action) = self.todo_command(command)
        {
            return action;
        }
        // On a page without tabs (My courses), Tab moves to the To-Do panel.
        if matches!(command, Command::NextTab | Command::PreviousTab)
            && let Some(level) = self.current()
            && level.page.tabs.is_empty()
            && !level.page.todo.is_empty()
        {
            self.todo_focus = true;
            if self.todo.selected().is_none() {
                self.todo.select(Some(0));
            }
            return None;
        }
        // Works from anywhere, also before My courses is shown (it says so).
        if command == Command::SyncAll {
            return Some(self.sync_all());
        }
        let level = self.stack.last_mut()?;
        match command {
            Command::Down => level.move_cursor(|c, last| (c + 1).min(last)),
            Command::Up => level.move_cursor(|c, _| c.saturating_sub(1)),
            Command::First => level.move_cursor(|_, _| 0),
            Command::Last => level.move_cursor(|_, last| last),
            Command::Open => return self.current()?.selected().map(|i| self.open(i)),
            Command::Back => return Some(self.back()),
            Command::NextTab => return self.switch_tab(1),
            Command::PreviousTab => return self.switch_tab(-1),
            Command::Download => {
                let item = level.selected().filter(|i| i.kind == Kind::File)?;
                return Some(Action::Download {
                    item: item.clone(),
                    open: false,
                });
            }
            Command::OpenInBrowser => {
                let item = level.selected().or(level.item.as_ref())?;
                return Some(Action::OpenInBrowser(item.clone()));
            }
            Command::CopyLink => {
                let item = level.selected().or(level.item.as_ref())?;
                return Some(Action::CopyLink(item.clone()));
            }
            Command::Reload => return Some(level.reload()),
            Command::Search if !level.deep => {
                return Some(Action::Load {
                    item: level.item.clone(),
                    url: None,
                    refresh: false,
                    deep: true,
                });
            }
            Command::Favorite => return Some(favorite_question(level.selected()?.clone())),
            Command::MarkForSync => return self.mark_for_sync(),
            Command::Sync => return Some(self.sync_selected()),
            Command::HandIn => return Some(self.hand_in()),
            Command::DeleteSubmission => return Some(self.delete_submission()),
            Command::Upload => return Some(self.upload()),
            Command::Login => return Some(Action::Login),
            // Other components' keys, and search inside search results.
            Command::Search
            | Command::SyncAll
            | Command::Filter
            | Command::Settings
            | Command::Help
            | Command::Quit => return None,
        }
        None
    }

    /// Commands while the To-Do panel has the focus; `None` hands the command back to the list.
    pub(super) fn todo_command(&mut self, command: Command) -> Option<Option<Action>> {
        let last = self.todo_items().len().checked_sub(1)?;
        let selected = self.todo.selected().unwrap_or(0).min(last);
        let to = match command {
            Command::Down => (selected + 1).min(last),
            Command::Up => selected.saturating_sub(1),
            Command::First => 0,
            Command::Last => last,
            Command::Open | Command::OpenInBrowser => {
                let todo = self.todo_items().get(selected);
                return Some(todo.map(|t| self.open(&t.assignment)));
            }
            Command::NextTab | Command::PreviousTab | Command::Back => {
                self.todo_focus = false;
                return Some(None);
            }
            // Anything else (filter, search, …) is about the list.
            _ => {
                self.todo_focus = false;
                return None;
            }
        };
        self.todo.select(Some(to));
        Some(None)
    }

    /// One level up; the list comes back with the filter you left it with, so tell the
    /// search bar.
    fn back(&mut self) -> Action {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
        self.pending = None;
        Action::Filter(self.query().to_string())
    }

    /// Marks the selected course or folder for the next sync, or unmarks it.
    fn mark_for_sync(&mut self) -> Option<Action> {
        let item = self.current()?.selected()?.clone();
        if !syncable(&item) {
            return Some(Action::Error(
                "Only courses, groups and folders can be marked for sync".into(),
            ));
        }
        match self.marked.iter().position(|m| m.id == item.id) {
            Some(i) => {
                self.marked.remove(i);
            }
            None => self.marked.push(item),
        }
        None
    }

    /// The marked ones, else the selected course or folder, else the one you're in.
    fn sync_selected(&mut self) -> Action {
        let roots = if self.marked.is_empty() {
            self.current()
                .and_then(|level| {
                    level
                        .selected()
                        .filter(|i| syncable(i))
                        .or(level.item.as_ref().filter(|i| syncable(i)))
                })
                .cloned()
                .into_iter()
                .collect()
        } else {
            std::mem::take(&mut self.marked)
        };
        if roots.is_empty() {
            return Action::Error("Select a course or folder to sync (Space marks several)".into());
        }
        Action::Sync(roots)
    }

    /// The assignment `H` and `X` are about: the nearest level that isn't its Submission.
    fn assignment(&self) -> Option<Item> {
        self.stack
            .iter()
            .rev()
            .filter_map(|l| l.item.as_ref())
            .find(|i| i.kind != Kind::Submission)
            .cloned()
    }

    fn hand_in(&self) -> Action {
        // Assignments that take files have a tab for them; other types don't.
        let submission = self.current().and_then(|level| {
            level.page.tabs.iter().find(|t| {
                t.url
                    .to_ascii_lowercase()
                    .contains("cmdclass=ilexsubmissionfilegui")
            })
        });
        match (self.assignment(), submission) {
            (Some(assignment), Some(tab)) => Action::AskHandIn {
                assignment,
                url: tab.url.clone(),
            },
            _ => Action::Error("Open an assignment that takes files to hand in".into()),
        }
    }

    fn delete_submission(&self) -> Action {
        // Handed-in files link to their download with `delivered=<id>`.
        let file = self
            .current()
            .and_then(|l| l.selected())
            .filter(|i| i.url.contains("&delivered="));
        match (self.assignment(), file.cloned()) {
            (Some(assignment), Some(file)) => Action::Ask {
                question: format!(
                    "Delete {} from your submission for {}? It can't be restored.",
                    file.title, assignment.title
                ),
                then: Box::new(Action::DeleteSubmission { assignment, file }),
            },
            _ => Action::Error(
                "Select a handed-in file (in an assignment's Submission) to delete it".into(),
            ),
        }
    }
}

impl Browser {
    /// Where the page on screen is, from the course down: `DEMO.101 › Unterlagen`.
    fn place(&self) -> String {
        let titles: Vec<&str> = self
            .stack
            .iter()
            .filter_map(|l| l.item.as_ref())
            .map(|i| i.title.as_str())
            .collect();
        titles.join(&format!(" {} ", crate::theme::chevron()))
    }

    /// `u`: files into the course or folder on screen, if its page has ILIAS' upload form.
    fn upload(&self) -> Action {
        let container = self
            .current()
            .filter(|level| !level.deep && level.page.rights.upload)
            .and_then(|level| level.item.clone());
        match container {
            Some(container) => Action::AskUpload {
                container,
                place: self.place(),
            },
            None => Action::Error("ILIAS doesn't let you upload files here".into()),
        }
    }
}

/// `f`: asks before adding `item` to the favourites or removing it.
fn favorite_question(item: Item) -> Action {
    if item.favorite_toggle.is_none() {
        return Action::Error(format!(
            "ILIAS offers no favourites option for {}",
            item.title
        ));
    }
    let question = if item.favorite {
        format!("Remove “{}” from your ILIAS favourites?", item.title)
    } else {
        format!("Add “{}” to your ILIAS favourites?", item.title)
    };
    Action::Ask {
        question,
        then: Box::new(Action::ToggleFavorite(item)),
    }
}
