//! Two-way sync's writes: sends the changes of a [`Review`](super::Review) you agreed to,
//! through the write module's own functions (`Client::upload_files`, …). Before each place
//! is written to, its page is read again; anything that changed there since the plan
//! becomes a conflict and is not sent. Every write goes into the [`Journal`], and the base
//! is updated after each one, so the next sync sees the file as in sync.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{Change, journal::Journal, version};
use crate::downloads::Downloads;
use crate::ilias::{Client, Item, Kind, lists_as};

/// What a push did.
#[derive(Debug, Default, PartialEq)]
pub struct Pushed {
    pub uploaded: Vec<PathBuf>,
    /// Edited files that replaced ILIAS' ones.
    pub replacements: Vec<PathBuf>,
    /// Files you deleted, now in the ILIAS trash.
    pub trashed: Vec<PathBuf>,
    /// Local folders now also on ILIAS.
    pub folders: Vec<PathBuf>,
    /// Changed on ILIAS since the plan: nothing sent.
    pub conflicts: Vec<String>,
    pub failed: Vec<String>,
    /// Dry run: nothing was sent.
    pub dry_run: bool,
}

impl Pushed {
    /// One line for the status bar.
    pub fn text(&self) -> String {
        if self.dry_run {
            return "Dry run: nothing sent to ILIAS".into();
        }
        let mut parts = vec![format!("{} uploaded", self.uploaded.len())];
        if !self.replacements.is_empty() {
            parts.push(format!("{} replaced", self.replacements.len()));
        }
        if !self.folders.is_empty() {
            parts.push(format!("{} new folders", self.folders.len()));
        }
        if !self.trashed.is_empty() {
            parts.push(format!("{} moved to the ILIAS trash", self.trashed.len()));
        }
        if !self.conflicts.is_empty() {
            parts.push(format!(
                "{} changed on ILIAS meanwhile (not sent)",
                self.conflicts.len()
            ));
        }
        if !self.failed.is_empty() {
            parts.push(format!("{} failed", self.failed.len()));
        }
        format!("Sent to ILIAS: {}", parts.join(", "))
    }

    /// What deserves a look, by section; `dir` shortens paths.
    pub fn report(&self, dir: &Path) -> Vec<(String, Vec<String>)> {
        let prefix = format!("{}{}", dir.display(), std::path::MAIN_SEPARATOR);
        let short = |lines: &[String]| lines.iter().map(|l| l.replace(&prefix, "")).collect();
        [
            (
                "Changed on ILIAS since the plan: nothing sent".to_string(),
                short(&self.conflicts),
            ),
            ("Failed".to_string(), short(&self.failed)),
        ]
        .into_iter()
        .filter(|(_, lines): &(String, Vec<String>)| !lines.is_empty())
        .collect()
    }
}

/// Notes a write that failed, in the journal and the result.
fn fail(journal: &mut Journal, pushed: &mut Pushed, place: &Item, path: &Path, why: String) {
    fail_as("upload", journal, pushed, place, path, why);
}

fn fail_as(
    action: &str,
    journal: &mut Journal,
    pushed: &mut Pushed,
    place: &Item,
    path: &Path,
    why: String,
) {
    let _ = journal.write(action, path, &object(place), &format!("failed: {why}"));
    pushed.failed.push(format!("{}: {why}", path.display()));
}

/// How the journal names an ILIAS object.
fn object(item: &Item) -> String {
    format!("{} {} {}", item.kind.code(), item.key(), item.title)
}

/// Sends the uploads among `changes`, place by place. Reads each place again first: a file
/// of the same name that appeared there since the plan is a conflict. Afterwards each
/// uploaded file is recorded in `downloads` with its ILIAS id and version.
pub fn push(
    client: &Client,
    changes: &[Change],
    downloads: &mut Downloads,
    journal: &mut Journal,
    progress: impl Fn(String),
) -> Pushed {
    let mut pushed = Pushed::default();
    // Uploads by place, in the order planned.
    let mut places: Vec<(&Item, Vec<&PathBuf>)> = Vec::new();
    let mut renames = Vec::new();
    for change in changes {
        if let Change::Upload {
            place,
            path,
            renamed_from,
        } = change
        {
            match places.iter_mut().find(|(p, _)| p.id == place.id) {
                Some((_, paths)) => paths.push(path),
                None => places.push((place, vec![path])),
            }
            if renamed_from.is_some() {
                renames.push(path);
            }
        }
    }
    let total: usize = places.iter().map(|(_, paths)| paths.len()).sum();
    let mut done = 0;
    for (place, paths) in places {
        progress(format!("Two-way sync: uploading {done}/{total} files"));
        done += paths.len();
        upload_into(
            client,
            place,
            paths,
            &renames,
            downloads,
            journal,
            &mut pushed,
        );
    }
    for change in changes {
        if let Change::NewFolder {
            place,
            dir,
            contents,
        } = change
        {
            progress(format!("Two-way sync: new folder {}", dir.display()));
            new_folder(
                client,
                place,
                dir,
                contents,
                downloads,
                journal,
                &mut pushed,
            );
        }
    }
    for change in changes {
        if let Change::Replace {
            place,
            file,
            path,
            base,
        } = change
        {
            progress(format!("Two-way sync: replacing {}", file.title));
            replace(
                client,
                place,
                file,
                path,
                base,
                downloads,
                journal,
                &mut pushed,
            );
        }
    }
    for change in changes {
        if let Change::Trash {
            place,
            file,
            path,
            base,
        } = change
        {
            progress(format!("Two-way sync: {} to the ILIAS trash", file.title));
            trash(client, place, file, path, base, journal, &mut pushed);
        }
    }
    pushed
}

/// Moves one file you deleted (and ticked) into the ILIAS trash, if ILIAS still has the
/// version you had and it is still gone from your disk. `delete_item` refuses where ILIAS
/// has no trash.
fn trash(
    client: &Client,
    place: &Item,
    file: &Item,
    path: &Path,
    base: &str,
    journal: &mut Journal,
    pushed: &mut Pushed,
) {
    let fail = |journal: &mut Journal, pushed: &mut Pushed, why: String| {
        fail_as("trash", journal, pushed, place, path, why);
    };
    // The re-check: the file as ILIAS lists it now, and your disk now.
    let now = match client.page(place, None) {
        Ok(page) => page.items().find(|i| i.id == file.id).cloned(),
        Err(e) => return fail(journal, pushed, format!("{e:#}")),
    };
    match now {
        None => return fail(journal, pushed, "no longer on ILIAS".into()),
        Some(now) if version(&now) != base => {
            pushed.conflicts.push(format!(
                "{}: changed on ILIAS since the plan ({} → {}); not moved to the trash",
                path.display(),
                base,
                version(&now)
            ));
            return;
        }
        Some(_) if path.exists() => {
            pushed.conflicts.push(format!(
                "{}: back on your disk; not moved to the trash",
                path.display()
            ));
            return;
        }
        Some(_) => {}
    }
    match client.delete_item(place, file) {
        Ok(false) => pushed.dry_run = true,
        Ok(true) => {
            let result = format!("ok: {} in the ILIAS trash", object(file));
            let _ = journal.write("trash", path, &object(place), &result);
            pushed.trashed.push(path.to_path_buf());
        }
        Err(e) => fail(journal, pushed, format!("{e:#}")),
    }
}

/// Uploads `paths` into `place` (a course or folder on ILIAS), after reading it again: a
/// file of the same name there now is a conflict. Each file sent is recorded as in sync.
#[allow(clippy::too_many_arguments)]
fn upload_into(
    client: &Client,
    place: &Item,
    paths: Vec<&PathBuf>,
    renames: &[&PathBuf],
    downloads: &mut Downloads,
    journal: &mut Journal,
    pushed: &mut Pushed,
) {
    // The re-check: ILIAS as it is now, not as planned.
    let page = match client.page(place, None) {
        Ok(page) => page,
        Err(e) => {
            for path in paths {
                fail(journal, pushed, place, path, format!("{e:#}"));
            }
            return;
        }
    };
    let mut send = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if !path.is_file() {
            fail(
                journal,
                pushed,
                place,
                path,
                "no longer on your disk".into(),
            );
        } else if page
            .items()
            .any(|i| i.kind == Kind::File && lists_as(&i.title, name))
        {
            pushed.conflicts.push(format!(
                "{}: ILIAS has a file of this name since the plan",
                path.display()
            ));
        } else {
            send.push(path.clone());
        }
    }
    if send.is_empty() {
        return;
    }
    match client.upload_files(place, &send) {
        Ok(None) => pushed.dry_run = true,
        Ok(Some(_)) => {
            // Now in sync: record each file as ILIAS lists it.
            let listed: Vec<Item> = client
                .page(place, None)
                .map(|p| p.items().cloned().collect())
                .unwrap_or_default();
            for path in send {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                let item = listed.iter().find(|i| {
                    i.kind == Kind::File
                        && lists_as(&i.title, name)
                        && downloads.entry(i.key()).is_none()
                });
                let result = match item {
                    Some(item) => match downloads.record(item.key(), path.clone(), version(item)) {
                        Ok(()) => format!("ok: {}", object(item)),
                        Err(e) => format!("ok, not recorded: {e:#}"),
                    },
                    None => "ok, not recorded: not found in the listing".into(),
                };
                // A rename: the old name is never proposed for the trash.
                if renames.contains(&&path) {
                    let hash = crate::downloads::hash_file(&path).unwrap_or_default();
                    let _ = downloads.mark_moved(&hash, &path);
                }
                let _ = journal.write("upload", &path, &object(place), &result);
                pushed.uploaded.push(path);
            }
        }
        Err(e) => {
            for path in &send {
                fail(journal, pushed, place, path, format!("{e:#}"));
            }
        }
    }
}

/// Creates the local folder `dir` in `place` on ILIAS, then its folders and files from
/// `contents` (as planned), each folder before what is in it. Reads `place` again first:
/// anything of that name there now is a conflict, and nothing below it is sent.
fn new_folder(
    client: &Client,
    place: &Item,
    dir: &Path,
    contents: &[PathBuf],
    downloads: &mut Downloads,
    journal: &mut Journal,
    pushed: &mut Pushed,
) {
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let taken = |page: &crate::ilias::Page| {
        page.items()
            .any(|i| crate::ilias::sanitize(&i.title).eq_ignore_ascii_case(name))
    };
    match client.page(place, None) {
        Err(e) => return fail_as("new folder", journal, pushed, place, dir, format!("{e:#}")),
        Ok(page) if taken(&page) => {
            pushed.conflicts.push(format!(
                "{}: ILIAS has something of this name here since the plan; nothing in it sent",
                dir.display()
            ));
            return;
        }
        Ok(_) if !dir.is_dir() => {
            return fail_as(
                "new folder",
                journal,
                pushed,
                place,
                dir,
                "no longer on your disk".into(),
            );
        }
        Ok(_) => {}
    }
    // The ILIAS folder of each local folder created so far.
    let mut created: HashMap<PathBuf, Item> = HashMap::new();
    let folders =
        std::iter::once(dir).chain(contents.iter().filter(|p| p.is_dir()).map(PathBuf::as_path));
    for folder in folders {
        let parent = match folder == dir {
            true => Some(place.clone()),
            false => folder.parent().and_then(|p| created.get(p)).cloned(),
        };
        // Its parent couldn't be created: nothing below it is sent.
        let Some(parent) = parent else { continue };
        match make_folder(client, &parent, folder) {
            Ok(Some(item)) => {
                let result = format!("ok: {}", object(&item));
                let _ = journal.write("new folder", folder, &object(&parent), &result);
                pushed.folders.push(folder.to_path_buf());
                created.insert(folder.to_path_buf(), item);
            }
            Ok(None) => {
                pushed.dry_run = true;
                return;
            }
            Err(e) => fail_as(
                "new folder",
                journal,
                pushed,
                &parent,
                folder,
                format!("{e:#}"),
            ),
        }
    }
    let mut folders: Vec<(&PathBuf, &Item)> = created.iter().collect();
    folders.sort_by_key(|(path, _)| path.as_path());
    for (folder, item) in folders {
        let files: Vec<&PathBuf> = contents
            .iter()
            .filter(|p| p.parent() == Some(folder.as_path()) && !p.is_dir())
            .collect();
        if !files.is_empty() {
            upload_into(client, item, files, &[], downloads, journal, pushed);
        }
    }
}

/// Creates the folder `dir` (by its name) in `parent` and returns it as ILIAS lists it;
/// `None` in a dry run.
fn make_folder(client: &Client, parent: &Item, dir: &Path) -> color_eyre::Result<Option<Item>> {
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    if !client.create_folder(parent, name)? {
        return Ok(None);
    }
    let listed = client.page(parent, None)?;
    let folder = listed
        .items()
        .find(|i| i.kind == Kind::Folder && i.title == name);
    match folder {
        Some(folder) => Ok(Some(folder.clone())),
        None => color_eyre::eyre::bail!("ILIAS doesn't list the new folder {name}"),
    }
}

/// Puts one edited file in place of ILIAS' one, if ILIAS still has the version it was
/// edited from (`base`); records it as in sync.
#[allow(clippy::too_many_arguments)]
fn replace(
    client: &Client,
    place: &Item,
    file: &Item,
    path: &Path,
    base: &str,
    downloads: &mut Downloads,
    journal: &mut Journal,
    pushed: &mut Pushed,
) {
    let fail = |journal: &mut Journal, pushed: &mut Pushed, why: String| {
        fail_as("replace", journal, pushed, place, path, why);
    };
    // The re-check: the file as ILIAS lists it now.
    let now = match client.page(place, None) {
        Ok(page) => page.items().find(|i| i.id == file.id).cloned(),
        Err(e) => return fail(journal, pushed, format!("{e:#}")),
    };
    match now {
        None => return fail(journal, pushed, "no longer on ILIAS".into()),
        Some(now) if version(&now) != base => {
            pushed.conflicts.push(format!(
                "{}: changed on ILIAS since the plan ({} → {})",
                path.display(),
                base,
                version(&now)
            ));
            return;
        }
        Some(_) if !path.is_file() => {
            return fail(journal, pushed, "no longer on your disk".into());
        }
        Some(_) => {}
    }
    match client.replace_file(place, file, path) {
        Ok(None) => pushed.dry_run = true,
        Ok(Some(number)) => {
            // In sync again: ILIAS' file is your copy now.
            let listed = client
                .page(place, None)
                .ok()
                .and_then(|p| p.items().find(|i| i.id == file.id).cloned());
            let recorded =
                listed.map(|item| downloads.record(file.key(), path.to_path_buf(), version(&item)));
            let result = match recorded {
                Some(Ok(())) => format!("ok: version {number} of {}", object(file)),
                Some(Err(e)) => format!("ok: version {number}, not recorded: {e:#}"),
                None => format!("ok: version {number}, not recorded: not in the listing"),
            };
            let _ = journal.write("replace", path, &object(place), &result);
            pushed.replacements.push(path.to_path_buf());
        }
        Err(e) => fail(journal, pushed, format!("{e:#}")),
    }
}

#[cfg(test)]
mod tests;
