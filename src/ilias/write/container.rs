//! Writes for courses and folders, by people ILIAS lets change them (see Rights in
//! CONTEXT.md): uploading files into one, a new version of a file in one, creating a folder
//! in one, moving an item of one into the trash.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use color_eyre::{Result, eyre::bail, eyre::eyre};
use serde::Deserialize;
use url::Url;

use super::super::pages::Form;
use super::super::{Client, Id, Item, Kind, lists_as, parse};

/// The name a file goes by on ILIAS: its file name.
fn file_name(path: &Path) -> Result<String> {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| eyre!("{} has no file name", path.display()))
}

impl Client {
    /// Uploads the files at `paths` into `container` (a course, group or folder) through its
    /// own "Upload Files" form, as the browser does: each file to the form's upload address,
    /// then the form with their ids. Returns their names once the container lists them, or
    /// `None` in dry-run mode, where nothing is sent.
    pub fn upload_files(&self, container: &Item, paths: &[PathBuf]) -> Result<Option<Vec<String>>> {
        let Id::Ref(ref_id) = container.id else {
            bail!("{} is not a course or folder", container.title);
        };
        let names = paths
            .iter()
            .map(|p| file_name(p))
            .collect::<Result<Vec<_>>>()?;
        if names.is_empty() {
            bail!("no files to upload");
        }
        let page = self.get_authed(&Url::parse(&container.url)?)?;
        // Only ever send what this function is for: files into this very container.
        let form = parse::files_form(&page.html, &page.url, ref_id)
            .ok_or_else(|| eyre!("ILIAS offers no upload into {} to you", container.title))?;
        if form.max_files.is_some_and(|max| paths.len() > max) {
            bail!(
                "ILIAS takes at most {} files at once here",
                form.max_files.unwrap_or(0)
            );
        }
        for (path, name) in paths.iter().zip(&names) {
            let size = std::fs::metadata(path)?.len();
            if let Some(max) = form.max_size.filter(|max| size > *max) {
                bail!(
                    "{name} is {}, ILIAS takes at most {} here",
                    megabytes(size),
                    megabytes(max)
                );
            }
        }
        if self.dry_run.load(Ordering::Relaxed) {
            return Ok(None);
        }

        self.ensure_online()?;
        let mut ids = Vec::new();
        for (path, name) in paths.iter().zip(&names) {
            ids.push(self.send_file(&form.upload, path, name)?);
        }
        let [title, description, id] = &form.fields;
        let mut fields = Vec::new();
        for (name, file_id) in names.iter().zip(&ids) {
            fields.push(Form::Field {
                name: title,
                value: name,
            });
            fields.push(Form::Field {
                name: description,
                value: "",
            });
            fields.push(Form::Field {
                name: id,
                value: file_id,
            });
        }
        self.request(&form.finish, Some(&fields), true)?;

        let listed: Vec<String> = self
            .page(container, None)?
            .items()
            .filter(|i| i.kind == Kind::File)
            .map(|i| i.title.clone())
            .collect();
        let missing: Vec<&String> = names
            .iter()
            .filter(|name| !listed.iter().any(|title| lists_as(title, name)))
            .collect();
        if !missing.is_empty() {
            bail!(
                "sent, but ILIAS doesn't list {}: check {} in ILIAS",
                missing
                    .iter()
                    .map(|n| n.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                container.title
            );
        }
        Ok(Some(names))
    }

    /// Posts one file to an upload address of a form, as the browser does before sending
    /// the form; returns the id ILIAS gives it.
    fn send_file(&self, upload: &Url, path: &Path, name: &str) -> Result<String> {
        let answer = self.request(
            upload,
            Some(&[Form::File {
                name: "file[0]",
                path,
            }]),
            true,
        )?;
        // Status 1 is ILIAS' "OK" for an upload; anything but this answer means it failed.
        #[derive(Deserialize)]
        struct Answer {
            status: i64,
            #[serde(default)]
            message: String,
            #[serde(default)]
            file_id: String,
        }
        let answer: Answer = serde_json::from_str(&answer.html)
            .map_err(|_| eyre!("ILIAS did not take {name}: unexpected answer"))?;
        if answer.status != 1 || answer.file_id.is_empty() {
            bail!("ILIAS did not take {name}: {}", answer.message);
        }
        Ok(answer.file_id)
    }

    /// Uploads `path` as a new version of `file` in `container`, through the file's own
    /// "Create New Version" form (never "Replace File": the older versions stay). The way
    /// there is read from the pages: the file's Versions entry in the container's menu,
    /// then that tab's "Create New Version". Returns the new version's number once the tab
    /// lists it, or `None` in dry-run mode, where nothing is sent.
    pub fn upload_version(
        &self,
        container: &Item,
        file: &Item,
        path: &Path,
    ) -> Result<Option<u32>> {
        let (Id::Ref(_), Id::Ref(file_ref)) = (container.id, file.id) else {
            bail!("ILIAS keeps no versions of {}", file.title);
        };
        let name = file_name(path)?;
        let page = self.get_authed(&Url::parse(&container.url)?)?;
        // Only ever send what this function is for: a new version of this very file.
        let tab = parse::versions_link(&page.html, &page.url, file_ref)
            .ok_or_else(|| eyre!("ILIAS doesn't let you change {}", file.title))?;
        let tab = self.get_authed(&tab)?;
        let before = parse::versions(&tab.html).into_iter().max().unwrap_or(0);
        let link = parse::new_version_link(&tab.html, &tab.url, file_ref)
            .ok_or_else(|| eyre!("ILIAS offers no new version of {}", file.title))?;
        let screen = self.get_authed(&link)?;
        let form = parse::version_form(&screen.html, &screen.url, file_ref)
            .ok_or_else(|| eyre!("unexpected form for a new version of {}", file.title))?;
        let size = std::fs::metadata(path)?.len();
        if let Some(max) = form.max_size.filter(|max| size > *max) {
            bail!(
                "{name} is {}, ILIAS takes at most {} here",
                megabytes(size),
                megabytes(max)
            );
        }
        if self.dry_run.load(Ordering::Relaxed) {
            return Ok(None);
        }

        self.ensure_online()?;
        let id = self.send_file(&form.upload, path, &name)?;
        self.request(
            &form.save,
            Some(&[
                Form::Field {
                    name: &form.title.0,
                    value: &form.title.1,
                },
                Form::Field {
                    name: &form.description,
                    value: "",
                },
                Form::Field {
                    name: &form.id,
                    value: &id,
                },
            ]),
            true,
        )?;
        let after = self.get_authed(&tab.url)?;
        match parse::versions(&after.html).into_iter().max() {
            Some(version) if version > before => Ok(Some(version)),
            _ => bail!(
                "sent, but ILIAS lists no new version of {}: check it in ILIAS",
                file.title
            ),
        }
    }

    /// Creates a folder `name` in `container` through its own "Add New Item" form. Returns
    /// `false` in dry-run mode, where nothing is sent.
    pub fn create_folder(&self, container: &Item, name: &str) -> Result<bool> {
        let Id::Ref(ref_id) = container.id else {
            bail!("{} is not a course or folder", container.title);
        };
        let name = name.trim();
        if name.is_empty() {
            bail!("a folder needs a name");
        }
        let page = self.get_authed(&Url::parse(&container.url)?)?;
        // Only ever send what this function is for: a folder in this very container.
        let link = parse::create_link(&page.html, &page.url, ref_id, &Kind::Folder)
            .ok_or_else(|| eyre!("ILIAS doesn't let you add a folder to {}", container.title))?;
        let screen = self.get_authed(&link)?;
        let form = parse::create_form(
            &screen.html,
            &screen.url,
            ref_id,
            &Kind::Folder,
            "ilObjFolderGUI",
        )
        .ok_or_else(|| eyre!("unexpected form for a new folder in {}", container.title))?;
        if self.dry_run.load(Ordering::Relaxed) {
            return Ok(false);
        }

        self.ensure_online()?;
        self.request(
            &form.action,
            Some(&[
                Form::Field {
                    name: &form.title,
                    value: name,
                },
                Form::Field {
                    name: &form.description,
                    value: "",
                },
            ]),
            true,
        )?;
        let listed = self
            .page(container, None)?
            .items()
            .any(|i| i.kind == Kind::Folder && i.title == name);
        if !listed {
            bail!(
                "sent, but ILIAS doesn't list the folder {name}: check {} in ILIAS",
                container.title
            );
        }
        Ok(true)
    }

    /// Moves `item` from `container` into the ILIAS trash, as its Delete in the action menu
    /// does: ILIAS asks, then deletes. Refuses where ILIAS has no trash (it would be gone for
    /// good). Returns `false` in dry-run mode, where nothing is sent.
    pub fn delete_item(&self, container: &Item, item: &Item) -> Result<bool> {
        let (Id::Ref(ref_id), Id::Ref(item_ref)) = (container.id, item.id) else {
            bail!("ILIAS can't delete {} here", item.title);
        };
        let page = self.get_authed(&Url::parse(&container.url)?)?;
        // Only ever send what this function is for: this very item, from this container.
        let link = parse::delete_link(&page.html, &page.url, ref_id, item_ref)
            .ok_or_else(|| eyre!("ILIAS doesn't let you delete {}", item.title))?;
        // Only shows ILIAS' question, but its command is a write's (`delete`).
        let asked = self.authed(&link, true)?;
        let form =
            parse::confirm_delete(&asked.html, &asked.url, ref_id, item_ref).ok_or_else(|| {
                eyre!(
                    "ILIAS did not ask to confirm; {} was not deleted",
                    item.title
                )
            })?;
        if !form.to_trash {
            bail!(
                "ILIAS would delete {} for good here (it has no trash): not deleted. Use the browser if you mean it.",
                item.title
            );
        }
        if self.dry_run.load(Ordering::Relaxed) {
            return Ok(false);
        }

        self.ensure_online()?;
        self.request(
            &form.action,
            Some(&[Form::Field {
                name: &form.field,
                value: &form.value,
            }]),
            true,
        )?;
        if self.page(container, None)?.items().any(|i| i.id == item.id) {
            bail!(
                "ILIAS still lists {}: check {} in ILIAS",
                item.title,
                container.title
            );
        }
        Ok(true)
    }
}

/// `1.2 MB`.
pub fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::ilias::pages::{FakeIlias, reply};

    fn base() -> Url {
        Url::parse("https://ilias.example/ilias/").unwrap()
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

    /// Folder 89's "Upload Files" form as the local ILIAS 10 renders it (shortened), with
    /// `upload` as the address for the files, `max` bytes per file and `listed` in the folder.
    fn folder_page(upload: &str, max: u64, listed: &str) -> String {
        format!(
            r#"<div id="il_center_col"><div class="ilContainerListItemOuter"><a class="il_ContainerItemTitle" href="ilias.php?cmdClass=ilObjFileGUI&amp;cmd=sendfile&amp;ref_id=95">{listed}</a></div></div>
            <form enctype="multipart/form-data" action="{}" method="post">
              <fieldset class="c-input" data-il-ui-component="file-field-input"><template>
                <input type="text" name="form/input_0[input_1][]" />
                <textarea name="form/input_0[input_2][]"></textarea>
                <input type="hidden" name="form/input_0[input_3][]" value="" />
              </template></fieldset></form>
            <script>il.UI.Input.File.init(
                'il_ui_fw_1',
                '{upload}',
                'ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjFileUploadHandlerGUI&cmd=remove&ref_id=89&cmdMode=asynch',
                'file_id',
                0,
                100,
                {max},
                '',
                false);</script>"#,
            FINISH.replace('&', "&amp;")
        )
    }

    /// ILIAS with that form: the upload answers with ids, and afterwards lists `listed`.
    fn with_form(upload: &'static str, listed: &'static str) -> Arc<FakeIlias> {
        let uploads = std::sync::atomic::AtomicUsize::new(0);
        Arc::new(FakeIlias::new(move |url| {
            if url.as_str().contains("cmd=upload&") {
                let n = uploads.fetch_add(1, Ordering::SeqCst);
                return reply(url, &format!(r#"{{"status":1,"file_id":"id-{n}"}}"#));
            }
            reply(url, &folder_page(upload, 1000, listed))
        }))
    }

    /// Files to upload, in a folder of their own per call (tests run in parallel).
    fn files(names: &[&str]) -> Vec<PathBuf> {
        static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let call = CALLS.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("ilias-tui-upload-{}-{call}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        names
            .iter()
            .map(|name| {
                let path = dir.join(name);
                std::fs::write(&path, "some text").unwrap();
                path
            })
            .collect()
    }

    #[test]
    fn uploads_files_like_the_browser_and_checks_they_are_listed() {
        let ilias = with_form(UPLOAD, "notes");
        let client = Client::with_pages(base(), ilias.clone());
        let uploaded = client.upload_files(&folder(), &files(&["notes.txt", "b & c.pdf"]));
        // Only "notes" is listed afterwards.
        let error = uploaded.unwrap_err().to_string();
        assert!(error.contains("doesn't list b & c.pdf"), "{error}");
        assert_eq!(
            ilias.posts(&base()),
            [
                format!("{UPLOAD} <- file[0]=@notes.txt"),
                format!("{UPLOAD} <- file[0]=@b & c.pdf"),
                format!(
                    "{FINISH} <- form/input_0[input_1][]=notes.txt&form/input_0[input_2][]=&form/input_0[input_3][]=id-0\
                     &form/input_0[input_1][]=b & c.pdf&form/input_0[input_2][]=&form/input_0[input_3][]=id-1"
                ),
            ]
        );

        let client = Client::with_pages(base(), with_form(UPLOAD, "notes"));
        let uploaded = client
            .upload_files(&folder(), &files(&["notes.txt"]))
            .unwrap();
        assert_eq!(uploaded, Some(vec!["notes.txt".to_string()]));
    }

    #[test]
    fn a_dry_run_uploads_nothing() {
        let ilias = with_form(UPLOAD, "notes");
        let client = Client::with_pages(base(), ilias.clone());
        client.set_dry_run(true);
        assert_eq!(
            client
                .upload_files(&folder(), &files(&["notes.txt"]))
                .unwrap(),
            None
        );
        assert!(ilias.posts(&base()).is_empty());
    }

    #[test]
    fn uploads_nothing_without_the_folders_own_form() {
        for upload in [
            // Another folder, another site, another command.
            "ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjFileUploadHandlerGUI&cmd=upload&ref_id=90",
            "https://evil.example/ilias/ilias.php?cmdClass=ilObjFileUploadHandlerGUI&cmd=upload&ref_id=89",
            "ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjFileUploadHandlerGUI&cmd=remove&ref_id=89",
        ] {
            let ilias = with_form(upload, "notes");
            let client = Client::with_pages(base(), ilias.clone());
            assert!(
                client
                    .upload_files(&folder(), &files(&["notes.txt"]))
                    .is_err(),
                "{upload}"
            );
            assert!(ilias.posts(&base()).is_empty(), "{upload}");
        }
        // A student's view: no form at all.
        let ilias = Arc::new(FakeIlias::new(|url| reply(url, "<p>Unterlagen</p>")));
        let client = Client::with_pages(base(), ilias.clone());
        let error = client
            .upload_files(&folder(), &files(&["notes.txt"]))
            .unwrap_err();
        assert!(error.to_string().contains("offers no upload"), "{error}");
        assert!(ilias.posts(&base()).is_empty());
        // Not a container with a reference id.
        let assignment = Item {
            id: Id::Assignment(1),
            ..folder()
        };
        assert!(
            client
                .upload_files(&assignment, &files(&["notes.txt"]))
                .is_err()
        );
    }

    #[test]
    fn a_file_too_big_for_ilias_is_not_sent() {
        let ilias = Arc::new(FakeIlias::new(|url| {
            reply(url, &folder_page(UPLOAD, 4, "x"))
        }));
        let client = Client::with_pages(base(), ilias.clone());
        let error = client
            .upload_files(&folder(), &files(&["notes.txt"]))
            .unwrap_err();
        assert!(error.to_string().contains("at most"), "{error}");
        assert!(ilias.posts(&base()).is_empty());
    }

    const VERSIONS: &str =
        "ilias.php?baseClass=ilrepositorygui&cmdNode=wq:mk:cv&cmdClass=ilFileVersionsGUI&ref_id=95";
    const NEW_VERSION: &str = "ilias.php?baseClass=ilrepositorygui&cmdNode=wq:mk:cv&cmdClass=ilFileVersionsGUI&cmd=addNewVersion&ref_id=95";
    const SAVE_VERSION: &str = "ilias.php?baseClass=ilrepositorygui&cmdNode=wq:mk:cv&cmdClass=ilFileVersionsGUI&cmd=post&fallbackCmd=saveVersion&ref_id=95&rtoken=t";
    const APPEND: &str = "ilias.php?baseClass=ilrepositorygui&cmdNode=wq:mk:cv:cx&cmdClass=ilFileVersionsUploadHandlerGUI&cmd=upload&ref_id=95&upload_mode=append&cmdMode=asynch";

    /// Folder 89 listing file 95 with Versions in its menu (`versions`), the Versions tab
    /// with "Create New Version" (`new_version`) and one more row once a version is saved
    /// (unless `lists_new` is false), and the form posting to `save`, its file to `upload`.
    fn versions_ilias(
        versions: &'static str,
        new_version: &'static str,
        save: &'static str,
        upload: &'static str,
        lists_new: bool,
    ) -> Arc<FakeIlias> {
        let saved = std::sync::atomic::AtomicBool::new(false);
        Arc::new(FakeIlias::new(move |url| {
            let at = url.as_str();
            if at.contains("cmd=upload&") {
                return reply(url, r#"{"status":1,"message":"","file_id":"v-1"}"#);
            }
            if at.contains("fallbackCmd=saveVersion") {
                saved.store(lists_new, Ordering::SeqCst);
                return reply(url, "<p>Version saved</p>");
            }
            if at.contains("cmd=addNewVersion") {
                return reply(
                    url,
                    &format!(
                        r#"<form class="c-form c-form--horizontal" enctype="multipart/form-data" action="{}" method="post">
                        <input type="text" value="Skript.pdf" name="form/input_0/input_1" maxlength="255" class="c-field-text" />
                        <textarea class="c-field-textarea" name="form/input_0/input_2"></textarea>
                        <template><input type="hidden" name="form/input_0/input_3[input_0][]" value="" /></template></form>
                        <script>il.UI.Input.File.init('il_ui_fw_1', '{upload}',
                            'ilias.php?baseClass=ilrepositorygui&cmdNode=wq:mk:cv:cx&cmdClass=ilFileVersionsUploadHandlerGUI&cmd=remove&ref_id=95&upload_mode=append&cmdMode=asynch',
                            'file_id', 0, 1, 1000, '', false);</script>"#,
                        save.replace('&', "&amp;")
                    ),
                );
            }
            if at.contains("cmdClass=ilFileVersionsGUI") {
                let row = |n: u32| {
                    format!(
                        r#"<tr class="tblrow1"><td class="std"><input type="checkbox" name="hist_id[]" value="{n}" /></td>
                        <td class="std ilRight"> {n} </td><td class="std"><a href="ilias.php?cmdClass=ilFileVersionsGUI&amp;cmd=sendFile&amp;ref_id=95&amp;hist_id={n}">Skript.pdf</a></td></tr>"#
                    )
                };
                let rows = match saved.load(Ordering::SeqCst) {
                    true => row(2) + &row(1),
                    false => row(1),
                };
                return reply(
                    url,
                    &format!(
                        r#"<button class="btn btn-default" data-action="{}">Create New Version</button>
                        <button class="btn btn-default" data-action="ilias.php?baseClass=ilrepositorygui&amp;cmdNode=wq:mk:cv&amp;cmdClass=ilFileVersionsGUI&amp;cmd=addReplacingVersion&amp;ref_id=95">Replace File</button>
                        <table class="table table-striped fullwidth" id="ilFileVersionsTableGUI"><thead><tr><th>Version</th></tr></thead><tbody>{rows}</tbody></table>"#,
                        new_version.replace('&', "&amp;")
                    ),
                );
            }
            reply(
                url,
                &format!(
                    r#"<div id="il_center_col"><div class="ilContainerListItemOuter"><a class="il_ContainerItemTitle" href="goto.php/file/95">Skript</a>
                    <ul><li><button class="btn btn-link" data-action="http://ilias.example/x">Info</button></li>
                    <li><button class="btn btn-link" data-action="{}">Versions</button></li></ul></div></div>"#,
                    versions.replace('&', "&amp;")
                ),
            )
        }))
    }

    #[test]
    fn uploads_a_new_version_through_create_new_version_and_checks_it_is_listed() {
        let ilias = versions_ilias(VERSIONS, NEW_VERSION, SAVE_VERSION, APPEND, true);
        let client = Client::with_pages(base(), ilias.clone());
        let path = &files(&["Skript.pdf"])[0];
        assert_eq!(
            client.upload_version(&folder(), &skript(), path).unwrap(),
            Some(2)
        );
        assert_eq!(
            ilias.requests(&base()),
            ["goto.php/fold/89", VERSIONS, NEW_VERSION, VERSIONS]
        );
        assert_eq!(
            ilias.posts(&base()),
            [
                format!("{APPEND} <- file[0]=@Skript.pdf"),
                format!(
                    "{SAVE_VERSION} <- form/input_0/input_1=Skript.pdf&form/input_0/input_2=&form/input_0/input_3[input_0][]=v-1"
                ),
            ]
        );
        // Sent, but no new row: an error, not a silent success.
        let ilias = versions_ilias(VERSIONS, NEW_VERSION, SAVE_VERSION, APPEND, false);
        let client = Client::with_pages(base(), ilias);
        let error = client
            .upload_version(&folder(), &skript(), path)
            .unwrap_err();
        assert!(error.to_string().contains("no new version"), "{error}");
    }

    #[test]
    fn a_dry_run_uploads_no_version() {
        let ilias = versions_ilias(VERSIONS, NEW_VERSION, SAVE_VERSION, APPEND, true);
        let client = Client::with_pages(base(), ilias.clone());
        client.set_dry_run(true);
        let path = &files(&["Skript.pdf"])[0];
        assert_eq!(
            client.upload_version(&folder(), &skript(), path).unwrap(),
            None
        );
        assert!(ilias.posts(&base()).is_empty());
    }

    #[test]
    fn uploads_no_version_but_a_new_one_of_this_file_on_this_ilias() {
        let path = &files(&["Skript.pdf"])[0];
        for (versions, new_version, save, upload) in [
            // The menu's Versions is another file's, or another site's.
            (
                "ilias.php?cmdClass=ilFileVersionsGUI&ref_id=96",
                NEW_VERSION,
                SAVE_VERSION,
                APPEND,
            ),
            (
                "https://evil.example/ilias/ilias.php?cmdClass=ilFileVersionsGUI&ref_id=95",
                NEW_VERSION,
                SAVE_VERSION,
                APPEND,
            ),
            // The tab's button leads to replacing the file.
            (
                VERSIONS,
                "ilias.php?cmdClass=ilFileVersionsGUI&cmd=addReplacingVersion&ref_id=95",
                SAVE_VERSION,
                APPEND,
            ),
            // The form saves for another file, or uploads to replace.
            (
                VERSIONS,
                NEW_VERSION,
                "ilias.php?cmdClass=ilFileVersionsGUI&cmd=post&fallbackCmd=saveVersion&ref_id=96",
                APPEND,
            ),
            (
                VERSIONS,
                NEW_VERSION,
                SAVE_VERSION,
                "ilias.php?cmdClass=ilFileVersionsUploadHandlerGUI&cmd=upload&ref_id=95&upload_mode=replace",
            ),
            (
                VERSIONS,
                NEW_VERSION,
                SAVE_VERSION,
                "https://evil.example/ilias/ilias.php?cmdClass=ilFileVersionsUploadHandlerGUI&cmd=upload&ref_id=95&upload_mode=append",
            ),
        ] {
            let ilias = versions_ilias(versions, new_version, save, upload, true);
            let client = Client::with_pages(base(), ilias.clone());
            assert!(
                client.upload_version(&folder(), &skript(), path).is_err(),
                "{versions} {new_version} {save} {upload}"
            );
            assert!(
                ilias.posts(&base()).is_empty(),
                "{versions} {new_version} {save} {upload}"
            );
        }
    }

    const CREATE: &str = "https://ilias.example/ilias/ilias.php?baseClass=ilrepositorygui&cmd=create&ref_id=89&new_type=fold";
    const SAVE: &str = "ilias.php?baseClass=ilrepositorygui&cmdNode=wq:mp&cmdClass=ilObjFolderGUI&cmd=post&fallbackCmd=save&ref_id=89&new_type=fold&rtoken=t";

    /// Folder 89 with "Add New Item → Folder" (if `create`), the form for it posting to
    /// `save`, and `listed` in it.
    fn folder_ilias(create: bool, save: &'static str, listed: &'static str) -> Arc<FakeIlias> {
        Arc::new(FakeIlias::new(move |url| {
            if url.as_str().contains("cmd=create") {
                return reply(
                    url,
                    &format!(
                        r#"<form class="c-form" action="{}" method="post">
                        <input type="text" name="form/input_0/input_1" maxlength="255" />
                        <textarea name="form/input_0/input_2"></textarea></form>"#,
                        save.replace('&', "&amp;")
                    ),
                );
            }
            let menu = match create {
                true => format!(
                    r#"<a class="il-link link-bulky" href="{}">Folder</a>"#,
                    CREATE.replace('&', "&amp;")
                ),
                false => String::new(),
            };
            reply(
                url,
                &format!(
                    r#"{menu}<div id="il_center_col"><div class="ilContainerListItemOuter">
                    <a class="il_ContainerItemTitle" href="goto.php/fold/96">{listed}</a></div></div>"#
                ),
            )
        }))
    }

    #[test]
    fn creates_a_folder_through_ilias_form_and_checks_it_is_listed() {
        let ilias = folder_ilias(true, SAVE, "Woche 5");
        let client = Client::with_pages(base(), ilias.clone());
        assert!(client.create_folder(&folder(), " Woche 5 ").unwrap());
        assert_eq!(
            ilias.posts(&base()),
            [format!(
                "{SAVE} <- form/input_0/input_1=Woche 5&form/input_0/input_2="
            )]
        );

        let client = Client::with_pages(base(), folder_ilias(true, SAVE, "Woche 4"));
        let error = client.create_folder(&folder(), "Woche 5").unwrap_err();
        assert!(error.to_string().contains("doesn't list"), "{error}");
    }

    #[test]
    fn a_dry_run_creates_no_folder() {
        let ilias = folder_ilias(true, SAVE, "Woche 5");
        let client = Client::with_pages(base(), ilias.clone());
        client.set_dry_run(true);
        assert!(!client.create_folder(&folder(), "Woche 5").unwrap());
        assert!(ilias.posts(&base()).is_empty());
    }

    #[test]
    fn creates_no_folder_without_ilias_offering_one_here() {
        // A student: no "Add New Item".
        let ilias = folder_ilias(false, SAVE, "x");
        let client = Client::with_pages(base(), ilias.clone());
        assert!(client.create_folder(&folder(), "Woche 5").is_err());
        assert!(ilias.posts(&base()).is_empty());
        // The form posts to another folder, another site, or makes something else.
        for save in [
            "ilias.php?cmdClass=ilObjFolderGUI&cmd=post&fallbackCmd=save&ref_id=90&new_type=fold",
            "https://evil.example/ilias/ilias.php?cmdClass=ilObjFolderGUI&cmd=post&fallbackCmd=save&ref_id=89&new_type=fold",
            "ilias.php?cmdClass=ilObjFolderGUI&cmd=post&fallbackCmd=delete&ref_id=89&new_type=fold",
            "ilias.php?cmdClass=ilObjCourseGUI&cmd=post&fallbackCmd=save&ref_id=89&new_type=crs",
        ] {
            let ilias = folder_ilias(true, save, "x");
            let client = Client::with_pages(base(), ilias.clone());
            assert!(
                client.create_folder(&folder(), "Woche 5").is_err(),
                "{save}"
            );
            assert!(ilias.posts(&base()).is_empty(), "{save}");
        }
        let client = Client::with_pages(base(), folder_ilias(true, SAVE, "x"));
        assert!(client.create_folder(&folder(), "  ").is_err());
    }

    const DELETE: &str = "ilias.php?baseClass=ilrepositorygui&cmdNode=wq:mp&cmdClass=ilObjFolderGUI&cmd=delete&ref_id=89&item_ref_id=95";
    const CONFIRMED: &str = "ilias.php?baseClass=ilrepositorygui&cmdNode=wq:mp&cmdClass=ilObjFolderGUI&cmd=post&fallbackCmd=confirmedDelete&ref_id=89&id=95&rtoken=t";
    const ASKED: &str = "Are you sure that you want to delete the following?";

    fn skript() -> Item {
        Item {
            kind: Kind::File,
            id: Id::Ref(95),
            title: "Skript".into(),
            ..Item::default()
        }
    }

    /// Folder 89 listing file 95 with a Delete going to `delete` (until it's deleted), and
    /// ILIAS' question `question` posting to `confirmed`.
    fn delete_ilias(
        delete: &'static str,
        confirmed: &'static str,
        question: &'static str,
    ) -> Arc<FakeIlias> {
        let gone = Arc::new(std::sync::atomic::AtomicBool::new(false));
        Arc::new(FakeIlias::new(move |url| {
            if url.as_str().contains("fallbackCmd=confirmedDelete") {
                gone.store(true, Ordering::SeqCst);
            }
            let question = format!(
                r#"<dialog class="c-modal"><div class="modal-body">
                <div class="alert alert-warning" role="status"><div class="ilAccHeadingHidden">-confirmation_message-</div>{question}</div>
                <ul><li>Skript.pdf</li></ul>
                <form class="c-form" action="{}" method="post"><input type="hidden" name="form/input_0" value="95" /></form>
                </div></dialog>"#,
                confirmed.replace('&', "&amp;")
            );
            let listing = match gone.load(Ordering::SeqCst) {
                true => String::new(),
                false => format!(
                    r#"<div class="ilContainerListItemOuter"><a class="il_ContainerItemTitle" href="goto.php/file/95">Skript</a>
                    <ul><li><button class="btn btn-link" data-action="{}">Delete</button></li></ul></div>"#,
                    delete.replace('&', "&amp;")
                ),
            };
            let asking = url.as_str().contains("cmd=delete");
            reply(
                url,
                &format!(
                    r#"<div id="il_center_col">{listing}</div>{}"#,
                    if asking { question } else { String::new() }
                ),
            )
        }))
    }

    #[test]
    fn deletes_into_the_trash_after_confirming_like_ilias_asks() {
        let ilias = delete_ilias(DELETE, CONFIRMED, ASKED);
        let client = Client::with_pages(base(), ilias.clone());
        assert!(client.delete_item(&folder(), &skript()).unwrap());
        assert_eq!(
            ilias.requests(&base()),
            ["goto.php/fold/89", DELETE, "goto.php/fold/89"]
        );
        assert_eq!(
            ilias.posts(&base()),
            [format!("{CONFIRMED} <- form/input_0=95")]
        );
    }

    #[test]
    fn a_dry_run_deletes_nothing() {
        let ilias = delete_ilias(DELETE, CONFIRMED, ASKED);
        let client = Client::with_pages(base(), ilias.clone());
        client.set_dry_run(true);
        assert!(!client.delete_item(&folder(), &skript()).unwrap());
        assert!(ilias.posts(&base()).is_empty());
    }

    #[test]
    fn deletes_nothing_for_good() {
        // ILIAS without a trash warns that the item would be gone for good.
        let warned = "Are you sure that you want to delete the following? (WARNING: The selected object(s) will be irretrievably deleted from the system.)";
        let ilias = delete_ilias(DELETE, CONFIRMED, warned);
        let client = Client::with_pages(base(), ilias.clone());
        let error = client.delete_item(&folder(), &skript()).unwrap_err();
        assert!(error.to_string().contains("for good"), "{error}");
        assert!(ilias.posts(&base()).is_empty());
    }

    #[test]
    fn deletes_nothing_but_this_item_from_this_folder() {
        for (delete, confirmed) in [
            // The menu's Delete is for another item, another folder, another site.
            (
                "ilias.php?cmdClass=ilObjFolderGUI&cmd=delete&ref_id=89&item_ref_id=96",
                CONFIRMED,
            ),
            (
                "ilias.php?cmdClass=ilObjFolderGUI&cmd=delete&ref_id=90&item_ref_id=95",
                CONFIRMED,
            ),
            (
                "https://evil.example/ilias/ilias.php?cmd=delete&ref_id=89&item_ref_id=95",
                CONFIRMED,
            ),
            // ILIAS' question posts to another folder or another command.
            (
                DELETE,
                "ilias.php?cmdClass=ilObjFolderGUI&cmd=post&fallbackCmd=confirmedDelete&ref_id=90&id=95",
            ),
            (
                DELETE,
                "ilias.php?cmdClass=ilObjFolderGUI&cmd=post&fallbackCmd=cut&ref_id=89&id=95",
            ),
            (
                DELETE,
                "https://evil.example/ilias/ilias.php?cmd=post&fallbackCmd=confirmedDelete&ref_id=89",
            ),
        ] {
            let ilias = delete_ilias(delete, confirmed, ASKED);
            let client = Client::with_pages(base(), ilias.clone());
            assert!(
                client.delete_item(&folder(), &skript()).is_err(),
                "{delete} {confirmed}"
            );
            assert!(ilias.posts(&base()).is_empty(), "{delete} {confirmed}");
        }
        // An item without a reference id.
        let client = Client::with_pages(base(), delete_ilias(DELETE, CONFIRMED, ASKED));
        let link = Item {
            id: Id::Local(1 << 63),
            ..skript()
        };
        assert!(client.delete_item(&folder(), &link).is_err());
    }
}
