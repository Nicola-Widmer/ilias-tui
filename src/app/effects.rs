//! What each action does besides being shown: requests, files, jobs on worker threads.
//! [`App::effect`] routes; every effect is a method of its own below.

use super::*;

/// Whether the components see an action after its effect.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Broadcast {
    Yes,
    /// It was turned into another action, which they will see instead.
    No,
}

impl App {
    /// Why `action` can't run now, if it can't.
    pub(super) fn refusal(&self, action: &Action) -> Option<&'static str> {
        // Offline nothing can be changed on ILIAS; say so before asking the question. (Other
        // sites can still be opened.)
        let sends = match action {
            Action::Ask { then, .. } => !matches!(**then, Action::OpenLink(_)),
            Action::AskHandIn { .. } | Action::AskUpload { .. } | Action::Sync(_) => true,
            _ => false,
        };
        if self.client.is_offline() && sends {
            return Some(
                "Offline: nothing can be sent to or fetched from ILIAS. Press L to go online.",
            );
        }
        let wants_sync = match action {
            Action::Sync(_) => true,
            Action::Ask { then, .. } => matches!(**then, Action::Sync(_)),
            _ => false,
        };
        if wants_sync && self.syncing.load(Ordering::SeqCst) {
            return Some("A sync is already running; start the next one when it has finished.");
        }
        // Offline, a deep search only reads the cache: no limit there.
        let wants_search =
            matches!(action, Action::Load { deep: true, .. }) && !self.client.is_offline();
        if wants_search && self.searching.load(Ordering::SeqCst) {
            return Some("A search is already running; start the next one when it has finished.");
        }
        None
    }

    pub(super) fn effect(&mut self, action: &Action) -> Result<Broadcast> {
        match action {
            Action::Quit => self.should_quit = true,
            Action::Load {
                item,
                url,
                refresh,
                deep,
            } => self.load(item.clone(), url.clone(), *refresh, *deep)?,
            // Opening a file that was saved before: use the local copy.
            Action::Download { item, open: true } => {
                if let Some(path) = self.downloads.get(item.key()).cloned() {
                    self.tx.send(Action::OpenFile(path))?;
                    return Ok(Broadcast::No);
                }
                self.download(item.clone(), true);
            }
            Action::Download { item, open: false } => self.download(item.clone(), false),
            Action::Downloaded { item, path, open } => self.downloaded(item, path, *open)?,
            // Files that could run code are only shown in the file manager.
            Action::OpenFile(path) if open::is_risky(path) => open::reveal(path)?,
            Action::OpenFile(path) => open::external(path.as_os_str())?,
            Action::OpenInBrowser(item) => self.open_in_browser(item)?,
            Action::OpenLink(url) if open::is_web_link(url) => open::external(url.as_ref())?,
            Action::CopyLink(item) => self.copy_link(item)?,
            Action::ToggleFavorite(item) => self.toggle_favorite(item.clone()),
            Action::HandIn {
                assignment,
                url,
                path,
            } => self.hand_in(assignment.clone(), url.clone(), path.clone()),
            Action::DeleteSubmission { assignment, file } => {
                self.delete_submission(assignment.clone(), file.clone())
            }
            Action::Upload { container, paths } => self.upload(container.clone(), paths.clone()),
            Action::Sync(roots) => self.sync(roots.clone()),
            Action::Synced { .. } => self.synced()?,
            Action::SetConfig(config) => self.set_config(config.clone())?,
            Action::Login => self.login(),
            _ => {}
        }
        Ok(Broadcast::Yes)
    }

    /// Where a page comes from: offline the cache only; else the cached copy now and the
    /// current one in the background; else ILIAS.
    fn load(
        &mut self,
        item: Option<Item>,
        url: Option<String>,
        refresh: bool,
        deep: bool,
    ) -> Result<()> {
        // Opening a course (or folder) marks what a sync found new in it as seen.
        if let Some(opened) = item.as_ref().filter(|_| !deep) {
            let new = self.seen.open(opened.key());
            if !new.is_empty() {
                self.tx.send(Action::NewItems(new))?;
                self.tx.send(Action::NewCounts(self.seen.counts()))?;
            }
        }
        let key = Cache::key(
            url.as_deref().or(item.as_ref().map(|i| i.url.as_str())),
            deep,
        );
        if self.client.is_offline() {
            let cached = if deep {
                self.cache.descendants(item.as_ref()).map(Page::from_items)
            } else {
                self.cache.get(&key)
            };
            self.tx.send(match cached {
                Some(page) => Action::Loaded {
                    item,
                    page,
                    refresh,
                    deep,
                },
                None => Action::Error(format!(
                    "Not available offline: {} was not opened before. Press L to go online.",
                    item.as_ref().map_or("My courses", |i| i.title.as_str())
                )),
            })?;
        } else if let Some(page) = (!refresh).then(|| self.cache.get(&key)).flatten() {
            // The refresh waits for the session check, which refreshes the page on screen.
            self.tx.send(Action::Loaded {
                item: item.clone(),
                page,
                refresh: false,
                deep,
            })?;
            if !self.checking {
                self.tx.send(Action::Load {
                    item,
                    url,
                    refresh: true,
                    deep,
                })?;
            }
        } else if self.checking && item.is_none() {
            // My courses for the first time: the session check reads the course list
            // anyway, so wait for it.
        } else {
            self.fetch(item, url, refresh, deep, key);
        }
        Ok(())
    }

    /// Fetches a page in the background and caches it.
    fn fetch(
        &self,
        item: Option<Item>,
        url: Option<String>,
        refresh: bool,
        deep: bool,
        key: String,
    ) {
        let cache = self.cache.clone();
        let searching = deep.then(|| {
            self.searching.store(true, Ordering::SeqCst);
            Running(self.searching.clone())
        });
        self.spawn(move |client, tx| {
            // Cleared when the job ends, whether it succeeds or fails.
            let _searching = searching;
            let page = match (&item, deep) {
                (_, true) => Page::from_items(client.descendants(item.as_ref(), |n| {
                    let _ = tx.send(Action::Progress(format!("Searching… {n} folders")));
                })?),
                (Some(item), false) => client.page(item, url.as_deref())?,
                // The cached copy knows the To-Do's courses.
                (None, false) => client.my_courses_after(cache.get(&key).as_ref())?,
            };
            // A page on screen: what's new on it is seen now, so only mark it.
            let new = match deep {
                true => {
                    cache.put(&key, &page);
                    Vec::new()
                }
                false => cache.update(&key, &page).all(),
            };
            if !new.is_empty() {
                let _ = tx.send(Action::NewItems(new));
            }
            Ok(Action::Loaded {
                item,
                page,
                refresh,
                deep,
            })
        });
    }

    fn download(&self, item: Item, open: bool) {
        let dir = self.download_dir.clone();
        self.spawn(move |client, _| {
            let path = client.download(&item, &dir)?;
            Ok(Action::Downloaded { item, path, open })
        });
    }

    fn downloaded(&mut self, item: &Item, path: &Path, open: bool) -> Result<()> {
        self.downloads
            .record(item.key(), path.to_path_buf(), sync::version(item))?;
        if open {
            self.tx.send(Action::OpenFile(path.to_path_buf()))?;
        }
        Ok(())
    }

    /// ILIAS links open at once; another site only after asking (it could be anything
    /// someone put on a page); anything but a web link never.
    fn open_in_browser(&self, item: &Item) -> Result<()> {
        let next = if open::is_ilias_link(&item.url, &self.client.base) {
            Action::OpenLink(item.url.clone())
        } else if open::is_web_link(&item.url) {
            Action::Ask {
                question: format!("Open {} in your browser? It is not on ILIAS.", item.url),
                then: Box::new(Action::OpenLink(item.url.clone())),
            }
        } else {
            Action::Error(format!("Not opening {}: not a web link", item.url))
        };
        self.tx.send(next)?;
        Ok(())
    }

    fn copy_link(&self, item: &Item) -> Result<()> {
        let link = item.permalink(&self.client.base);
        crate::clipboard::copy(&link)?;
        self.tx.send(Action::Copied(link))?;
        Ok(())
    }

    fn toggle_favorite(&self, item: Item) {
        self.spawn(move |client, _| {
            let sent = client.toggle_favorite(&item)?;
            Ok(Action::FavoriteToggled {
                item,
                dry_run: !sent,
            })
        });
    }

    fn hand_in(&self, assignment: Item, url: String, path: PathBuf) {
        self.spawn(move |client, _| {
            let file = client.hand_in(&url, &path)?;
            Ok(Action::HandedIn { assignment, file })
        });
    }

    fn delete_submission(&self, assignment: Item, file: Item) {
        self.spawn(move |client, _| {
            let deleted = client.delete_submission(&file)?;
            Ok(Action::SubmissionDeleted {
                assignment,
                file,
                deleted,
            })
        });
    }

    fn upload(&self, container: Item, paths: Vec<PathBuf>) {
        self.spawn(move |client, _| {
            let files = client.upload_files(&container, &paths)?;
            Ok(Action::Uploaded { container, files })
        });
    }

    /// Saves the files below `roots` on a worker; one sync at a time (see `refusal`).
    fn sync(&self, roots: Vec<Item>) {
        let dir = self.download_dir.clone();
        let cache = self.cache.clone();
        let rules = sync::Rules::new(&self.config);
        self.syncing.store(true, Ordering::SeqCst);
        let syncing = Running(self.syncing.clone());
        self.spawn(move |client, tx| {
            // Cleared when the job ends, whether it succeeds or fails.
            let _syncing = syncing;
            let mut downloads = Downloads::load(&client.base);
            let summary = sync::sync(client, &roots, &dir, &mut downloads, &cache, rules, |p| {
                let _ = tx.send(Action::Progress(p));
            })?;
            let mut seen = Seen::load(&client.base);
            for new in &summary.unseen {
                seen.record(new.root, &new.path, new.container, &new.new);
            }
            Ok(Action::Synced {
                summary: summary.text(rules.max_mb),
                report: summary.report(&dir, rules.max_mb),
                saved: summary.saved,
            })
        });
    }

    /// The sync wrote its own copies of the index and of what's new; read them again.
    fn synced(&mut self) -> Result<()> {
        self.downloads = Downloads::load(&self.client.base);
        self.seen = Seen::load(&self.client.base);
        self.tx.send(Action::NewCounts(self.seen.counts()))?;
        Ok(())
    }

    fn set_config(&mut self, config: Config) -> Result<()> {
        let dir_changed = config.download_dir != self.config.download_dir;
        self.config = config;
        self.config.save()?;
        crate::icons::set_nerd_font(self.config.nerd_font());
        crate::theme::apply(self.config.palette().0);
        self.client.set_dry_run(self.config.dry_run());
        if dir_changed {
            // Chosen here, it applies now, even over ILIAS_DOWNLOAD_DIR.
            self.download_dir =
                (self.config.download_dir.clone()).unwrap_or_else(|| self.config.download_dir());
            self.tx
                .send(Action::DownloadDirSet(self.download_dir.clone()))?;
        }
        Ok(())
    }

    /// Offline, first tries the saved session; else (or if it expired) the browser login.
    fn login(&self) {
        self.spawn(|client, _| {
            if client.is_offline() {
                client.set_offline(false);
                match client.check_session() {
                    Ok(true) => return Ok(Action::LoggedIn),
                    Ok(false) => {}
                    Err(e) => {
                        client.set_offline(true);
                        return Err(e.wrap_err("Still offline: ILIAS can't be reached"));
                    }
                }
            }
            client.set_cookies(auth::browser_login(&client.base)?);
            session::save(&client.base, &client.cookies())?;
            Ok(Action::LoggedIn)
        });
    }

    /// Runs blocking network work off the UI thread. The job may send progress actions;
    /// its result is sent when it finishes, errors as [`Action::Error`].
    pub(super) fn spawn(
        &self,
        job: impl FnOnce(&Client, &Sender<Action>) -> Result<Action> + Send + 'static,
    ) {
        let (client, tx) = (self.client.clone(), self.tx.clone());
        thread::spawn(move || {
            let action = job(&client, &tx).unwrap_or_else(|e| Action::Error(format!("{e:#}")));
            let _ = tx.send(action);
        });
    }
}
