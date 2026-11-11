//! The executor: runs the ops you agreed to (one-way sync: all of them, straight away), in
//! dependency order: downloads (3 at a time), new folders before what is in them, uploads,
//! replaced and renamed files, the ILIAS trash, your Trash. Each goes through the same
//! steps: re-check that ILIAS and your disk are still as planned (else it is a conflict and
//! nothing is sent), do it, update the base, and for every write a line in the journal.
//! Writes go only through the write module's own functions (`Client::upload_files`, …).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::journal::Journal;
use super::model::{Base, Disk, Rel, name, parent};
use super::ops::{Op, Why};
use super::version;
use crate::ilias::{Client, Item, Kind, Page, lists_as, sanitize};

/// Downloads running at the same time; polite to the server, still much faster than one.
const PARALLEL: usize = 3;

/// How one op went.
#[derive(Debug, Clone, PartialEq)]
pub enum Done {
    /// Done; `saved`: where a download (or ILIAS' copy beside yours) ended up.
    Ok {
        saved: Option<Rel>,
    },
    /// Changed since the plan: nothing sent, both copies kept.
    Conflict(String),
    Failed(String),
    /// Dry run: nothing sent.
    DryRun,
}

pub const OK: Done = Done::Ok { saved: None };

/// What a download op fetches: the file at its version, and your copy it replaces or (your
/// copy being edited) goes beside.
struct Fetch<'a> {
    file: &'a Item,
    version: &'a str,
    replace: Option<&'a str>,
    beside: Option<&'a str>,
}

fn fetch(op: &Op) -> Option<Fetch<'_>> {
    match op {
        Op::Download {
            file,
            version,
            replace,
            ..
        } => Some(Fetch {
            file,
            version,
            replace: replace.as_deref(),
            beside: None,
        }),
        Op::Conflict {
            path,
            why: Why::Both { file, version },
        } => Some(Fetch {
            file,
            version,
            replace: None,
            beside: Some(path),
        }),
        _ => None,
    }
}

/// Runs `ops` and returns how each went, in the order of `ops`; those that aren't run
/// (kept, conflicts other than ILIAS' copy beside yours) are left out.
pub fn run(
    client: &Client,
    ops: &[Op],
    base: &mut Base,
    journal: &mut Journal,
    progress: impl Fn(String) + Sync,
) -> Vec<(Op, Done)> {
    let mut run = Run {
        client,
        base,
        journal,
        done: Vec::new(),
    };
    run.downloads(
        ops.iter().filter_map(|op| Some((op, fetch(op)?))).collect(),
        &progress,
    );
    let of = |f: fn(&Op) -> bool| ops.iter().filter(move |op| f(op));
    for op in of(|op| matches!(op, Op::MkdirRemote { .. })) {
        progress(format!("Two-way sync: new folder {}", op.path()));
        run.mkdir(op);
    }
    run.uploads(
        of(|op| matches!(op, Op::Upload { .. })).collect(),
        &progress,
    );
    for op in of(|op| matches!(op, Op::Replace { .. } | Op::Rename { .. })) {
        progress(format!("Two-way sync: replacing {}", op.path()));
        let done = run.replace(op);
        run.done.push((op.clone(), done));
    }
    // Files first, then folders (a folder's files may be proposed on their own too).
    let files = of(|op| matches!(op, Op::TrashRemote { item, .. } if item.kind != Kind::Folder));
    let folders = of(|op| matches!(op, Op::TrashRemote { item, .. } if item.kind == Kind::Folder));
    for op in files.chain(folders) {
        progress(format!("Two-way sync: {} to the ILIAS trash", op.path()));
        let done = run.trash(op);
        run.done.push((op.clone(), done));
    }
    for op in of(|op| matches!(op, Op::TrashLocal { .. })) {
        let done = run.trash_local(op.path());
        run.done.push((op.clone(), done));
    }
    let mut done = run.done;
    done.sort_by_key(|(op, _)| ops.iter().position(|o| o == op));
    done
}

struct Run<'a> {
    client: &'a Client,
    base: &'a mut Base,
    journal: &'a mut Journal,
    done: Vec<(Op, Done)>,
}

/// How the journal names an ILIAS object.
fn object(item: &Item) -> String {
    format!("{} {} {}", item.kind.code(), item.key(), item.title)
}

impl Run<'_> {
    /// A write that failed: in the journal, and the result.
    fn failed(&mut self, action: &str, place: &Item, path: &str, why: String) -> Done {
        self.journal(action, path, &object(place), &format!("failed: {why}"));
        Done::Failed(format!("{path}: {why}"))
    }

    fn journal(&mut self, action: &str, path: &str, ilias: &str, result: &str) {
        let _ = self
            .journal
            .write(action, &self.base.abs(path), ilias, result);
    }

    /// Records `path` as ILIAS' `item` as it is listed now.
    fn record(&mut self, path: &str, item: &Item) -> color_eyre::Result<()> {
        let abs = self.base.abs(path);
        self.base.record(&abs, item.key(), version(item))
    }

    /// Fetches the files `PARALLEL` at a time into fresh files, then puts each in place.
    fn downloads(&mut self, ops: Vec<(&Op, Fetch)>, progress: &(impl Fn(String) + Sync)) {
        let (client, root) = (self.client, self.base.root().to_path_buf());
        let (next, count) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let fetched = Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            for _ in 0..PARALLEL.min(ops.len()) {
                scope.spawn(|| {
                    while let Some((op, f)) = ops.get(next.fetch_add(1, Ordering::Relaxed)) {
                        let fresh = client.download(f.file, &root);
                        fetched
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push((*op, fresh));
                        let n = count.fetch_add(1, Ordering::Relaxed) + 1;
                        progress(format!("Sync: {n}/{} files", ops.len()));
                    }
                });
            }
        });
        for (op, fresh) in fetched.into_inner().unwrap_or_else(|e| e.into_inner()) {
            let Some(f) = fetch(op) else { continue };
            let done = fresh
                .and_then(|fresh| self.place(&f, fresh))
                .unwrap_or_else(|e| Done::Failed(format!("{}: {e:#}", f.file.title)));
            self.done.push((op.clone(), done));
        }
    }

    /// Puts a fresh download where it goes: in place of your copy if that is still as
    /// saved (the re-check), else beside it.
    fn place(&mut self, f: &Fetch, fresh: PathBuf) -> color_eyre::Result<Done> {
        let base = &mut *self.base;
        let untouched = |target: &str| {
            base.file(target)
                .is_some_and(|s| s.now(&base.abs(target)) == Disk::Untouched)
        };
        let path = match (f.beside.or(f.replace.filter(|t| !untouched(t))), f.replace) {
            (Some(yours), _) => {
                // Your edited copy keeps its place and stays edited; only the version is new.
                let today = jiff::Zoned::now().date().to_string();
                let path = super::beside_name(&base.abs(yours), &today);
                std::fs::rename(&fresh, &path)?;
                base.change(|e| {
                    if let Some(entry) = e.files.get_mut(yours) {
                        entry.version = f.version.to_string();
                    }
                })?;
                return Ok(Done::Ok {
                    saved: base.rel(&path),
                });
            }
            (None, Some(target)) => {
                let path = base.abs(target);
                std::fs::rename(&fresh, &path)?;
                path
            }
            (None, None) => fresh,
        };
        base.record(&path, f.file.key(), f.version.to_string())?;
        Ok(Done::Ok {
            saved: base.rel(&path),
        })
    }

    /// Uploads, place by place, in the order planned.
    fn uploads(&mut self, ops: Vec<&Op>, progress: &impl Fn(String)) {
        let mut places: Vec<(&Item, Vec<&Op>)> = Vec::new();
        for op in ops {
            let Op::Upload { place, .. } = op else {
                continue;
            };
            match places.iter_mut().find(|(p, _)| p.id == place.id) {
                Some((_, ops)) => ops.push(op),
                None => places.push((place, vec![op])),
            }
        }
        let total: usize = places.iter().map(|(_, ops)| ops.len()).sum();
        let mut sent = 0;
        for (place, ops) in places {
            progress(format!("Two-way sync: uploading {sent}/{total} files"));
            sent += ops.len();
            let paths = ops.iter().map(|op| op.path().to_string()).collect();
            for (done, op) in self.upload_into(place, paths).into_iter().zip(ops) {
                self.done.push((op.clone(), done));
            }
        }
    }

    /// Uploads `paths` into `place`, after reading it again: a file of the same name there
    /// now is a conflict. Each file sent is recorded as in sync. Returns how each went, in
    /// the order of `paths`.
    fn upload_into(&mut self, place: &Item, paths: Vec<Rel>) -> Vec<Done> {
        // The re-check: ILIAS as it is now, not as planned.
        let page = match self.client.page(place, None) {
            Ok(page) => page,
            Err(e) => {
                let why = format!("{e:#}");
                return (paths.iter())
                    .map(|p| self.failed("upload", place, p, why.clone()))
                    .collect();
            }
        };
        let mut results: Vec<Option<Done>> = Vec::new();
        for path in &paths {
            let listed =
                (page.items()).any(|i| i.kind == Kind::File && lists_as(&i.title, name(path)));
            results.push(if !self.base.abs(path).is_file() {
                Some(self.failed("upload", place, path, "no longer on your disk".into()))
            } else if listed {
                let why = format!("{path}: ILIAS has a file of this name since the plan");
                Some(Done::Conflict(why))
            } else {
                None
            });
        }
        let send: Vec<PathBuf> = (paths.iter().zip(&results))
            .filter(|(_, done)| done.is_none())
            .map(|(p, _)| self.base.abs(p))
            .collect();
        let sent = match send.is_empty() {
            true => Ok(None),
            false => self.client.upload_files(place, &send),
        };
        // Now in sync: record each file as ILIAS lists it.
        let listed: Vec<Item> = match &sent {
            Ok(Some(_)) => (self.client.page(place, None))
                .map(|p| p.items().cloned().collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        let mut out = Vec::new();
        for (path, done) in paths.iter().zip(results) {
            out.push(match (done, &sent) {
                (Some(done), _) => done,
                (None, Ok(None)) => Done::DryRun,
                (None, Err(e)) => self.failed("upload", place, path, format!("{e:#}")),
                (None, Ok(Some(_))) => {
                    let item = listed.iter().find(|i| {
                        i.kind == Kind::File
                            && lists_as(&i.title, name(path))
                            && self.base.find(i.key()).is_none()
                    });
                    let result = match item.map(|item| (item, self.record(path, item))) {
                        Some((item, Ok(()))) => format!("ok: {}", object(item)),
                        Some((_, Err(e))) => format!("ok, not recorded: {e:#}"),
                        None => "ok, not recorded: not found in the listing".into(),
                    };
                    self.journal("upload", path, &object(place), &result);
                    OK
                }
            });
        }
        out
    }

    /// Creates your new folder in `place`, then the folders in it, each before what is in
    /// it, then each one's files. Reads `place` again first: anything of that name there now
    /// is a conflict, and nothing below it is sent.
    fn mkdir(&mut self, op: &Op) {
        let Op::MkdirRemote {
            place,
            dir,
            folders,
            files,
        } = op
        else {
            return;
        };
        let done = self.make_tree(place, dir, folders, files);
        self.done.push((op.clone(), done));
    }

    fn make_tree(&mut self, place: &Item, dir: &Rel, folders: &[Rel], files: &[Rel]) -> Done {
        let taken = |page: &Page| {
            (page.items()).any(|i| sanitize(&i.title).eq_ignore_ascii_case(name(dir)))
        };
        match self.client.page(place, None) {
            Err(e) => return self.failed("new folder", place, dir, format!("{e:#}")),
            Ok(page) if taken(&page) => {
                return Done::Conflict(format!(
                    "{dir}: ILIAS has something of this name here since the plan; nothing in it sent"
                ));
            }
            Ok(_) if !self.base.abs(dir).is_dir() => {
                return self.failed("new folder", place, dir, "no longer on your disk".into());
            }
            Ok(_) => {}
        }
        // The ILIAS folder of each local folder created so far.
        let mut created: HashMap<Rel, Item> = HashMap::new();
        let mut problems = Vec::new();
        for folder in std::iter::once(dir).chain(folders) {
            let parent = match folder == dir {
                true => Some(place.clone()),
                false => created.get(parent(folder)).cloned(),
            };
            // Its parent couldn't be created: nothing below it is sent.
            let Some(parent) = parent else { continue };
            match self.make_folder(&parent, folder) {
                Ok(Some(item)) => {
                    self.journal(
                        "new folder",
                        folder,
                        &object(&parent),
                        &format!("ok: {}", object(&item)),
                    );
                    let _ = self
                        .base
                        .change(|e| e.record_folder(folder, Some(item.key())));
                    created.insert(folder.clone(), item);
                }
                Ok(None) => return Done::DryRun,
                Err(e) => {
                    problems.push(self.failed("new folder", &parent, folder, format!("{e:#}")))
                }
            }
        }
        let mut created: Vec<(Rel, Item)> = created.into_iter().collect();
        created.sort_by(|a, b| a.0.cmp(&b.0));
        for (folder, item) in created {
            let here: Vec<Rel> = files
                .iter()
                .filter(|p| parent(p) == folder)
                .cloned()
                .collect();
            if !here.is_empty() {
                problems.extend(self.upload_into(&item, here));
            }
        }
        let problems: Vec<String> = (problems.into_iter())
            .filter_map(|done| match done {
                Done::Failed(why) | Done::Conflict(why) => Some(why),
                Done::Ok { .. } | Done::DryRun => None,
            })
            .collect();
        match problems.is_empty() {
            true => OK,
            false => Done::Failed(problems.join("; ")),
        }
    }

    /// Creates the folder `dir` (by its name) in `parent` and returns it as ILIAS lists it;
    /// `None` in a dry run.
    fn make_folder(&self, parent: &Item, dir: &str) -> color_eyre::Result<Option<Item>> {
        let name = name(dir);
        if !self.client.create_folder(parent, name)? {
            return Ok(None);
        }
        let listed = self.client.page(parent, None)?;
        let folder = listed
            .items()
            .find(|i| i.kind == Kind::Folder && i.title == name);
        match folder {
            Some(folder) => Ok(Some(folder.clone())),
            None => color_eyre::eyre::bail!("ILIAS doesn't list the new folder {name}"),
        }
    }

    /// Puts your edited file in place of ILIAS' one, if ILIAS still has the version it was
    /// edited from; for a rename (the old name still gone from your disk) with your new name
    /// as the title. Records it as in sync.
    fn replace(&mut self, op: &Op) -> Done {
        let (place, file, path, base, from) = match op {
            Op::Replace {
                place,
                file,
                path,
                base,
            } => (place, file, path, base, None),
            Op::Rename {
                place,
                file,
                from,
                path,
                base,
            } => (place, file, path, base, Some(from)),
            _ => return Done::Failed("not a replacement".into()),
        };
        let action = if from.is_some() { "rename" } else { "replace" };
        // The re-check: the file as ILIAS lists it now, and your disk now.
        let now = match self.client.page(place, None) {
            Ok(page) => page.items().find(|i| i.id == file.id).cloned(),
            Err(e) => return self.failed(action, place, path, format!("{e:#}")),
        };
        match now {
            None => return self.failed(action, place, path, "no longer on ILIAS".into()),
            Some(now) if version(&now) != *base => {
                let now = version(&now);
                return Done::Conflict(format!(
                    "{path}: changed on ILIAS since the plan ({base} → {now})"
                ));
            }
            Some(_) if !self.base.abs(path).is_file() => {
                return self.failed(action, place, path, "no longer on your disk".into());
            }
            Some(_) if from.is_some_and(|old| self.base.abs(old).exists()) => {
                return Done::Conflict(format!(
                    "{path}: the old name is back on your disk; not renamed"
                ));
            }
            Some(_) => {}
        }
        let title = from.map(|_| name(path));
        match self
            .client
            .replace_file(place, file, &self.base.abs(path), title)
        {
            Ok(None) => Done::DryRun,
            Ok(Some(number)) => {
                // In sync again: ILIAS' file is your copy now.
                let listed = (self.client.page(place, None).ok())
                    .and_then(|p| p.items().find(|i| i.id == file.id).cloned());
                let result = match listed.map(|item| self.record(path, &item)) {
                    Some(Ok(())) => format!("ok: version {number} of {}", object(file)),
                    Some(Err(e)) => format!("ok: version {number}, not recorded: {e:#}"),
                    None => format!("ok: version {number}, not recorded: not in the listing"),
                };
                self.journal(action, path, &object(place), &result);
                OK
            }
            Err(e) => self.failed(action, place, path, format!("{e:#}")),
        }
    }

    /// Moves a file (or folder) you deleted into the ILIAS trash, if ILIAS still has the
    /// version you had and it is still gone from your disk. `delete_item` refuses where
    /// ILIAS has no trash.
    fn trash(&mut self, op: &Op) -> Done {
        let Op::TrashRemote {
            place,
            item,
            path,
            base,
            ..
        } = op
        else {
            return Done::Failed("not a deletion".into());
        };
        let now = match self.client.page(place, None) {
            Ok(page) => page.items().find(|i| i.id == item.id).cloned(),
            Err(e) => return self.failed("trash", place, path, format!("{e:#}")),
        };
        match now {
            None => return self.failed("trash", place, path, "no longer on ILIAS".into()),
            Some(now) if version(&now) != *base => {
                let now = version(&now);
                return Done::Conflict(format!(
                    "{path}: changed on ILIAS since the plan ({base} → {now}); not moved to the trash"
                ));
            }
            Some(_) if self.base.abs(path).exists() => {
                return Done::Conflict(format!(
                    "{path}: back on your disk; not moved to the trash"
                ));
            }
            Some(_) => {}
        }
        match self.client.delete_item(place, item) {
            Ok(false) => Done::DryRun,
            Ok(true) => {
                let result = format!("ok: {} in the ILIAS trash", object(item));
                self.journal("trash", path, &object(place), &result);
                let _ = self.base.change(|e| match item.kind {
                    Kind::Folder => e.forget_below(path),
                    _ => e.forget_file(path),
                });
                OK
            }
            Err(e) => self.failed("trash", place, path, format!("{e:#}")),
        }
    }

    /// Moves your untouched copy of a file gone on ILIAS into your Trash, if it is still
    /// exactly as saved.
    fn trash_local(&mut self, path: &str) -> Done {
        let (ok, kept) = super::local::trashable(&[path.to_string()], self.base);
        if let Some(why) = kept.into_iter().next() {
            return Done::Conflict(why);
        }
        // Gone from your disk meanwhile, or moved now: either way not saved any more.
        let moved = match ok.is_empty() {
            true => Ok(()),
            false => super::local::to_trash(&self.base.abs(path)),
        };
        let done = match moved {
            Ok(()) => OK,
            Err(e) => Done::Failed(format!("{path}: {e}")),
        };
        if !ok.is_empty() {
            let result = match &done {
                Done::Failed(why) => format!("failed: {why}"),
                _ => "ok: in your trash".into(),
            };
            self.journal("local trash", path, "gone on ILIAS", &result);
        }
        if done == OK {
            let _ = self.base.change(|e| e.forget_file(path));
        }
        done
    }
}

#[cfg(test)]
mod tests;
