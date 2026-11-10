use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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
    let dir = std::env::temp_dir().join(format!("ilias-tui-push-{name}-{}", std::process::id()));
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

/// Folder 89 listing file 120 as `title` ("Notes" until renamed), with Versions in its
/// menu, the Versions tab with "Replace File", and that form (markup of the local ILIAS 10,
/// shortened). Saving the form renames the file to the title sent; the fake can't read it,
/// so it takes `renamed`.
fn rename_ilias(renamed: &'static str) -> Arc<FakeIlias> {
    let saved = AtomicBool::new(false);
    Arc::new(FakeIlias::new(move |url| {
        let query = url.query().unwrap_or_default();
        if query.contains("cmd=upload&") {
            return reply(url, r#"{"status":1,"file_id":"r-1"}"#);
        }
        if query.contains("fallbackCmd=createReplacingVersion") {
            saved.store(true, Ordering::SeqCst);
            return reply(url, "<p>saved</p>");
        }
        if query.contains("cmd=addReplacingVersion") {
            return reply(
                url,
                r#"<form class="c-form" enctype="multipart/form-data" action="ilias.php?baseClass=ilrepositorygui&amp;cmdClass=ilFileVersionsGUI&amp;cmd=post&amp;fallbackCmd=createReplacingVersion&amp;ref_id=120&amp;rtoken=t" method="post">
                <input type="text" value="Notes.md" name="form/input_0/input_1" /><textarea name="form/input_0/input_2"></textarea>
                <template><input type="hidden" name="form/input_0/input_3[input_0][]" value="" /></template></form>
                <script>il.UI.Input.File.init('il_ui_fw_1', 'ilias.php?baseClass=ilrepositorygui&cmdClass=ilFileVersionsUploadHandlerGUI&cmd=upload&ref_id=120&upload_mode=replace&cmdMode=asynch', 'x', 'file_id', 0, 1, 1000, '', false);</script>"#,
            );
        }
        let done = saved.load(Ordering::SeqCst);
        if query.contains("cmdClass=ilFileVersionsGUI") {
            let version = if done { 2 } else { 1 };
            return reply(
                url,
                &format!(
                    r#"<button class="btn btn-default" data-action="ilias.php?baseClass=ilrepositorygui&amp;cmdClass=ilFileVersionsGUI&amp;cmd=addReplacingVersion&amp;ref_id=120">Replace File</button>
                    <table id="ilFileVersionsTableGUI"><tbody><tr><td><input type="checkbox" name="hist_id[]" value="{version}" /></td><td> {version} </td></tr></tbody></table>"#
                ),
            );
        }
        let title = if done { renamed } else { "Notes" };
        reply(
            url,
            &format!(
                r#"<div id="il_center_col"><div class="ilContainerListItemOuter"><a class="il_ContainerItemTitle" href="ilias.php?cmdClass=ilObjFileGUI&amp;cmd=sendfile&amp;ref_id=120">{title}</a>
                <div class="il_ItemProperties"><span class="il_ItemProperty">md</span><span class="il_ItemProperty">4 Bytes</span></div>
                <ul><li><button class="btn btn-link" data-action="ilias.php?baseClass=ilrepositorygui&amp;cmdClass=ilFileVersionsGUI&amp;ref_id=120">Versions</button></li></ul></div></div>"#
            ),
        )
    }))
}

#[test]
fn a_rename_renames_the_file_on_ilias_through_replace_file() {
    let mut s = setup("rename");
    // Saved as "Notes.md" (file 120), renamed by you to "Notizen.md".
    let old = s.dir.join("Notes.md");
    std::fs::write(&old, "mine").unwrap();
    s.downloads
        .record(120, old.clone(), "4 Bytes".into())
        .unwrap();
    std::fs::remove_file(&old).unwrap();
    let fake = rename_ilias("Notizen");
    let client = Client::with_pages(base(), fake.clone());
    let notes = Item {
        kind: Kind::File,
        id: Id::Ref(120),
        title: "Notes".into(),
        ..Item::default()
    };
    let rename = vec![Change::Rename {
        place: folder(),
        file: notes.clone(),
        from: old.clone(),
        path: s.file.clone(),
        base: "4 Bytes".into(),
    }];
    let pushed = push(&client, &rename, &mut s.downloads, &mut s.journal, |_| {});
    assert_eq!(pushed.renamed, [s.file.clone()], "{pushed:?}");
    let posts = fake.posts(&base());
    assert_eq!(posts.len(), 2, "{posts:?}");
    assert!(posts[0].ends_with("upload_mode=replace&cmdMode=asynch <- file[0]=@Notizen.md"));
    // The new name as the title, not ILIAS' old one.
    assert!(
        posts[1].ends_with("<- form/input_0/input_1=Notizen.md&form/input_0/input_2=&form/input_0/input_3[input_0][]=r-1"),
        "{}",
        posts[1]
    );
    // In sync under the new name.
    assert_eq!(s.downloads.entry(120).unwrap().path, s.file);
    assert_eq!(journal(&s)[0]["action"], "rename");

    // The old name back on your disk: not renamed.
    let mut s = setup("rename-back");
    std::fs::write(&old, "mine").unwrap();
    let fake = rename_ilias("Notizen");
    let client = Client::with_pages(base(), fake.clone());
    let rename = vec![Change::Rename {
        place: folder(),
        file: notes,
        from: old.clone(),
        path: s.file.clone(),
        base: "4 Bytes".into(),
    }];
    let pushed = push(&client, &rename, &mut s.downloads, &mut s.journal, |_| {});
    assert!(
        pushed.conflicts[0].contains("old name is back"),
        "{pushed:?}"
    );
    assert!(fake.posts(&base()).is_empty());
    let _ = std::fs::remove_file(&old);
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
fn a_file_changed_on_ilias_since_the_plan_is_not_replaced() {
    let mut s = setup("version-since");
    // Planned from version "1 Byte"; ILIAS lists "Notizen" (140) with "4 Bytes" now.
    let fake = ilias(UPLOAD, &[("Notizen", 140)]);
    let client = Client::with_pages(base(), fake.clone());
    let file = Item {
        kind: Kind::File,
        id: Id::Ref(140),
        title: "Notizen".into(),
        ..Item::default()
    };
    let edited = vec![Change::Replace {
        place: folder(),
        file: file.clone(),
        path: s.file.clone(),
        base: "1 Byte".into(),
    }];
    let pushed = push(&client, &edited, &mut s.downloads, &mut s.journal, |_| {});
    assert_eq!(pushed.conflicts.len(), 1, "{pushed:?}");
    assert!(pushed.conflicts[0].contains("changed on ILIAS since the plan"));
    assert!(fake.posts(&base()).is_empty());
    // As planned, but ILIAS offers no Versions for it here: it fails, nothing sent.
    let unchanged = vec![Change::Replace {
        place: folder(),
        file,
        path: s.file.clone(),
        base: "4 Bytes".into(),
    }];
    let pushed = push(
        &client,
        &unchanged,
        &mut s.downloads,
        &mut s.journal,
        |_| {},
    );
    assert!(
        pushed.failed[0].contains("doesn't let you change"),
        "{pushed:?}"
    );
    assert!(fake.posts(&base()).is_empty());
    assert_eq!(journal(&s)[0]["action"], "replace");
    let _ = std::fs::remove_dir_all(&s.dir);
}

const DELETE: &str = "ilias.php?baseClass=ilrepositorygui&cmdNode=wq:mp&cmdClass=ilObjFolderGUI&cmd=delete&ref_id=89&item_ref_id=95";
const CONFIRMED: &str = "ilias.php?baseClass=ilrepositorygui&cmdNode=wq:mp&cmdClass=ilObjFolderGUI&cmd=post&fallbackCmd=confirmedDelete&ref_id=89&id=95&rtoken=t";

/// Folder 89 listing file 95 "Skript" at `size` with Delete in its menu (until it is
/// deleted), and ILIAS' question `question` for it (markup of the local ILIAS 10).
fn trash_ilias(size: &'static str, question: &'static str) -> Arc<FakeIlias> {
    let gone = AtomicBool::new(false);
    Arc::new(FakeIlias::new(move |url| {
        if url.as_str().contains("fallbackCmd=confirmedDelete") {
            gone.store(true, Ordering::SeqCst);
        }
        if url.as_str().contains("cmd=delete&") {
            return reply(
                url,
                &format!(
                    r#"<dialog class="c-modal"><div class="modal-body">
                    <div class="alert alert-warning" role="status">{question}</div>
                    <form class="c-form" action="{}" method="post"><input type="hidden" name="form/input_0" value="95" /></form>
                    </div></dialog>"#,
                    CONFIRMED.replace('&', "&amp;")
                ),
            );
        }
        let listing = match gone.load(Ordering::SeqCst) {
            true => String::new(),
            false => format!(
                r#"<div class="ilContainerListItemOuter"><a class="il_ContainerItemTitle" href="ilias.php?cmdClass=ilObjFileGUI&amp;cmd=sendfile&amp;ref_id=95">Skript</a>
                <div class="il_ItemProperties"><span class="il_ItemProperty">pdf</span><span class="il_ItemProperty">{size}</span></div>
                <ul><li><button class="btn btn-link" data-action="{}">Delete</button></li></ul></div>"#,
                DELETE.replace('&', "&amp;")
            ),
        };
        reply(url, &format!(r#"<div id="il_center_col">{listing}</div>"#))
    }))
}

const ASKED: &str = "Are you sure that you want to delete the following?";

fn skript_trash(s: &Setup, base: &str) -> Vec<Change> {
    vec![Change::Trash {
        place: folder(),
        file: Item {
            kind: Kind::File,
            id: Id::Ref(95),
            title: "Skript".into(),
            ..Item::default()
        },
        path: s.dir.join("Skript.pdf"),
        base: base.into(),
    }]
}

#[test]
fn moves_a_ticked_deletion_into_the_trash_and_journals_it() {
    let mut s = setup("trash");
    let fake = trash_ilias("1.2 MB", ASKED);
    let client = Client::with_pages(base(), fake.clone());
    let changes = skript_trash(&s, "1.2 MB");
    let pushed = push(&client, &changes, &mut s.downloads, &mut s.journal, |_| {});
    assert_eq!(pushed.trashed, [s.dir.join("Skript.pdf")], "{pushed:?}");
    assert_eq!(
        fake.posts(&base()),
        [format!("{CONFIRMED} <- form/input_0=95")]
    );
    let lines = journal(&s);
    assert_eq!(lines[0]["action"], "trash");
    assert_eq!(lines[0]["result"], "ok: file 95 Skript in the ILIAS trash");
    let _ = std::fs::remove_dir_all(&s.dir);
}

#[test]
fn a_dry_run_trashes_nothing() {
    let mut s = setup("trash-dry");
    let fake = trash_ilias("1.2 MB", ASKED);
    let client = Client::with_pages(base(), fake.clone());
    client.set_dry_run(true);
    let changes = skript_trash(&s, "1.2 MB");
    let pushed = push(&client, &changes, &mut s.downloads, &mut s.journal, |_| {});
    assert!(pushed.dry_run && pushed.trashed.is_empty(), "{pushed:?}");
    assert!(fake.posts(&base()).is_empty());
    assert!(journal(&s).is_empty());
    let _ = std::fs::remove_dir_all(&s.dir);
}

#[test]
fn trashes_nothing_for_good_nor_anything_changed_since_the_plan() {
    let mut s = setup("trash-refuse");
    // No trash on this ILIAS: it would be gone for good.
    let warned = "Are you sure that you want to delete the following? (WARNING: The selected object(s) will be irretrievably deleted from the system.)";
    let fake = trash_ilias("1.2 MB", warned);
    let client = Client::with_pages(base(), fake.clone());
    let changes = skript_trash(&s, "1.2 MB");
    let pushed = push(&client, &changes, &mut s.downloads, &mut s.journal, |_| {});
    assert!(pushed.failed[0].contains("for good"), "{pushed:?}");
    assert!(fake.posts(&base()).is_empty());
    // Changed on ILIAS since the plan.
    let fake = trash_ilias("1.3 MB", ASKED);
    let client = Client::with_pages(base(), fake.clone());
    let pushed = push(&client, &changes, &mut s.downloads, &mut s.journal, |_| {});
    assert!(
        pushed.conflicts[0].contains("changed on ILIAS since the plan"),
        "{pushed:?}"
    );
    assert!(fake.posts(&base()).is_empty());
    // Back on your disk.
    std::fs::write(s.dir.join("Skript.pdf"), "restored").unwrap();
    let fake = trash_ilias("1.2 MB", ASKED);
    let client = Client::with_pages(base(), fake.clone());
    let pushed = push(&client, &changes, &mut s.downloads, &mut s.journal, |_| {});
    assert!(
        pushed.conflicts[0].contains("back on your disk"),
        "{pushed:?}"
    );
    assert!(fake.posts(&base()).is_empty());
    let _ = std::fs::remove_dir_all(&s.dir);
}

/// An ILIAS where every folder (89 to start with) offers "Add New Item → Folder" and
/// "Upload Files" (markup of the local ILIAS 10, shortened). The `n`th folder saved is
/// `folders[n]` (ref id 150 + n) in the folder the form was for, the `n`th upload
/// finished is `files[n]` (ref id 160 + n). `taken` is listed in 89 from the start.
fn tree_ilias(
    folders: &'static [&'static str],
    files: &'static [&'static str],
    taken: Option<&'static str>,
) -> Arc<FakeIlias> {
    use std::sync::Mutex;
    // (parent, kind, ref id, title)
    let items = Mutex::new(Vec::<(u64, &str, u64, &str)>::new());
    if let Some(name) = taken {
        items.lock().unwrap().push((89, "fold", 140, name));
    }
    let (saved, finished) = (AtomicUsize::new(0), AtomicUsize::new(0));
    Arc::new(FakeIlias::new(move |url| {
        let ref_id: u64 = url
            .query_pairs()
            .find(|(k, _)| k == "ref_id")
            .and_then(|(_, v)| v.parse().ok())
            .or_else(|| url.path().rsplit('/').next()?.parse().ok())
            .unwrap_or(0);
        let query = url.query().unwrap_or_default();
        if query.contains("cmd=upload&") {
            return reply(url, r#"{"status":1,"file_id":"f"}"#);
        }
        if query.contains("fallbackCmd=save&") {
            let n = saved.fetch_add(1, Ordering::SeqCst);
            items
                .lock()
                .unwrap()
                .push((ref_id, "fold", 150 + n as u64, folders[n]));
            return reply(url, "<p>saved</p>");
        }
        if query.contains("fallbackCmd=uploadFiles") {
            let n = finished.fetch_add(1, Ordering::SeqCst);
            items
                .lock()
                .unwrap()
                .push((ref_id, "file", 160 + n as u64, files[n]));
            return reply(url, "<p>uploaded</p>");
        }
        if query.contains("cmd=create&") {
            let save = format!(
                "ilias.php?baseClass=ilrepositorygui&amp;cmdClass=ilObjFolderGUI&amp;cmd=post&amp;fallbackCmd=save&amp;ref_id={ref_id}&amp;new_type=fold&amp;rtoken=t"
            );
            return reply(
                url,
                &format!(
                    r#"<form class="c-form" action="{save}" method="post"><input type="text" name="form/input_0/input_1" /><textarea name="form/input_0/input_2"></textarea></form>"#
                ),
            );
        }
        let listing: String = items
            .lock()
            .unwrap()
            .iter()
            .filter(|(parent, ..)| *parent == ref_id)
            .map(|(_, kind, id, title)| {
                let href = match *kind {
                    "fold" => format!("goto.php/fold/{id}"),
                    _ => format!("ilias.php?cmdClass=ilObjFileGUI&amp;cmd=sendfile&amp;ref_id={id}"),
                };
                format!(r#"<div class="ilContainerListItemOuter"><a class="il_ContainerItemTitle" href="{href}">{title}</a></div>"#)
            })
            .collect();
        let finish = format!(
            "ilias.php?baseClass=ilrepositorygui&amp;cmdClass=ilObjFileGUI&amp;cmd=post&amp;fallbackCmd=uploadFiles&amp;ref_id={ref_id}&amp;new_type=file&amp;rtoken=t"
        );
        let upload = format!(
            "ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjFileUploadHandlerGUI&cmd=upload&ref_id={ref_id}&cmdMode=asynch"
        );
        let create = format!(
            "https://ilias.example/ilias/ilias.php?baseClass=ilrepositorygui&amp;cmd=create&amp;ref_id={ref_id}&amp;new_type=fold"
        );
        reply(
            url,
            &format!(
                r#"<a class="il-link link-bulky" href="{create}">Folder</a><div id="il_center_col">{listing}</div>
                <form enctype="multipart/form-data" action="{finish}" method="post"><template>
                <input type="text" name="form/input_0[input_1][]" /><textarea name="form/input_0[input_2][]"></textarea>
                <input type="hidden" name="form/input_0[input_3][]" value="" /></template></form>
                <script>il.UI.Input.File.init('il_ui_fw_1', '{upload}', 'x', 'file_id', 0, 100, 1000, '', false);</script>"#
            ),
        )
    }))
}

/// `Eigenes/` with `a.md`, and `Eigenes/Woche 1/` with `b.md`, as the plan has them.
fn new_folder_change(s: &Setup) -> Vec<Change> {
    let dir = s.dir.join("Eigenes");
    std::fs::create_dir_all(dir.join("Woche 1")).unwrap();
    std::fs::write(dir.join("a.md"), "a").unwrap();
    std::fs::write(dir.join("Woche 1/b.md"), "b").unwrap();
    vec![Change::NewFolder {
        place: folder(),
        contents: vec![
            dir.join("Woche 1"),
            dir.join("Woche 1/b.md"),
            dir.join("a.md"),
        ],
        dir,
    }]
}

#[test]
fn creates_new_folders_with_what_is_in_them_and_records_their_files() {
    let mut s = setup("folders");
    let fake = tree_ilias(&["Eigenes", "Woche 1"], &["a", "b"], None);
    let client = Client::with_pages(base(), fake.clone());
    let changes = new_folder_change(&s);
    let pushed = push(&client, &changes, &mut s.downloads, &mut s.journal, |_| {});
    assert_eq!(
        pushed.folders,
        [s.dir.join("Eigenes"), s.dir.join("Eigenes/Woche 1")],
        "{pushed:?}"
    );
    assert_eq!(pushed.uploaded.len(), 2, "{pushed:?}");
    let posts: Vec<String> = fake
        .posts(&base())
        .into_iter()
        .filter(|p| !p.contains("cmd=upload&"))
        .collect();
    assert_eq!(
        posts,
        [
            // Eigenes in folder 89, Woche 1 in Eigenes (150), then each folder's files.
            "ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjFolderGUI&cmd=post&fallbackCmd=save&ref_id=89&new_type=fold&rtoken=t <- form/input_0/input_1=Eigenes&form/input_0/input_2=",
            "ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjFolderGUI&cmd=post&fallbackCmd=save&ref_id=150&new_type=fold&rtoken=t <- form/input_0/input_1=Woche 1&form/input_0/input_2=",
            "ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjFileGUI&cmd=post&fallbackCmd=uploadFiles&ref_id=150&new_type=file&rtoken=t <- form/input_0[input_1][]=a.md&form/input_0[input_2][]=&form/input_0[input_3][]=f",
            "ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjFileGUI&cmd=post&fallbackCmd=uploadFiles&ref_id=151&new_type=file&rtoken=t <- form/input_0[input_1][]=b.md&form/input_0[input_2][]=&form/input_0[input_3][]=f",
        ]
    );
    // In sync afterwards.
    assert_eq!(
        s.downloads.entry(160).unwrap().path,
        s.dir.join("Eigenes/a.md")
    );
    assert_eq!(
        s.downloads.entry(161).unwrap().path,
        s.dir.join("Eigenes/Woche 1/b.md")
    );
    let actions: Vec<String> = journal(&s)
        .iter()
        .map(|l| l["action"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(actions, ["new folder", "new folder", "upload", "upload"]);
    let _ = std::fs::remove_dir_all(&s.dir);
}

#[test]
fn a_dry_run_creates_no_folder_and_a_name_taken_since_the_plan_is_a_conflict() {
    let mut s = setup("folders-dry");
    let fake = tree_ilias(&["Eigenes", "Woche 1"], &["a", "b"], None);
    let client = Client::with_pages(base(), fake.clone());
    client.set_dry_run(true);
    let changes = new_folder_change(&s);
    let pushed = push(&client, &changes, &mut s.downloads, &mut s.journal, |_| {});
    assert!(pushed.dry_run && pushed.folders.is_empty(), "{pushed:?}");
    assert!(fake.posts(&base()).is_empty());

    let fake = tree_ilias(&["Eigenes", "Woche 1"], &["a", "b"], Some("Eigenes"));
    let client = Client::with_pages(base(), fake.clone());
    let pushed = push(&client, &changes, &mut s.downloads, &mut s.journal, |_| {});
    assert!(pushed.conflicts[0].contains("since the plan"), "{pushed:?}");
    assert!(fake.posts(&base()).is_empty());
    assert!(journal(&s).is_empty());
    let _ = std::fs::remove_dir_all(&s.dir);
}
