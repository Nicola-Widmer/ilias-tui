//! Writes for courses and folders, by people ILIAS lets change them (see Rights in
//! CONTEXT.md): uploading files into one, creating a folder in one.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use color_eyre::{Result, eyre::bail, eyre::eyre};
use serde::Deserialize;
use url::Url;

use super::super::pages::Form;
use super::super::{Client, Id, Item, Kind, parse};

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
            let answer = self.request(
                &form.upload,
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
            ids.push(answer.file_id);
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
}

/// Whether ILIAS' title for a file is that file: ILIAS may leave out the extension.
fn lists_as(title: &str, name: &str) -> bool {
    title == name
        || Path::new(name)
            .file_stem()
            .is_some_and(|stem| title == stem)
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
}
