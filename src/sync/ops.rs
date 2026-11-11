//! What sync does, one [`Op`] per path, as the [`diff`](super::diff) decides it. The review,
//! the report's sections, the status line, the brake and the question are all built from
//! these by their variant, never from their text.

use super::model::Rel;
use crate::ilias::Item;

/// What a sync does.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    pub ops: Vec<Op>,
    /// Changes to the base alone, made right away: nothing on ILIAS or your disk changes.
    pub settle: Vec<Settle>,
    /// Saved files that are the same on both sides.
    pub unchanged: usize,
}

/// One thing to do about one path. Everything shown (review, report, status line, brake)
/// is built from these, by their variant.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// Save ILIAS' `file` into `dir`; `replace`: in place of your untouched copy there.
    Download {
        file: Item,
        version: String,
        dir: Rel,
        replace: Option<Rel>,
    },
    /// Upload your new file `path` into `place`. `from`: a saved file with the same content
    /// that you deleted, shown as where it may come from (it stays proposed for the trash).
    Upload {
        place: Item,
        path: Rel,
        from: Option<Rel>,
    },
    /// Your edited `path` replaces ILIAS' `file`, which still has the version saved.
    Replace {
        place: Item,
        file: Item,
        path: Rel,
        base: String,
    },
    /// You renamed `from` to `path` in the same folder: `file` gets the new name.
    Rename {
        place: Item,
        file: Item,
        from: Rel,
        path: Rel,
        base: String,
    },
    /// Create your new folder `dir` in `place`, then the `folders` in it all the way down
    /// (each before those in it), then the `files` in all of them.
    MkdirRemote {
        place: Item,
        dir: Rel,
        folders: Vec<Rel>,
        files: Vec<Rel>,
    },
    /// Move `item` (a file, or a folder with `files` files in it) into the ILIAS trash: you
    /// deleted `path`. A proposal: ticked in the review.
    TrashRemote {
        place: Item,
        item: Item,
        path: Rel,
        base: String,
        files: usize,
    },
    /// Move your untouched `path`, gone on ILIAS, into your Trash (after asking).
    TrashLocal { path: Rel },
    /// Nothing sent; both copies stay.
    Conflict { path: Rel, why: Why },
    /// Left as it is, and why.
    Keep { path: String, reason: Reason },
}

/// Why something is a conflict.
#[derive(Debug, Clone, PartialEq)]
pub enum Why {
    /// Edited by you and changed on ILIAS: ILIAS' copy (`file` at `version`) is saved
    /// beside yours.
    Both { file: Item, version: String },
    /// ILIAS has a file of this name here that sync didn't save.
    SameName,
    /// A copy of a file sync saved.
    CopyOf(Rel),
    /// Several ILIAS folders are saved into this one folder.
    SharedFolder,
    /// A folder of this name would come back from ILIAS as another name.
    ComesBackAs(String),
}

/// Why something is left as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// On ILIAS, over the size limit (`path` is its title and size).
    TooBig,
    /// On your disk, over the size limit: not sent.
    TooBigToSend,
    /// ILIAS doesn't let you upload or change it there.
    NotAllowed,
    /// Changed on your disk, saved before content hashes were recorded.
    CantTell,
    /// Gone on ILIAS, edited by you.
    GoneEdited,
    /// Deleted by you, and not sent: one-way sync, or ILIAS doesn't let you delete it
    /// there. It is not downloaded again.
    Deleted,
}

/// A change to the base alone.
#[derive(Debug, Clone, PartialEq)]
pub enum Settle {
    /// Gone on both sides.
    Forget(Rel),
    /// Gone on both sides.
    ForgetFolder(Rel),
    /// Recorded before versions were: as it is now.
    Adopt { path: Rel, version: String },
    /// Recorded before hashes were, still as saved: its hash now.
    Hash { path: Rel, hash: String },
    /// A place's folder, on your disk.
    Folder { path: Rel, id: u64 },
}

impl Op {
    /// Whether it changes something on ILIAS.
    pub fn writes(&self) -> bool {
        matches!(
            self,
            Op::Upload { .. }
                | Op::Replace { .. }
                | Op::Rename { .. }
                | Op::MkdirRemote { .. }
                | Op::TrashRemote { .. }
        )
    }

    /// The path it is about (for a download, where it goes).
    pub fn path(&self) -> &str {
        match self {
            Op::Download { dir, replace, .. } => replace.as_deref().unwrap_or(dir),
            Op::Upload { path, .. }
            | Op::Replace { path, .. }
            | Op::Rename { path, .. }
            | Op::TrashRemote { path, .. }
            | Op::TrashLocal { path }
            | Op::Conflict { path, .. }
            | Op::Keep { path, .. } => path,
            Op::MkdirRemote { dir, .. } => dir,
        }
    }
}
