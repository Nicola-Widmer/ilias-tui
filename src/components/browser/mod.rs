//! The main view: "My courses" and, below it, a stack of opened containers. Each level shows
//! the ILIAS page like the web UI does: tabs, page text, section headings and items.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::widgets::ListState;

use super::Component;
use crate::action::Action;
use crate::ilias::{Id, Item, Kind, Todo};

mod draw;
mod level;
mod rows;

use level::{Level, Row};
use rows::*;

#[derive(Default)]
pub struct Browser {
    stack: Vec<Level>,
    /// What is being loaded: ref id (`None` = "My courses") and whether it is a deep search.
    /// Responses for anything else are stale (the user navigated away) and get dropped.
    pending: Option<(Option<Id>, bool)>,
    /// Ref ids of files saved locally; Enter opens these without downloading.
    saved: HashSet<u64>,
    /// Ref ids of items that are new since your last visit (marked for this run).
    new: HashSet<u64>,
    /// New items per course you haven't opened since (from `Seen`).
    new_counts: HashMap<u64, usize>,
    /// Clickable areas from the last draw.
    hits: Vec<(Rect, Hit)>,
    /// What was clicked last and when, to detect double-clicks.
    last_click: Option<(Hit, Instant)>,
    /// Selection in the To-Do panel ("My courses" only).
    todo: ListState,
    /// Keys go to the To-Do panel instead of the list.
    todo_focus: bool,
    /// Courses and folders marked with Space for the next sync.
    marked: Vec<Item>,
    /// How sync decides (from the settings), for the question before `A`.
    sync_rules: crate::sync::Rules,
}

/// What `S` can sync: places that hold files.
fn syncable(item: &Item) -> bool {
    matches!(
        item.kind,
        Kind::Course | Kind::Group | Kind::Folder | Kind::Session
    )
}

/// What a mouse click at some screen area means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hit {
    /// An item row; the position within `Level::visible`.
    Item(usize),
    Tab(usize),
    /// A breadcrumb; the index into `Browser::stack`.
    Crumb(usize),
    /// An entry of the To-Do panel.
    Todo(usize),
}

const DOUBLE_CLICK: Duration = Duration::from_millis(400);

impl Browser {
    pub fn new(saved: HashSet<u64>, sync_rules: crate::sync::Rules) -> Self {
        Self {
            saved,
            sync_rules,
            ..Self::default()
        }
    }

    fn current(&self) -> Option<&Level> {
        self.stack.last()
    }

    /// The filter on the list on screen.
    fn query(&self) -> &str {
        self.current().map_or("", |l| l.query.as_str())
    }

    /// `A`: asks before syncing every course in My courses.
    fn sync_all(&self) -> Action {
        // A course can be listed twice (Favourites and its semester): count and sync it once.
        let mut seen = HashSet::new();
        let courses: Vec<Item> = self
            .stack
            .iter()
            .find(|l| l.item.is_none() && !l.deep)
            .map(|l| {
                l.items
                    .iter()
                    .filter(|i| syncable(i) && seen.insert(i.id))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if courses.is_empty() {
            return Action::Error("No courses to sync: My courses is empty or not loaded".into());
        }
        let n = courses.len();
        Action::Ask {
            question: format!(
                "Sync all {n} course{} in My courses? {}",
                if n == 1 { "" } else { "s" },
                self.sync_rules.describe()
            ),
            then: Box::new(Action::Sync(courses)),
        }
    }

    fn open(&self, item: &Item) -> Action {
        if item.is_container() {
            Action::Load {
                item: Some(item.clone()),
                url: None,
                refresh: false,
                deep: false,
            }
        } else if item.kind == Kind::File {
            Action::Download {
                item: item.clone(),
                open: true,
            }
        } else {
            Action::OpenInBrowser(item.clone())
        }
    }

    /// The To-Do entries of the page on screen.
    fn todo_items(&self) -> &[Todo] {
        self.current().map_or(&[], |l| l.page.todo.as_slice())
    }

    fn click(&mut self, hit: Hit) -> Option<Action> {
        let double = self
            .last_click
            .is_some_and(|(h, at)| h == hit && at.elapsed() < DOUBLE_CLICK);
        self.last_click = (!double).then(|| (hit, Instant::now()));
        match hit {
            Hit::Item(position) => {
                self.todo_focus = false;
                self.stack.last_mut()?.cursor = Some(position);
                double
                    .then(|| self.current()?.selected().map(|i| self.open(i)))
                    .flatten()
            }
            Hit::Todo(index) => {
                self.todo_focus = true;
                self.todo.select(Some(index));
                double
                    .then(|| {
                        self.todo_items()
                            .get(index)
                            .map(|t| self.open(&t.assignment))
                    })
                    .flatten()
            }
            Hit::Tab(index) => {
                let level = self.current()?;
                let tab = level.page.tabs.get(index).filter(|t| !t.active)?;
                Some(Action::Load {
                    item: level.item.clone(),
                    url: Some(tab.url.clone()),
                    refresh: false,
                    deep: false,
                })
            }
            Hit::Crumb(index) if index + 1 < self.stack.len() => {
                self.stack.truncate(index + 1);
                self.pending = None;
                Some(Action::Filter(self.query().to_string()))
            }
            Hit::Crumb(_) => None,
        }
    }

    /// Loads the next (`+1`) or previous (`-1`) tab of the current page.
    fn switch_tab(&self, step: isize) -> Option<Action> {
        let level = self.current()?;
        let tabs = &level.page.tabs;
        let active = tabs.iter().position(|t| t.active).unwrap_or(0) as isize;
        let next = (active + step).rem_euclid(tabs.len().max(1) as isize) as usize;
        Some(Action::Load {
            item: level.item.clone(),
            url: Some(tabs.get(next)?.url.clone()),
            refresh: false,
            deep: false,
        })
    }
}

impl Browser {
    /// Keys while the To-Do panel has the focus; `None` hands the key back to the list.
    fn todo_key(&mut self, key: KeyEvent) -> Option<Option<Action>> {
        let last = self.todo_items().len().checked_sub(1)?;
        let selected = self.todo.selected().unwrap_or(0).min(last);
        let to = match key.code {
            KeyCode::Down | KeyCode::Char('j') => (selected + 1).min(last),
            KeyCode::Up | KeyCode::Char('k') => selected.saturating_sub(1),
            KeyCode::Home | KeyCode::Char('g') => 0,
            KeyCode::End | KeyCode::Char('G') => last,
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l' | 'o') => {
                return Some(
                    self.todo_items()
                        .get(selected)
                        .map(|t| self.open(&t.assignment)),
                );
            }
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => {
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
}

impl Component for Browser {
    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if self.todo_focus
            && let Some(action) = self.todo_key(key)
        {
            return action;
        }
        // On a page without tabs (My courses), Tab moves to the To-Do panel.
        if matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
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
        if key.code == KeyCode::Char('A') {
            return Some(self.sync_all());
        }
        // From inside an assignment's Submission, `H` and `X` are about the assignment.
        let assignment = self
            .stack
            .iter()
            .rev()
            .filter_map(|l| l.item.as_ref())
            .find(|i| i.kind != Kind::Submission)
            .cloned();
        let level = self.stack.last_mut()?;
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => level.move_cursor(|c, last| (c + 1).min(last)),
            KeyCode::Up | KeyCode::Char('k') => level.move_cursor(|c, _| c.saturating_sub(1)),
            KeyCode::Home | KeyCode::Char('g') => level.move_cursor(|_, _| 0),
            KeyCode::End | KeyCode::Char('G') => level.move_cursor(|_, last| last),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                return self.current()?.selected().map(|i| self.open(i));
            }
            KeyCode::Left | KeyCode::Backspace | KeyCode::Esc | KeyCode::Char('h') => {
                if self.stack.len() > 1 {
                    self.stack.pop();
                }
                self.pending = None;
                // The list comes back with the filter you left it with; tell the search bar.
                return Some(Action::Filter(self.query().to_string()));
            }
            KeyCode::Tab => return self.switch_tab(1),
            KeyCode::BackTab => return self.switch_tab(-1),
            KeyCode::Char('d') => {
                let item = level.selected().filter(|i| i.kind == Kind::File)?;
                return Some(Action::Download {
                    item: item.clone(),
                    open: false,
                });
            }
            KeyCode::Char('o') => {
                let item = level.selected().or(level.item.as_ref())?;
                return Some(Action::OpenInBrowser(item.clone()));
            }
            KeyCode::Char('y') => {
                let item = level.selected().or(level.item.as_ref())?;
                return Some(Action::CopyLink(item.clone()));
            }
            KeyCode::Char('r') => {
                let url = level
                    .page
                    .tabs
                    .iter()
                    .find(|t| t.active)
                    .map(|t| t.url.clone());
                return Some(Action::Load {
                    item: level.item.clone(),
                    url,
                    refresh: true,
                    deep: level.deep,
                });
            }
            KeyCode::Char('s') if !level.deep => {
                return Some(Action::Load {
                    item: level.item.clone(),
                    url: None,
                    refresh: false,
                    deep: true,
                });
            }
            KeyCode::Char('f') => {
                let item = level.selected()?.clone();
                if item.favorite_toggle.is_none() {
                    return Some(Action::Error(format!(
                        "ILIAS offers no favourites option for {}",
                        item.title
                    )));
                }
                let question = if item.favorite {
                    format!("Remove “{}” from your ILIAS favourites?", item.title)
                } else {
                    format!("Add “{}” to your ILIAS favourites?", item.title)
                };
                return Some(Action::Ask {
                    question,
                    then: Box::new(Action::ToggleFavorite(item)),
                });
            }
            KeyCode::Char(' ') => {
                let item = level.selected()?.clone();
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
            }
            KeyCode::Char('S') => {
                // The marked ones, else the selected course/folder, else the one you're in.
                let roots = if self.marked.is_empty() {
                    level
                        .selected()
                        .filter(|i| syncable(i))
                        .or(level.item.as_ref().filter(|i| syncable(i)))
                        .cloned()
                        .into_iter()
                        .collect()
                } else {
                    std::mem::take(&mut self.marked)
                };
                if roots.is_empty() {
                    return Some(Action::Error(
                        "Select a course or folder to sync (Space marks several)".into(),
                    ));
                }
                return Some(Action::Sync(roots));
            }
            KeyCode::Char('H') => {
                // Assignments that take files have a tab for them; other types don't.
                let submission = level.page.tabs.iter().find(|t| {
                    t.url
                        .to_ascii_lowercase()
                        .contains("cmdclass=ilexsubmissionfilegui")
                });
                return Some(match (assignment, submission) {
                    (Some(assignment), Some(tab)) => Action::AskHandIn {
                        assignment,
                        url: tab.url.clone(),
                    },
                    _ => Action::Error("Open an assignment that takes files to hand in".into()),
                });
            }
            KeyCode::Char('X') => {
                // Handed-in files link to their download with `delivered=<id>`.
                let file = level.selected().filter(|i| i.url.contains("&delivered="));
                return Some(match (assignment, file.cloned()) {
                    (Some(assignment), Some(file)) => Action::Ask {
                        question: format!(
                            "Delete {} from your submission for {}? It can't be restored.",
                            file.title, assignment.title
                        ),
                        then: Box::new(Action::DeleteSubmission { assignment, file }),
                    },
                    _ => Action::Error(
                        "Select a handed-in file (in an assignment's Submission) to delete it"
                            .into(),
                    ),
                });
            }
            KeyCode::Char('L') => return Some(Action::Login),
            _ => return None,
        }
        None
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent) -> Option<Action> {
        if self.todo_focus
            && matches!(
                mouse.kind,
                MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
            )
        {
            let code = match mouse.kind {
                MouseEventKind::ScrollDown => KeyCode::Down,
                _ => KeyCode::Up,
            };
            return self.todo_key(KeyEvent::from(code)).flatten();
        }
        let level = self.stack.last_mut()?;
        match mouse.kind {
            MouseEventKind::ScrollDown => level.move_cursor(|c, last| (c + 1).min(last)),
            MouseEventKind::ScrollUp => level.move_cursor(|c, _| c.saturating_sub(1)),
            MouseEventKind::Down(MouseButton::Left) => {
                let at = Position::new(mouse.column, mouse.row);
                let (_, hit) = *self.hits.iter().find(|(rect, _)| rect.contains(at))?;
                return self.click(hit);
            }
            _ => {}
        }
        None
    }

    fn update(&mut self, action: &Action) -> Option<Action> {
        match action {
            Action::SetConfig(config) => self.sync_rules = crate::sync::Rules::new(config),
            Action::Load { item, deep, .. } => {
                self.pending = Some((item.as_ref().map(|i| i.id), *deep));
            }
            Action::Loaded {
                item,
                page,
                refresh,
                deep,
            } => {
                // Only the page asked for last counts; the rest is stale.
                if self.pending != Some((item.as_ref().map(|i| i.id), *deep)) {
                    return None;
                }
                self.pending = None;
                match self.stack.last_mut() {
                    Some(level) if *refresh => level.refresh(page),
                    // Another tab of the page on screen (cached or fetched) takes its place.
                    Some(level)
                        if level.deep == *deep
                            && level.item.as_ref().map(|i| i.id) == item.as_ref().map(|i| i.id) =>
                    {
                        *level = Level::new(item.clone(), *deep, page.clone());
                        return Some(Action::Filter(String::new()));
                    }
                    _ => {
                        self.todo_focus = false;
                        self.stack
                            .push(Level::new(item.clone(), *deep, page.clone()));
                        // A new list starts unfiltered; tell the search bar.
                        return Some(Action::Filter(String::new()));
                    }
                }
            }
            Action::Filter(query) => {
                let level = self.stack.last_mut()?;
                // Coming back with the same filter keeps the selection where it was.
                if level.query != *query {
                    level.filter(query);
                }
                // Going back (Esc, a breadcrumb) sends the level's filter: the moment a page
                // that went out of date while hidden is on screen again.
                if std::mem::take(&mut level.out_of_date) {
                    return Some(level.reload());
                }
            }
            Action::Downloaded { item, .. } => {
                self.saved.insert(item.key());
            }
            Action::Synced { saved, .. } => self.saved.extend(saved),
            Action::NewItems(items) => self.new.extend(items),
            Action::NewCounts(counts) => self.new_counts.clone_from(counts),
            Action::Error(_) => self.pending = None,
            // The ★ marks (and My courses' favourites) are now wrong: reload the page on
            // screen, and mark the hidden ones that show the item, or the favourites.
            Action::FavoriteToggled {
                item,
                dry_run: false,
            } => {
                let (top, hidden) = self.stack.split_last_mut()?;
                for level in hidden {
                    level.out_of_date |=
                        level.item.is_none() || level.items.iter().any(|i| i.id == item.id);
                }
                return Some(top.reload());
            }
            // The submission shows on the page on screen and on the ones below it, and the
            // To-Do may change.
            Action::HandedIn { file: Some(_), .. }
            | Action::SubmissionDeleted { deleted: true, .. } => {
                let (top, hidden) = self.stack.split_last_mut()?;
                for level in hidden {
                    level.out_of_date = true;
                }
                return Some(top.reload());
            }
            // Reload whatever is on screen with the fresh session (or from the cache, offline);
            // the levels below reload once they are on screen again.
            Action::LoggedIn | Action::Offline => {
                if let Some((_, hidden)) = self.stack.split_last_mut() {
                    for level in hidden {
                        level.out_of_date = true;
                    }
                }
                let level = self.current();
                return Some(Action::Load {
                    item: level.and_then(|l| l.item.clone()),
                    url: None,
                    refresh: level.is_some(),
                    deep: level.is_some_and(|l| l.deep),
                });
            }
            _ => {}
        }
        None
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        self.render(frame, area);
    }
}

#[cfg(test)]
mod tests;
