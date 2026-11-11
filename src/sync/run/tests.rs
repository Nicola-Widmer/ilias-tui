//! The executor against a fake ILIAS: exactly what is posted, nothing in a dry run, a
//! change since the plan becomes a conflict, and the base is updated after each op.
//! (Downloads bypass the fake, so only their failure is tested here.)

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::*;
use crate::ilias::Id;
use crate::ilias::pages::{FakeIlias, reply};
use crate::sync::model::{Disk, FileEntry};
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

struct Setup {
    dir: PathBuf,
    base: Base,
    journal: Journal,
}

impl Setup {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ilias-tui-run-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Notizen.md"), "mine").unwrap();
        Self {
            base: Base::new(&dir),
            journal: Journal::in_file(dir.join("journal.jsonl")),
            dir,
        }
    }

    fn run(&mut self, client: &Client, ops: &[Op]) -> Vec<Done> {
        let done = run(client, ops, &mut self.base, &mut self.journal, |_| {});
        done.into_iter().map(|(_, done)| done).collect()
    }

    fn journal(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(self.dir.join("journal.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

impl Drop for Setup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn upload(path: &str) -> Vec<Op> {
    vec![Op::Upload {
        place: folder(),
        path: path.into(),
        from: None,
    }]
}

fn file_item(id: u64, title: &str) -> Item {
    Item {
        kind: Kind::File,
        id: Id::Ref(id),
        title: title.into(),
        ..Item::default()
    }
}

#[test]
fn uploads_new_files_records_them_in_the_base_and_journals_it() {
    let mut s = Setup::new("upload");
    let fake = ilias(UPLOAD, &[]);
    let client = Client::with_pages(base(), fake.clone());
    assert_eq!(s.run(&client, &upload("Notizen.md")), [OK]);
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
    let (path, entry) = s.base.find(140).unwrap();
    assert_eq!((path, entry.version.as_str()), ("Notizen.md", "4 Bytes"));
    assert_eq!(entry.now(&s.dir.join("Notizen.md")), Disk::Untouched);
    assert_eq!(s.base.path(140), Some(s.dir.join("Notizen.md")));
    let lines = s.journal();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["action"], "upload");
    assert_eq!(lines[0]["ilias"], "fold 89 Unterlagen");
    assert_eq!(lines[0]["result"], "ok: file 140 Notizen");
}

#[test]
fn a_dry_run_sends_records_and_journals_nothing() {
    let mut s = Setup::new("dry");
    let fake = ilias(UPLOAD, &[]);
    let client = Client::with_pages(base(), fake.clone());
    client.set_dry_run(true);
    assert_eq!(s.run(&client, &upload("Notizen.md")), [Done::DryRun]);
    assert!(fake.posts(&base()).is_empty());
    assert!(s.base.find(140).is_none());
    assert!(s.journal().is_empty());
}

#[test]
fn a_file_of_the_same_name_since_the_plan_is_a_conflict() {
    let mut s = Setup::new("since");
    // Someone uploaded a "Notizen" after the plan was made.
    let fake = ilias(UPLOAD, &[("Notizen", 141)]);
    let client = Client::with_pages(base(), fake.clone());
    let done = s.run(&client, &upload("Notizen.md"));
    assert!(
        matches!(&done[..], [Done::Conflict(why)] if why.contains("since the plan")),
        "{done:?}"
    );
    assert!(fake.posts(&base()).is_empty());
    assert!(s.journal().is_empty(), "nothing was sent");
}

#[test]
fn refuses_a_form_for_another_place_and_a_file_gone_since_the_plan() {
    let mut s = Setup::new("refuse");
    let other = "ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjFileUploadHandlerGUI&cmd=upload&ref_id=90&cmdMode=asynch";
    let fake = ilias(other, &[]);
    let client = Client::with_pages(base(), fake.clone());
    assert!(matches!(
        &s.run(&client, &upload("Notizen.md"))[..],
        [Done::Failed(_)]
    ));
    assert!(fake.posts(&base()).is_empty());
    assert!(
        s.journal()[0]["result"]
            .as_str()
            .unwrap()
            .starts_with("failed: ")
    );
    std::fs::remove_file(s.dir.join("Notizen.md")).unwrap();
    let fake = ilias(UPLOAD, &[]);
    let client = Client::with_pages(base(), fake.clone());
    let done = s.run(&client, &upload("Notizen.md"));
    assert!(matches!(&done[..], [Done::Failed(why)] if why.contains("no longer on your disk")));
    assert!(fake.posts(&base()).is_empty());
}

#[test]
fn a_rename_goes_through_replace_file_and_moves_the_base_entry() {
    let mut s = Setup::new("rename");
    // Saved as "Notes.md" (file 120), renamed by you to "Notizen.md".
    let entry = FileEntry::stamp(&s.dir.join("Notizen.md"), 120, "4 Bytes".into());
    s.base.change(|e| e.record_file("Notes.md", entry)).unwrap();
    let fake = rename_ilias("Notizen");
    let client = Client::with_pages(base(), fake.clone());
    let rename = vec![Op::Rename {
        place: folder(),
        file: file_item(120, "Notes"),
        from: "Notes.md".into(),
        path: "Notizen.md".into(),
        base: "4 Bytes".into(),
    }];
    assert_eq!(s.run(&client, &rename), [OK]);
    let posts = fake.posts(&base());
    assert_eq!(posts.len(), 2, "{posts:?}");
    assert!(posts[0].ends_with("upload_mode=replace&cmdMode=asynch <- file[0]=@Notizen.md"));
    // The new name as the title, not ILIAS' old one.
    assert!(
        posts[1].ends_with("<- form/input_0/input_1=Notizen.md&form/input_0/input_2=&form/input_0/input_3[input_0][]=r-1"),
        "{}",
        posts[1]
    );
    // In sync under the new name only.
    assert_eq!(
        s.base.files().map(|(p, e)| (p, e.id)).collect::<Vec<_>>(),
        [("Notizen.md", 120)]
    );
    assert_eq!(s.journal()[0]["action"], "rename");

    // The old name back on your disk: not renamed.
    let mut s = Setup::new("rename-back");
    std::fs::write(s.dir.join("Notes.md"), "mine").unwrap();
    let fake = rename_ilias("Notizen");
    let client = Client::with_pages(base(), fake.clone());
    let done = s.run(&client, &rename);
    assert!(matches!(&done[..], [Done::Conflict(why)] if why.contains("old name is back")));
    assert!(fake.posts(&base()).is_empty());
}

#[test]
fn a_file_changed_on_ilias_since_the_plan_is_not_replaced() {
    let mut s = Setup::new("replace-since");
    // Planned from version "1 Byte"; ILIAS lists "Notizen" (140) with "4 Bytes" now.
    let fake = ilias(UPLOAD, &[("Notizen", 140)]);
    let client = Client::with_pages(base(), fake.clone());
    let replace = |base: &str| {
        vec![Op::Replace {
            place: folder(),
            file: file_item(140, "Notizen"),
            path: "Notizen.md".into(),
            base: base.into(),
        }]
    };
    let done = s.run(&client, &replace("1 Byte"));
    assert!(
        matches!(&done[..], [Done::Conflict(why)] if why.contains("changed on ILIAS since the plan"))
    );
    assert!(fake.posts(&base()).is_empty());
    // As planned, but ILIAS offers no Versions for it here: it fails, nothing sent.
    let done = s.run(&client, &replace("4 Bytes"));
    assert!(
        matches!(&done[..], [Done::Failed(why)] if why.contains("doesn't let you change")),
        "{done:?}"
    );
    assert!(fake.posts(&base()).is_empty());
    assert_eq!(s.journal()[0]["action"], "replace");
}

fn skript_trash(base: &str) -> Vec<Op> {
    vec![Op::TrashRemote {
        place: folder(),
        item: file_item(95, "Skript"),
        path: "Skript.pdf".into(),
        base: base.into(),
        files: 1,
    }]
}

#[test]
fn moves_a_ticked_deletion_into_the_trash_and_forgets_it() {
    let mut s = Setup::new("trash");
    s.base
        .change(|e| {
            e.record_file(
                "Skript.pdf",
                FileEntry {
                    id: 95,
                    ..FileEntry::default()
                },
            )
        })
        .unwrap();
    let fake = trash_ilias("1.2 MB", ASKED);
    let client = Client::with_pages(base(), fake.clone());
    assert_eq!(s.run(&client, &skript_trash("1.2 MB")), [OK]);
    assert_eq!(
        fake.posts(&base()),
        [format!("{CONFIRMED} <- form/input_0=95")]
    );
    assert!(
        s.base.file("Skript.pdf").is_none(),
        "gone on both sides now"
    );
    let lines = s.journal();
    assert_eq!(lines[0]["action"], "trash");
    assert_eq!(lines[0]["result"], "ok: file 95 Skript in the ILIAS trash");
}

#[test]
fn a_dry_run_trashes_nothing() {
    let mut s = Setup::new("trash-dry");
    let fake = trash_ilias("1.2 MB", ASKED);
    let client = Client::with_pages(base(), fake.clone());
    client.set_dry_run(true);
    assert_eq!(s.run(&client, &skript_trash("1.2 MB")), [Done::DryRun]);
    assert!(fake.posts(&base()).is_empty());
    assert!(s.journal().is_empty());
}

#[test]
fn trashes_nothing_for_good_nor_anything_changed_since_the_plan() {
    let mut s = Setup::new("trash-refuse");
    // No trash on this ILIAS: it would be gone for good.
    let warned = "Are you sure that you want to delete the following? (WARNING: The selected object(s) will be irretrievably deleted from the system.)";
    let fake = trash_ilias("1.2 MB", warned);
    let client = Client::with_pages(base(), fake.clone());
    let done = s.run(&client, &skript_trash("1.2 MB"));
    assert!(
        matches!(&done[..], [Done::Failed(why)] if why.contains("for good")),
        "{done:?}"
    );
    assert!(fake.posts(&base()).is_empty());
    // Changed on ILIAS since the plan.
    let fake = trash_ilias("1.3 MB", ASKED);
    let client = Client::with_pages(base(), fake.clone());
    let done = s.run(&client, &skript_trash("1.2 MB"));
    assert!(
        matches!(&done[..], [Done::Conflict(why)] if why.contains("changed on ILIAS since the plan"))
    );
    assert!(fake.posts(&base()).is_empty());
    // Back on your disk.
    std::fs::write(s.dir.join("Skript.pdf"), "restored").unwrap();
    let fake = trash_ilias("1.2 MB", ASKED);
    let client = Client::with_pages(base(), fake.clone());
    let done = s.run(&client, &skript_trash("1.2 MB"));
    assert!(matches!(&done[..], [Done::Conflict(why)] if why.contains("back on your disk")));
    assert!(fake.posts(&base()).is_empty());
}

/// `Eigenes/` with `a.md`, and `Eigenes/Woche 1/` with `b.md`, as the plan has them.
fn new_folder(s: &Setup) -> Vec<Op> {
    let dir = s.dir.join("Eigenes");
    std::fs::create_dir_all(dir.join("Woche 1")).unwrap();
    std::fs::write(dir.join("a.md"), "a").unwrap();
    std::fs::write(dir.join("Woche 1/b.md"), "b").unwrap();
    vec![Op::MkdirRemote {
        place: folder(),
        dir: "Eigenes".into(),
        folders: vec!["Eigenes/Woche 1".into()],
        files: vec!["Eigenes/Woche 1/b.md".into(), "Eigenes/a.md".into()],
    }]
}

#[test]
fn creates_new_folders_before_what_is_in_them_and_records_all_of_it() {
    let mut s = Setup::new("mkdir");
    let fake = tree_ilias(&["Eigenes", "Woche 1"], &["a", "b"], None);
    let client = Client::with_pages(base(), fake.clone());
    let ops = new_folder(&s);
    assert_eq!(s.run(&client, &ops), [OK]);
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
    // In sync afterwards: the folders with their ILIAS ids, the files too.
    let folders: Vec<_> = s.base.folders().map(|(p, e)| (p, e.id)).collect();
    assert_eq!(
        folders,
        [("Eigenes", Some(150)), ("Eigenes/Woche 1", Some(151))]
    );
    assert_eq!(s.base.find(160).map(|(p, _)| p), Some("Eigenes/a.md"));
    assert_eq!(
        s.base.find(161).map(|(p, _)| p),
        Some("Eigenes/Woche 1/b.md")
    );
    let actions: Vec<String> = s
        .journal()
        .iter()
        .map(|l| l["action"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(actions, ["new folder", "new folder", "upload", "upload"]);
}

#[test]
fn a_dry_run_creates_no_folder_and_a_name_taken_since_the_plan_is_a_conflict() {
    let mut s = Setup::new("mkdir-dry");
    let fake = tree_ilias(&["Eigenes", "Woche 1"], &["a", "b"], None);
    let client = Client::with_pages(base(), fake.clone());
    client.set_dry_run(true);
    let ops = new_folder(&s);
    assert_eq!(s.run(&client, &ops), [Done::DryRun]);
    assert!(fake.posts(&base()).is_empty());
    assert_eq!(s.base.folders().count(), 0);

    let fake = tree_ilias(&["Eigenes", "Woche 1"], &["a", "b"], Some("Eigenes"));
    let client = Client::with_pages(base(), fake.clone());
    let done = s.run(&client, &ops);
    assert!(matches!(&done[..], [Done::Conflict(why)] if why.contains("since the plan")));
    assert!(fake.posts(&base()).is_empty());
    assert!(s.journal().is_empty());
}

#[test]
fn your_trash_takes_only_files_still_as_saved() {
    let mut s = Setup::new("local-trash");
    let fake = ilias(UPLOAD, &[]);
    let client = Client::with_pages(base(), fake.clone());
    s.base
        .record(&s.dir.join("Notizen.md"), 7, "v1".into())
        .unwrap();
    std::fs::write(s.dir.join("Notizen.md"), "mine, with notes").unwrap();
    let ops = [
        Op::TrashLocal {
            path: "Notizen.md".into(),
        },
        Op::TrashLocal {
            path: "Fremd.md".into(),
        },
    ];
    let done = s.run(&client, &ops);
    assert!(
        matches!(&done[..], [Done::Conflict(a), Done::Conflict(b)] if a.contains("changed since the sync") && b.contains("not saved by sync")),
        "{done:?}"
    );
    assert!(s.dir.join("Notizen.md").exists());
    assert!(s.base.file("Notizen.md").is_some());
}

#[test]
fn a_failed_download_leaves_the_base_as_it_was_and_the_order_is_the_plans() {
    let mut s = Setup::new("download");
    let fake = ilias(UPLOAD, &[]);
    let client = Client::with_pages(base(), fake.clone());
    client.set_offline(true);
    let download = Op::Download {
        file: file_item(9, "DEMO › Blatt 3"),
        version: "v2".into(),
        dir: String::new(),
        replace: None,
    };
    let ops = [upload("Notizen.md").remove(0), download];
    let outcomes = run(&client, &ops, &mut s.base, &mut s.journal, |_| {});
    // Downloads run first, but the outcomes come in the plan's order.
    assert!(matches!(outcomes[0].0, Op::Upload { .. }));
    assert!(
        matches!(&outcomes[1].1, Done::Failed(why) if why.contains("Blatt 3")),
        "{outcomes:?}"
    );
    assert!(s.base.find(9).is_none());
}
