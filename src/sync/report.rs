//! What a sync shows, built from its ops by their variant: the status line, the report's
//! sections, and the review (what `Enter` sends, the proposals, what goes to your Trash).

use super::model::Rel;
use super::ops::{Op, Reason, Why};
use super::run::Done;
use crate::ilias::Kind;

/// A section of the report, in the order shown. The report shows each with its own icon
/// and colour.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub enum Section {
    /// Changed on ILIAS and edited by you: ILIAS' copy saved beside yours.
    Beside,
    Upload,
    Replace,
    Rename,
    /// Deleted by you: to the ILIAS trash, ticked unless you untick it.
    Proposals,
    NewFolders,
    Conflicts,
    GoneEdited,
    /// Gone on ILIAS, unchanged on your disk: to your Trash, after asking.
    TrashLocal,
    NotAllowed,
    CantTell,
    TooBigToSend {
        max_mb: f64,
    },
    TooBig {
        max_mb: f64,
    },
    /// Changed since the plan, found while sending.
    ChangedSince,
    Failed,
}

impl Section {
    pub fn title(&self) -> String {
        match self {
            Section::Beside => {
                "Changed on ILIAS, kept your edited copy; new version saved as".into()
            }
            Section::Upload => "New on your disk: to upload".into(),
            Section::Replace => {
                "Edited by you: to replace on ILIAS (its earlier versions are deleted)".into()
            }
            Section::Rename => {
                "Renamed by you: to rename on ILIAS (its earlier versions are deleted)".into()
            }
            Section::Proposals => "Deleted by you: to the ILIAS trash (Space unticks)".into(),
            Section::NewFolders => {
                "New folders on your disk: to create on ILIAS, with what is in them".into()
            }
            Section::Conflicts => "Conflicts: nothing sent, both copies kept".into(),
            Section::GoneEdited => "Gone on ILIAS, edited by you: kept on your disk".into(),
            Section::TrashLocal => {
                "Gone on ILIAS, unchanged on your disk: to your Trash (asks)".into()
            }
            Section::NotAllowed => {
                "Only on your disk: ILIAS doesn't let you upload or change it there".into()
            }
            Section::CantTell => {
                "Changed on your disk, saved by an older version: can't tell if edited, not sent"
                    .into()
            }
            Section::TooBigToSend { max_mb } => format!("Not sent: bigger than {max_mb} MB"),
            Section::TooBig { max_mb } => format!("Skipped: bigger than {max_mb} MB"),
            Section::ChangedSince => "Changed on ILIAS since the plan: nothing sent".into(),
            Section::Failed => "Failed".into(),
        }
    }
}

pub type Report = Vec<(Section, Vec<String>)>;

/// What a two-way sync would send, for the review.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Review {
    /// What `Enter` sends besides the ticked proposals: uploads, new folders, replaced and
    /// renamed files.
    pub send: Vec<Op>,
    /// Deletions proposed for the ILIAS trash, ticked unless you untick them.
    pub proposals: Vec<Op>,
    /// Untouched files gone on ILIAS, for your Trash after asking.
    pub trash_local: Vec<Rel>,
    /// Dry run: nothing is sent, the review is a plain report.
    pub dry_run: bool,
}

impl Review {
    pub fn new(ops: &[Op], dry_run: bool) -> Self {
        let mut review = Self {
            dry_run,
            ..Self::default()
        };
        for op in ops {
            match op {
                Op::TrashRemote { .. } => review.proposals.push(op.clone()),
                Op::TrashLocal { path } => review.trash_local.push(path.clone()),
                op if op.writes() => review.send.push(op.clone()),
                _ => {}
            }
        }
        review
    }
}

/// One line about a conflict.
pub fn conflict(path: &str, why: &Why) -> String {
    match why {
        Why::Both { .. } => {
            format!("{path}: edited by you and changed on ILIAS; ILIAS' copy saved beside")
        }
        Why::SameName => format!(
            "{path}: ILIAS has a file of this name here that sync didn't save; kept both, nothing sent"
        ),
        Why::CopyOf(saved) => format!("{path}: a copy of {saved}, which sync saved; not sent"),
        Why::SharedFolder => {
            format!("{path}: several ILIAS folders are saved here, so nothing in it is sent")
        }
        Why::ComesBackAs(name) => {
            format!("{path}: a folder of this name would come back as {name}; not sent")
        }
    }
}

/// How an op is listed in its section.
fn line(op: &Op) -> String {
    match op {
        Op::Upload {
            path,
            from: Some(old),
            ..
        } => format!("{path} (renamed from {old})"),
        Op::Rename { path, from, .. } => format!("{path} (was {from})"),
        Op::TrashRemote { path, item, .. } if item.kind == Kind::Folder => {
            format!("{path}/ (the folder, with what is in it)")
        }
        Op::MkdirRemote { dir, files, .. } => format!("{dir}/ ({} files in it)", files.len()),
        Op::Conflict { path, why } => conflict(path, why),
        op => op.path().to_string(),
    }
}

/// The section an op is listed in before it runs; `None` for what isn't listed.
fn section(op: &Op, max_mb: f64) -> Option<Section> {
    Some(match op {
        Op::Upload { .. } => Section::Upload,
        Op::Replace { .. } => Section::Replace,
        Op::Rename { .. } => Section::Rename,
        Op::TrashRemote { .. } => Section::Proposals,
        Op::MkdirRemote { .. } => Section::NewFolders,
        Op::TrashLocal { .. } => Section::TrashLocal,
        Op::Conflict {
            why: Why::Both { .. },
            ..
        } => return None,
        Op::Conflict { .. } => Section::Conflicts,
        Op::Keep { reason, .. } => match reason {
            Reason::GoneEdited => Section::GoneEdited,
            Reason::NotAllowed => Section::NotAllowed,
            Reason::CantTell => Section::CantTell,
            Reason::TooBigToSend => Section::TooBigToSend { max_mb },
            Reason::TooBig => Section::TooBig { max_mb },
            Reason::Deleted => return None,
        },
        Op::Download { .. } => return None,
    })
}

/// The counts of a sync, for its status line.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub new: usize,
    pub updated: usize,
    pub beside: usize,
    pub unchanged: usize,
    /// Files you deleted, proposed for the ILIAS trash (those in folders included).
    pub trash: usize,
    /// Files you deleted that stay deleted, not sent.
    pub kept: usize,
    pub too_big: usize,
    pub failed: usize,
}

/// A sync: its plan and how the ops it ran (downloads) went.
#[derive(Debug, Default)]
pub struct Summary {
    pub ops: Vec<Op>,
    /// Saved files the same on both sides.
    pub unchanged: usize,
    pub done: Vec<(Op, Done)>,
    /// Ref ids of files that are saved now.
    pub saved: Vec<u64>,
    /// What's new on the pages read, for [`crate::seen::Seen::record`].
    pub unseen: Vec<super::FoundNew>,
    pub review: Review,
    pub two_way: bool,
    /// What was synced, for the status line: `DEMO.101`, `Unterlagen and 2 more`.
    pub places: String,
}

/// Where a download ended up, if not where it was meant to: ILIAS' copy beside yours.
fn beside<'a>(op: &Op, done: &'a Done) -> Option<&'a Rel> {
    match (op, done) {
        (Op::Conflict { .. }, Done::Ok { saved }) => saved.as_ref(),
        (
            Op::Download {
                replace: Some(target),
                ..
            },
            Done::Ok { saved },
        ) => saved.as_ref().filter(|saved| *saved != target),
        _ => None,
    }
}

impl Summary {
    pub fn counts(&self) -> Counts {
        let mut c = Counts {
            unchanged: self.unchanged,
            ..Counts::default()
        };
        for op in &self.ops {
            match op {
                Op::Replace { .. }
                | Op::Keep {
                    reason: Reason::CantTell,
                    ..
                } => c.unchanged += 1,
                Op::Keep {
                    reason: Reason::Deleted,
                    ..
                } => c.kept += 1,
                Op::Keep {
                    reason: Reason::TooBig,
                    ..
                } => c.too_big += 1,
                Op::TrashRemote { files, .. } => c.trash += files,
                _ => {}
            }
        }
        for (op, done) in &self.done {
            match (op, done) {
                (_, Done::Failed(_)) => c.failed += 1,
                _ if beside(op, done).is_some() => c.beside += 1,
                (Op::Download { replace: None, .. }, Done::Ok { .. }) => c.new += 1,
                (Op::Download { .. }, Done::Ok { .. }) => c.updated += 1,
                _ => {}
            }
        }
        c
    }

    /// One line for the status bar.
    pub fn text(&self, max_mb: f64) -> String {
        let c = self.counts();
        let to_send = self.review.send.len() + self.review.proposals.len();
        let kept = match self.two_way {
            true => "deleted by you (ILIAS doesn't let you delete them there)",
            false => "deleted by you (not downloaded again)",
        };
        let parts = [
            (1, format!("{} new", c.new)),
            (1, format!("{} updated", c.updated)),
            (
                c.beside,
                format!("{} edited by you (new version saved beside)", c.beside),
            ),
            (1, format!("{} unchanged", c.unchanged)),
            (
                c.trash,
                format!("{} deleted by you (to the ILIAS trash)", c.trash),
            ),
            (c.kept, format!("{} {kept}", c.kept)),
            (
                to_send,
                format!("{to_send} to send to ILIAS (see the report)"),
            ),
            (
                to_send * usize::from(self.review.dry_run),
                "dry run: nothing will be sent".into(),
            ),
            (c.too_big, format!("{} over {max_mb} MB skipped", c.too_big)),
            (c.failed, format!("{} failed", c.failed)),
        ];
        let parts: Vec<String> = parts
            .into_iter()
            .filter(|(n, _)| *n > 0)
            .map(|(_, p)| p)
            .collect();
        match self.places.as_str() {
            "" => format!("Synced: {}", parts.join(", ")),
            places => format!("Synced {places}: {}", parts.join(", ")),
        }
    }

    /// The files worth a closer look, by section; only the sections with something in
    /// them. The proposals are listed in the review's order.
    pub fn report(&self, max_mb: f64) -> Report {
        let mut lines: Vec<(Section, String)> = Vec::new();
        for (op, done) in &self.done {
            match done {
                Done::Failed(why) => lines.push((Section::Failed, why.clone())),
                _ => lines.extend(beside(op, done).map(|p| (Section::Beside, p.clone()))),
            }
        }
        for op in self
            .ops
            .iter()
            .filter(|op| !matches!(op, Op::TrashRemote { .. }))
        {
            lines.extend(section(op, max_mb).map(|s| (s, line(op))));
        }
        for op in &self.review.proposals {
            lines.push((Section::Proposals, line(op)));
        }
        group(lines)
    }
}

/// Lines by section, in the sections' order.
fn group(mut lines: Vec<(Section, String)>) -> Report {
    // Stable: lines stay in their order within a section.
    lines.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut report: Report = Vec::new();
    for (section, line) in lines {
        match report.last_mut() {
            Some((last, lines)) if *last == section => lines.push(line),
            _ => report.push((section, vec![line])),
        }
    }
    report
}

/// The status line after sending.
pub fn pushed_text(done: &[(Op, Done)]) -> String {
    if !done.is_empty() && done.iter().all(|(_, d)| *d == Done::DryRun) {
        return "Dry run: nothing sent to ILIAS".into();
    }
    let mut n = [0; 7];
    for (op, d) in done {
        let at = match (op, d) {
            (_, Done::Conflict(_)) => 5,
            (_, Done::Failed(_)) => 6,
            (_, Done::DryRun) => continue,
            (Op::Upload { .. }, _) => 0,
            (Op::Replace { .. }, _) => 1,
            (Op::Rename { .. }, _) => 2,
            (Op::MkdirRemote { files, .. }, _) => {
                n[0] += files.len();
                3
            }
            (Op::TrashRemote { .. }, _) => 4,
            _ => continue,
        };
        n[at] += 1;
    }
    let what = [
        "uploaded",
        "replaced",
        "renamed",
        "new folders",
        "moved to the ILIAS trash",
        "changed on ILIAS meanwhile (not sent)",
        "failed",
    ];
    let parts: Vec<String> = (0..what.len())
        .filter(|&i| i == 0 || n[i] > 0)
        .map(|i| format!("{} {}", n[i], what[i]))
        .collect();
    format!("Sent to ILIAS: {}", parts.join(", "))
}

/// What deserves a look after sending.
pub fn pushed_report(done: &[(Op, Done)]) -> Report {
    group(
        (done.iter())
            .filter_map(|(_, d)| match d {
                Done::Conflict(why) => Some((Section::ChangedSince, why.clone())),
                Done::Failed(why) => Some((Section::Failed, why.clone())),
                _ => None,
            })
            .collect(),
    )
}
