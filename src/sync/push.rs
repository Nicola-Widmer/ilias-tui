//! Two-way sync's writes: sends the changes of a [`Review`](super::Review) you agreed to,
//! through the write module's own functions (`Client::upload_files`, …). Before each place
//! is written to, its page is read again; anything that changed there since the plan
//! becomes a conflict and is not sent. Every write goes into the [`Journal`], and the base
//! is updated after each one, so the next sync sees the file as in sync.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{Change, journal::Journal, version};
use crate::downloads::Downloads;
use crate::ilias::write::megabytes;
use crate::ilias::{Client, Item, Kind, lists_as};

/// More files than this in one run, and sync asks once more before sending.
pub const MAX_FILES: usize = 20;
/// More than this share of a folder's files changed (new versions, the trash), and sync
/// asks once more.
pub const MAX_SHARE: f64 = 0.3;

/// What a push did.
#[derive(Debug, Default, PartialEq)]
pub struct Pushed {
    pub uploaded: Vec<PathBuf>,
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

/// The question before sending `changes`: what goes where, with sizes.
pub fn question(changes: &[Change], dir: &Path) -> String {
    let short = |path: &Path| path.strip_prefix(dir).unwrap_or(path).display().to_string();
    let files: Vec<String> = changes
        .iter()
        .filter_map(|c| match c {
            Change::Upload { path, .. } => {
                let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                Some(format!("{} ({})", short(path), megabytes(size)))
            }
            _ => None,
        })
        .collect();
    let mut shown = files.iter().take(8).cloned().collect::<Vec<_>>().join(", ");
    if files.len() > 8 {
        shown.push_str(&format!(" and {} more (see the report)", files.len() - 8));
    }
    format!(
        "Upload {} new file{} into their ILIAS folders? Everyone who can open them there sees them: {shown}",
        files.len(),
        if files.len() == 1 { "" } else { "s" },
    )
}

/// Why `changes` are more than usual (more than [`MAX_FILES`] files, or more than
/// [`MAX_SHARE`] of a folder's files on ILIAS replaced or trashed), for a second question;
/// `listed` is how many files ILIAS lists in each place (by key).
pub fn brake(changes: &[Change], listed: &HashMap<u64, usize>) -> Option<String> {
    let count = |f: fn(&Change) -> bool| changes.iter().filter(|c| f(c)).count();
    let uploads = count(|c| matches!(c, Change::Upload { .. } | Change::NewVersion { .. }));
    let trash = count(|c| matches!(c, Change::Trash { .. }));
    if uploads > MAX_FILES || trash > MAX_FILES {
        return Some(format!(
            "That is a lot at once: {uploads} files to upload and {trash} to move to the trash \
             (more than {MAX_FILES}). If you moved or renamed a folder, say no. Send them all?"
        ));
    }
    let mut touched: HashMap<u64, (&Item, usize)> = HashMap::new();
    for change in changes {
        if let Change::NewVersion { place, .. } | Change::Trash { place, .. } = change {
            touched.entry(place.key()).or_insert((place, 0)).1 += 1;
        }
    }
    let mut places: Vec<_> = touched.into_values().collect();
    places.sort_by_key(|(place, _)| place.title.clone());
    places.into_iter().find_map(|(place, n)| {
        let files = listed.get(&place.key()).copied().unwrap_or(0);
        (files > 0 && n as f64 > MAX_SHARE * files as f64).then(|| {
            format!(
                "That is a lot for one folder: {n} of the {files} files in {} would be replaced \
                 or moved to the trash. Send them all?",
                place.title
            )
        })
    })
}

/// Notes a write that failed, in the journal and the result.
fn fail(journal: &mut Journal, pushed: &mut Pushed, place: &Item, path: &Path, why: String) {
    let _ = journal.write("upload", path, &object(place), &format!("failed: {why}"));
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
        // The re-check: ILIAS as it is now, not as planned.
        let page = match client.page(place, None) {
            Ok(page) => page,
            Err(e) => {
                for path in paths {
                    fail(journal, &mut pushed, place, path, format!("{e:#}"));
                }
                continue;
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
                    &mut pushed,
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
            continue;
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
                        Some(item) => {
                            match downloads.record(item.key(), path.clone(), version(item)) {
                                Ok(()) => format!("ok: {}", object(item)),
                                Err(e) => format!("ok, not recorded: {e:#}"),
                            }
                        }
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
                    fail(journal, &mut pushed, place, path, format!("{e:#}"));
                }
            }
        }
    }
    pushed
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;
    use crate::ilias::Id;
    use crate::ilias::pages::{FakeIlias, reply};
    use pretty_assertions::assert_eq;

    fn base() -> url::Url {
        url::Url::parse("https://ilias.example/ilias/").unwrap()
    }

    fn folder() -> Item {
        Item {
            kind: Kind::Folder,
            id: Id::Ref(89),
            title: "Unterlagen".into(),
            url: "https://ilias.example/ilias/goto.php/fold/89".into(),
            ..Item::default()
        }
    }

    const FINISH: &str = "ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjFileGUI&cmd=post&fallbackCmd=uploadFiles&ref_id=89&new_type=file&rtoken=t";
    const UPLOAD: &str = "ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjFileUploadHandlerGUI&cmd=upload&ref_id=89&cmdMode=asynch";

    /// Folder 89 with its "Upload Files" form (markup of the local ILIAS 10, shortened),
    /// files posting to `upload`, listing `listed` (title, ref id).
    fn page(upload: &str, listed: &[(&str, u64)]) -> String {
        let items: String = listed
            .iter()
            .map(|(title, id)| {
                format!(
                    r#"<div class="ilContainerListItemOuter"><a class="il_ContainerItemTitle" href="ilias.php?cmdClass=ilObjFileGUI&amp;cmd=sendfile&amp;ref_id={id}">{title}</a>
                    <div class="il_ItemProperties"><span class="il_ItemProperty">md</span><span class="il_ItemProperty">4 Bytes</span></div></div>"#
                )
            })
            .collect();
        format!(
            r#"<div id="il_center_col">{items}</div>
            <form enctype="multipart/form-data" action="{}" method="post">
              <fieldset class="c-input" data-il-ui-component="file-field-input"><template>
                <input type="text" name="form/input_0[input_1][]" />
                <textarea name="form/input_0[input_2][]"></textarea>
                <input type="hidden" name="form/input_0[input_3][]" value="" />
              </template></fieldset></form>
            <script>il.UI.Input.File.init('il_ui_fw_1', '{upload}',
                'ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjFileUploadHandlerGUI&cmd=remove&ref_id=89&cmdMode=asynch',
                'file_id', 0, 100, 1000, '', false);</script>"#,
            FINISH.replace('&', "&amp;")
        )
    }

    /// ILIAS whose folder lists `before` until the upload is finished, then also the new
    /// file "Notizen" (ref id 140).
    fn ilias(upload: &'static str, before: &'static [(&'static str, u64)]) -> Arc<FakeIlias> {
        let finished = AtomicBool::new(false);
        Arc::new(FakeIlias::new(move |url| {
            if url.as_str().contains("cmd=upload&") {
                return reply(url, r#"{"status":1,"file_id":"id-0"}"#);
            }
            if url.as_str().contains("fallbackCmd=uploadFiles") {
                finished.store(true, Ordering::SeqCst);
            }
            let mut listed = before.to_vec();
            if finished.load(Ordering::SeqCst) {
                listed.push(("Notizen", 140));
            }
            reply(url, &page(upload, &listed))
        }))
    }

    struct Setup {
        dir: PathBuf,
        file: PathBuf,
        downloads: Downloads,
        journal: Journal,
    }

    fn setup(name: &str) -> Setup {
        let dir =
            std::env::temp_dir().join(format!("ilias-tui-push-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Notizen.md");
        std::fs::write(&file, "mine").unwrap();
        Setup {
            downloads: Downloads::in_file(dir.join("index.json")),
            journal: Journal::in_file(dir.join("journal.jsonl")),
            file,
            dir,
        }
    }

    fn upload(path: &Path) -> Vec<Change> {
        vec![Change::Upload {
            place: folder(),
            path: path.to_path_buf(),
            renamed_from: None,
        }]
    }

    fn journal(s: &Setup) -> Vec<serde_json::Value> {
        std::fs::read_to_string(s.dir.join("journal.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn uploads_new_files_records_them_as_in_sync_and_journals_it() {
        let mut s = setup("ok");
        let fake = ilias(UPLOAD, &[]);
        let client = Client::with_pages(base(), fake.clone());
        let pushed = push(
            &client,
            &upload(&s.file),
            &mut s.downloads,
            &mut s.journal,
            |_| {},
        );
        assert_eq!(pushed.uploaded, [s.file.clone()], "{pushed:?}");
        assert_eq!(
            fake.posts(&base()),
            [
                format!("{UPLOAD} <- file[0]=@Notizen.md"),
                format!(
                    "{FINISH} <- form/input_0[input_1][]=Notizen.md&form/input_0[input_2][]=&form/input_0[input_3][]=id-0"
                ),
            ]
        );
        // The base: the next sync sees ILIAS' file 140 as this very copy, untouched.
        let saved = s.downloads.entry(140).unwrap();
        assert_eq!(saved.path, s.file);
        assert_eq!(saved.version, "4 Bytes");
        assert_eq!(saved.local(), crate::downloads::Local::Untouched);
        let lines = journal(&s);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["action"], "upload");
        assert_eq!(lines[0]["ilias"], "fold 89 Unterlagen");
        assert_eq!(lines[0]["result"], "ok: file 140 Notizen");
        let _ = std::fs::remove_dir_all(&s.dir);
    }

    #[test]
    fn an_uploaded_rename_keeps_the_old_name_out_of_the_trash_for_good() {
        let mut s = setup("rename");
        // Saved as "Notes.md" (file 120), renamed by you to "Notizen.md".
        let old = s.dir.join("Notes.md");
        std::fs::write(&old, "mine").unwrap();
        s.downloads.record(120, old.clone(), "v1".into()).unwrap();
        std::fs::remove_file(&old).unwrap();
        let client = Client::with_pages(base(), ilias(UPLOAD, &[("Notes", 120)]));
        let rename = vec![Change::Upload {
            place: folder(),
            path: s.file.clone(),
            renamed_from: Some(old),
        }];
        let pushed = push(&client, &rename, &mut s.downloads, &mut s.journal, |_| {});
        assert_eq!(pushed.uploaded, [s.file.clone()], "{pushed:?}");
        assert_eq!(
            s.downloads.entry(120).unwrap().moved_to,
            Some(s.file.clone())
        );
        let _ = std::fs::remove_dir_all(&s.dir);
    }

    #[test]
    fn a_dry_run_uploads_and_journals_nothing() {
        let mut s = setup("dry");
        let fake = ilias(UPLOAD, &[]);
        let client = Client::with_pages(base(), fake.clone());
        client.set_dry_run(true);
        let pushed = push(
            &client,
            &upload(&s.file),
            &mut s.downloads,
            &mut s.journal,
            |_| {},
        );
        assert!(pushed.dry_run && pushed.uploaded.is_empty(), "{pushed:?}");
        assert!(fake.posts(&base()).is_empty());
        assert!(s.downloads.entry(140).is_none());
        assert!(journal(&s).is_empty());
        let _ = std::fs::remove_dir_all(&s.dir);
    }

    #[test]
    fn a_file_of_the_same_name_since_the_plan_is_a_conflict() {
        let mut s = setup("since");
        // Someone uploaded a "Notizen" after the plan was made.
        let fake = ilias(UPLOAD, &[("Notizen", 141)]);
        let client = Client::with_pages(base(), fake.clone());
        let pushed = push(
            &client,
            &upload(&s.file),
            &mut s.downloads,
            &mut s.journal,
            |_| {},
        );
        assert_eq!(pushed.conflicts.len(), 1, "{pushed:?}");
        assert!(fake.posts(&base()).is_empty());
        assert!(journal(&s).is_empty(), "nothing was sent");
        let _ = std::fs::remove_dir_all(&s.dir);
    }

    #[test]
    fn refuses_a_form_for_another_place_and_journals_the_failure() {
        let mut s = setup("refuse");
        let other = "ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjFileUploadHandlerGUI&cmd=upload&ref_id=90&cmdMode=asynch";
        let fake = ilias(other, &[]);
        let client = Client::with_pages(base(), fake.clone());
        let pushed = push(
            &client,
            &upload(&s.file),
            &mut s.downloads,
            &mut s.journal,
            |_| {},
        );
        assert_eq!(pushed.failed.len(), 1, "{pushed:?}");
        assert!(fake.posts(&base()).is_empty());
        assert!(
            journal(&s)[0]["result"]
                .as_str()
                .unwrap()
                .starts_with("failed: ")
        );
        // A file gone from disk since the plan isn't sent either.
        std::fs::remove_file(&s.file).unwrap();
        let fake = ilias(UPLOAD, &[]);
        let client = Client::with_pages(base(), fake.clone());
        let pushed = push(
            &client,
            &upload(&s.file),
            &mut s.downloads,
            &mut s.journal,
            |_| {},
        );
        assert!(
            pushed.failed[0].contains("no longer on your disk"),
            "{pushed:?}"
        );
        assert!(fake.posts(&base()).is_empty());
        let _ = std::fs::remove_dir_all(&s.dir);
    }

    #[test]
    fn asks_once_more_for_more_than_usual() {
        let uploads: Vec<Change> = (0..=MAX_FILES)
            .map(|n| Change::Upload {
                place: folder(),
                path: PathBuf::from(format!("/dl/{n}.pdf")),
                renamed_from: None,
            })
            .collect();
        let listed = HashMap::from([(89, 100)]);
        assert!(brake(&uploads[..MAX_FILES], &listed).is_none());
        assert!(
            brake(&uploads, &listed)
                .unwrap()
                .contains("21 files to upload")
        );
        // 4 of a folder's 10 files replaced or trashed.
        let file = |id| Item {
            kind: Kind::File,
            id: Id::Ref(id),
            ..Item::default()
        };
        let mut changes: Vec<Change> = (1..=3)
            .map(|id| Change::NewVersion {
                place: folder(),
                file: file(id),
                path: PathBuf::from(format!("/dl/{id}.pdf")),
                base: "v1".into(),
            })
            .collect();
        let listed = HashMap::from([(89, 10)]);
        assert!(brake(&changes, &listed).is_none(), "3 of 10");
        changes.push(Change::Trash {
            place: folder(),
            file: file(4),
            path: PathBuf::from("/dl/4.pdf"),
            base: "v1".into(),
        });
        assert!(
            brake(&changes, &listed)
                .unwrap()
                .contains("4 of the 10 files in Unterlagen")
        );
    }

    #[test]
    fn the_question_names_each_file_and_its_size() {
        let s = setup("question");
        let question = question(&upload(&s.file), &s.dir);
        assert_eq!(
            question,
            "Upload 1 new file into their ILIAS folders? Everyone who can open them there sees them: Notizen.md (0.0 MB)"
        );
        let _ = std::fs::remove_dir_all(&s.dir);
    }
}
