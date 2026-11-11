//! The diff: what to do about every path, decided three-way from the [`Base`], ILIAS now
//! ([`Remote`]) and your disk now ([`Local`]). Pure: nothing here reads or writes.
//!
//! | Your disk vs base | ILIAS vs base  | Op                                                |
//! | ----------------- | -------------- | ------------------------------------------------- |
//! | –                 | new            | [`Op::Download`]                                  |
//! | new file          | –              | [`Op::Upload`]; a file of that name on ILIAS → conflict |
//! | new folder        | –              | [`Op::MkdirRemote`], with what is in it           |
//! | unchanged         | changed        | [`Op::Download`], replacing your copy             |
//! | edited            | unchanged      | [`Op::Replace`]                                   |
//! | edited            | changed        | [`Op::Conflict`]: ILIAS' copy saved beside        |
//! | renamed (same folder) | unchanged  | [`Op::Rename`]                                    |
//! | moved, unclear rename | unchanged  | [`Op::Upload`] + [`Op::TrashRemote`] of the old   |
//! | deleted           | unchanged      | [`Op::TrashRemote`] (a proposal)                  |
//! | folder deleted    | none changed   | one [`Op::TrashRemote`] of the folder             |
//! | deleted           | changed        | [`Op::Download`] again                            |
//! | unchanged         | gone           | [`Op::TrashLocal`] (asks)                         |
//! | edited            | gone           | [`Op::Keep`]                                      |
//! | deleted           | gone           | forgotten ([`Settle::Forget`])                    |
//! | changed, no hash  | unchanged      | [`Op::Keep`]: can't tell                          |
//!
//! The first pass decides each path on its own, in one `match`; then renames and moves pair
//! uploads with deletions, whole deleted folders replace their files' deletions, the rights
//! turn what ILIAS doesn't let you do into [`Op::Keep`]. A one-way sync decides the same
//! rows without the ops that write (and runs none of the passes).

use std::collections::{HashMap, HashSet};

pub use super::ops::{Op, Plan, Reason, Settle, Why};

use super::Rules;
use super::model::{
    Base, Disk, Local, LocalFile, Place, Rel, Remote, RemoteFile, name, parent, within,
};
use crate::ilias::{Item, Kind, sanitize};

/// ILIAS' file compared with the base.
#[derive(Debug, Clone, Copy)]
enum Ilias<'a> {
    Unchanged(&'a RemoteFile),
    Changed(&'a RemoteFile),
    Gone,
    /// Not part of what this sync read.
    NotRead,
}

/// Decides everything: see the module docs.
pub fn diff(base: &Base, remote: &Remote, local: &Local, rules: &Rules) -> Plan {
    let mut d = Diff {
        base,
        remote,
        local,
        rules,
        plan: Plan::default(),
    };
    d.files();
    d.remote_only();
    d.folders_settle();
    if rules.two_way {
        d.local_only();
        d.renames();
        d.deleted_folders();
        d.rights();
    }
    d.plan
}

struct Diff<'a> {
    base: &'a Base,
    remote: &'a Remote,
    local: &'a Local,
    rules: &'a Rules,
    plan: Plan,
}

impl Diff<'_> {
    fn op(&mut self, op: Op) {
        self.plan.ops.push(op);
    }

    fn keep(&mut self, path: &str, reason: Reason) {
        self.op(Op::Keep {
            path: path.to_string(),
            reason,
        });
    }

    /// Download `file`, unless it is over the size limit.
    fn fetch(&mut self, file: &RemoteFile, replace: Option<&str>) {
        let mb = file.item.properties.iter().find_map(|p| super::size_mb(p));
        if mb.is_some_and(|mb| mb > self.rules.max_mb) {
            let size = file
                .item
                .properties
                .iter()
                .find(|p| super::size_mb(p).is_some());
            let what = match size {
                Some(size) => format!("{} ({size})", file.item.title),
                None => file.item.title.clone(),
            };
            return self.keep(&what, Reason::TooBig);
        }
        self.op(Op::Download {
            file: file.item.clone(),
            version: file.version.clone(),
            dir: file.dir.clone(),
            replace: replace.map(str::to_string),
        });
    }

    /// Whether ILIAS no longer has what is saved at `path`: its folder was read and doesn't
    /// list it, or the folder itself is gone.
    fn gone(&self, path: &str) -> bool {
        let dir = parent(path);
        self.remote.place_at(dir).next().is_some() || self.gone_dirs().any(|d| within(dir, d))
    }

    /// Saved folders in a place that no longer lists them.
    fn gone_dirs(&self) -> impl Iterator<Item = &str> {
        self.base.folders().map(|(dir, _)| dir).filter(|dir| {
            let mut places = self.remote.place_at(parent(dir)).peekable();
            places.peek().is_some()
                && self.remote.place_at(dir).next().is_none()
                && !places.any(|p| p.lists_folder(name(dir)))
        })
    }

    /// The first pass: every saved file, from its state on both sides.
    fn files(&mut self) {
        use Disk::*;
        let (base, remote, local, rules) = (self.base, self.remote, self.local, *self.rules);
        let two = rules.two_way;
        let place = |file: &RemoteFile| file.place.map(|i| &remote.places[i]);
        for (path, entry) in base.files() {
            let now = local.files.get(path);
            let ilias = match remote.file(entry.id) {
                Some(f) if f.version == entry.version => Ilias::Unchanged(f),
                Some(f) => Ilias::Changed(f),
                None if self.gone(path) => Ilias::Gone,
                None => Ilias::NotRead,
            };
            match (entry.compare(now), ilias) {
                (_, Ilias::NotRead) => {}
                (Missing, Ilias::Gone) => self.plan.settle.push(Settle::Forget(path.into())),
                (Untouched, Ilias::Gone) if two => self.op(Op::TrashLocal { path: path.into() }),
                (Edited | Unsure, Ilias::Gone) if two => self.keep(path, Reason::GoneEdited),
                (Untouched | Edited | Unsure, Ilias::Gone) => {}
                (Untouched | Edited | Unsure, Ilias::Unchanged(f) | Ilias::Changed(f))
                    if entry.version.is_empty() =>
                {
                    self.plan.settle.push(Settle::Adopt {
                        path: path.into(),
                        version: f.version.clone(),
                    });
                    self.plan.unchanged += 1;
                }
                (Missing, Ilias::Unchanged(f) | Ilias::Changed(f)) if rules.missing_only => {
                    self.fetch(f, None)
                }
                (Unsure, Ilias::Unchanged(_)) if two => self.keep(path, Reason::CantTell),
                (Edited, Ilias::Unchanged(f)) if two => match place(f) {
                    Some(place) => self.op(Op::Replace {
                        place: place.item.clone(),
                        file: f.item.clone(),
                        path: path.into(),
                        base: entry.version.clone(),
                    }),
                    None => self.keep(path, Reason::NotAllowed),
                },
                (Untouched | Edited | Unsure, Ilias::Changed(_)) if rules.missing_only => {
                    self.plan.unchanged += 1
                }
                (Untouched | Edited | Unsure, Ilias::Unchanged(_)) => {
                    if let (true, Some(hash)) =
                        (entry.hash.is_empty(), now.and_then(|n| n.hash.clone()))
                    {
                        self.plan.settle.push(Settle::Hash {
                            path: path.into(),
                            hash,
                        });
                    }
                    self.plan.unchanged += 1;
                }
                (Missing, Ilias::Changed(f)) => self.fetch(f, None),
                (Missing, Ilias::Unchanged(f)) => match place(f).filter(|_| two) {
                    Some(place) => self.op(Op::TrashRemote {
                        place: place.item.clone(),
                        item: f.item.clone(),
                        path: path.into(),
                        base: entry.version.clone(),
                        files: 1,
                    }),
                    None => self.keep(path, Reason::Deleted),
                },
                (Untouched, Ilias::Changed(f)) => self.fetch(f, Some(path)),
                (Edited | Unsure, Ilias::Changed(f)) => {
                    let before = self.plan.ops.len();
                    self.fetch(f, None);
                    // Not over the size limit: ILIAS' copy goes beside yours.
                    if let Some(Op::Download { .. }) = self.plan.ops.get(before) {
                        self.plan.ops[before] = Op::Conflict {
                            path: path.into(),
                            why: Why::Both {
                                file: f.item.clone(),
                                version: f.version.clone(),
                            },
                        };
                    }
                }
            }
        }
    }

    /// Files on ILIAS that sync never saved.
    fn remote_only(&mut self) {
        let (base, remote) = (self.base, self.remote);
        for file in &remote.files {
            if base.find(file.item.key()).is_none() {
                self.fetch(file, None);
            }
        }
    }

    /// Files and folders on your disk that sync never saved, in the places' folders.
    fn local_only(&mut self) {
        let (base, remote, local) = (self.base, self.remote, self.local);
        let max = self.rules.max_bytes();
        let mut seen = HashSet::new();
        for dir in remote.places.iter().map(|p| p.dir.as_str()) {
            if !seen.insert(dir) {
                continue;
            }
            let places: Vec<&Place> = remote.place_at(dir).collect();
            let place = places[0];
            let new_files: Vec<(&Rel, &LocalFile)> = local
                .files
                .iter()
                .filter(|(path, _)| parent(path) == dir && base.file(path).is_none())
                .collect();
            let new_folders: Vec<&Rel> = local
                .folders
                .iter()
                .filter(|path| {
                    parent(path) == dir
                        && remote.place_at(path).next().is_none()
                        && !base.has_folder(path)
                        && !place.lists_folder(name(path))
                })
                .collect();
            if places.len() > 1 {
                if !new_files.is_empty() || !new_folders.is_empty() {
                    self.op(Op::Conflict {
                        path: dir.into(),
                        why: Why::SharedFolder,
                    });
                }
                continue;
            }
            for (path, file) in new_files {
                if file.len > max {
                    self.keep(path, Reason::TooBigToSend);
                } else if place.lists_file(name(path)) {
                    self.op(Op::Conflict {
                        path: path.clone(),
                        why: Why::SameName,
                    });
                } else {
                    self.op(Op::Upload {
                        place: place.item.clone(),
                        path: path.clone(),
                        from: None,
                    });
                }
            }
            for folder in new_folders {
                let name = name(folder);
                if sanitize(name) != name {
                    self.op(Op::Conflict {
                        path: folder.clone(),
                        why: Why::ComesBackAs(sanitize(name)),
                    });
                    continue;
                }
                let mut files: Vec<Rel> = Vec::new();
                for (path, file) in &local.files {
                    if !within(path, folder) {
                        continue;
                    }
                    match file.len > max {
                        true => self.plan.ops.push(Op::Keep {
                            path: path.clone(),
                            reason: Reason::TooBigToSend,
                        }),
                        false => files.push(path.clone()),
                    }
                }
                let folders: Vec<Rel> = local
                    .folders
                    .iter()
                    .filter(|p| *p != folder && within(p, folder))
                    .cloned()
                    .collect();
                self.op(Op::MkdirRemote {
                    place: place.item.clone(),
                    dir: folder.clone(),
                    folders,
                    files,
                });
            }
        }
    }

    /// The places' folders on your disk become saved folders; saved folders gone on both
    /// sides are forgotten.
    fn folders_settle(&mut self) {
        for place in &self.remote.places {
            if self.local.folders.contains(&place.dir) && !self.base.has_folder(&place.dir) {
                self.plan.settle.push(Settle::Folder {
                    path: place.dir.clone(),
                    id: place.item.key(),
                });
            }
        }
        let gone: Vec<Rel> = self
            .gone_dirs()
            .filter(|dir| !self.local.folders.contains(*dir))
            .map(str::to_string)
            .collect();
        for dir in gone {
            self.plan.settle.push(Settle::ForgetFolder(dir));
        }
    }

    /// Pairs new files with deleted ones of the same content: exactly one in the same folder
    /// is a rename; one elsewhere, or several, an upload, the deleted ones staying proposed.
    /// A copy of any other saved file is a conflict.
    fn renames(&mut self) {
        let (base, remote, local) = (self.base, self.remote, self.local);
        let forgotten: HashSet<Rel> = (self.plan.settle.iter())
            .filter_map(|s| match s {
                Settle::Forget(path) => Some(path.clone()),
                _ => None,
            })
            .collect();
        // Deletions proposed now, by content.
        let mut deleted: Vec<(Rel, &str)> = (self.plan.ops.iter())
            .filter_map(|op| match op {
                Op::TrashRemote { path, .. } => {
                    let hash = base.file(path)?.hash.as_str();
                    (!hash.is_empty()).then(|| (path.clone(), hash))
                }
                _ => None,
            })
            .collect();
        let mut renamed = HashSet::new();
        let mut ops = std::mem::take(&mut self.plan.ops);
        for at in 0..ops.len() {
            let Op::Upload { path, .. } = &ops[at] else {
                continue;
            };
            let Some(content) = local.files.get(path).and_then(|f| f.hash.clone()) else {
                continue;
            };
            let path = path.clone();
            let same: Vec<usize> = (0..deleted.len())
                .filter(|&i| deleted[i].1 == content)
                .collect();
            let Some(&first) = same.first() else {
                // No deletion of this content: a copy of a saved file is a conflict.
                let copy = base
                    .files()
                    .find(|(saved, e)| e.hash == content && !forgotten.contains(*saved));
                if let Some((saved, _)) = copy {
                    let why = Why::CopyOf(saved.to_string());
                    ops[at] = Op::Conflict { path, why };
                }
                continue;
            };
            let (old, _) = deleted.remove(first);
            let trash = ops
                .iter()
                .find(|op| matches!(op, Op::TrashRemote { path, .. } if *path == old));
            let rename = match trash {
                Some(Op::TrashRemote {
                    place, item, base, ..
                }) if same.len() == 1 && parent(&old) == parent(&path) => {
                    let place_rights = remote.places.iter().find(|p| p.item.id == place.id);
                    (place_rights.is_some_and(|p| p.rights.versions.contains(&item.key()))).then(
                        || Op::Rename {
                            place: place.clone(),
                            file: item.clone(),
                            from: old.clone(),
                            path: path.clone(),
                            base: base.clone(),
                        },
                    )
                }
                _ => None,
            };
            match rename {
                Some(op) => {
                    ops[at] = op;
                    renamed.insert(old);
                }
                // Moved, or which one was renamed can't be told: an upload; the deleted
                // ones stay proposed.
                None if same.len() > 1 => {
                    if let Op::Upload { from, .. } = &mut ops[at] {
                        *from = Some(old);
                    }
                }
                None => {}
            }
        }
        ops.retain(|op| !matches!(op, Op::TrashRemote { path, .. } if renamed.contains(path)));
        self.plan.ops = ops;
    }

    /// A saved folder you deleted, whose files on ILIAS are all proposed for the trash (none
    /// changed there): one proposal for the topmost, in place of its files'.
    fn deleted_folders(&mut self) {
        let (base, remote, local) = (self.base, self.remote, self.local);
        let mut folders: Vec<&str> = base
            .folders()
            .map(|(dir, _)| dir)
            .filter(|dir| !local.folders.contains(*dir))
            .collect();
        folders.sort_by_key(|dir| dir.matches('/').count());
        let mut trashed: Vec<&str> = Vec::new();
        for dir in folders {
            if trashed.iter().any(|t| within(dir, t)) {
                continue;
            }
            let Some(folder) = remote.place_at(dir).find(|p| p.item.kind == Kind::Folder) else {
                continue;
            };
            let Some(parent) = remote
                .places
                .iter()
                .find(|p| p.items.iter().any(|i| i.id == folder.item.id))
            else {
                continue;
            };
            if !parent.rights.delete.contains(&folder.item.key()) {
                continue;
            }
            let proposed: HashSet<u64> = self
                .plan
                .ops
                .iter()
                .filter_map(|op| match op {
                    Op::TrashRemote { item, .. } if item.kind == Kind::File => Some(item.key()),
                    _ => None,
                })
                .collect();
            let files: Vec<u64> = remote
                .files
                .iter()
                .filter(|f| within(&f.dir, dir))
                .map(|f| f.item.key())
                .collect();
            if !files.iter().all(|id| proposed.contains(id)) {
                continue;
            }
            self.plan.ops.retain(
                |op| !matches!(op, Op::TrashRemote { item, .. } if files.contains(&item.key())),
            );
            let listed = parent.items.iter().find(|i| i.id == folder.item.id);
            self.plan.ops.push(Op::TrashRemote {
                place: parent.item.clone(),
                item: folder.item.clone(),
                path: dir.to_string(),
                base: listed.map(super::version).unwrap_or_default(),
                files: files.len(),
            });
            trashed.push(dir);
        }
    }

    /// What ILIAS doesn't let you do there is left as it is.
    fn rights(&mut self) {
        let places: HashMap<u64, &Place> = self
            .remote
            .places
            .iter()
            .map(|p| (p.item.key(), p))
            .collect();
        let rights = |place: &Item| places.get(&place.key()).map(|p| &p.rights);
        for op in &mut self.plan.ops {
            let refused = match &*op {
                Op::Upload { place, .. } => !rights(place).is_some_and(|r| r.upload),
                Op::Replace { place, file, .. } => {
                    !rights(place).is_some_and(|r| r.versions.contains(&file.key()))
                }
                Op::MkdirRemote { place, .. } => {
                    !rights(place).is_some_and(|r| r.may_create(&Kind::Folder))
                }
                Op::TrashRemote { place, item, .. } => {
                    if !rights(place).is_some_and(|r| r.delete.contains(&item.key())) {
                        *op = Op::Keep {
                            path: op.path().to_string(),
                            reason: Reason::Deleted,
                        };
                    }
                    false
                }
                _ => false,
            };
            if refused {
                *op = Op::Keep {
                    path: op.path().to_string(),
                    reason: Reason::NotAllowed,
                };
            }
        }
    }
}

#[cfg(test)]
mod tests;
