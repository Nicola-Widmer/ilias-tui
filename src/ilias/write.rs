//! Every write (a request that changes data on ILIAS) lives here, so they are easy to review
//! in one place (see "Rules for features that write" in `docs/roadmap.md`). All other page
//! requests pass the write guard ([`is_write`]), which refuses anything that looks like a
//! write. The TUI asks before each favourites change and each file it hands in
//! ([`Action::Ask`](crate::action::Action::Ask)), and `ILIAS_DRY_RUN=1` turns those into
//! no-ops.

use std::path::Path;
use std::sync::atomic::Ordering;

use color_eyre::{Result, eyre::bail, eyre::eyre};
use serde::Deserialize;
use url::Url;

use super::pages::Form;
use super::{Client, Item, MEMBERSHIPS, parse};

/// ILIAS commands (`cmd=`) that change something, as found on ILIAS 10 pages. Many writes
/// also carry a one-time `rtoken`, but not all (favourites, read marks, join/leave don't).
const WRITE_COMMANDS: &[&str] = &[
    "addToDesk",
    "removeFromDesk",
    "removeRecommendation",
    "markPostRead",
    "markPostUnread",
    "saveRating",
    "join",
    "leave",
    "saveNotification",
    "enableForumNotification",
    "disableForumNotification",
    "toggleThreadNotification",
    "changePDItemPresentation",
    "changePDItemSorting",
    "setPdModeEvents",
    "doLogout",
    "delete",
    "confirmedRemove",
    "confirmedDelete",
    "upload",
    "addUpload",
    "uploadFile",
    "uploadZip",
    "confirmDeleteDelivered",
    "deleteDelivered",
];

/// The write guard's test: does requesting `url` look like it would change something?
pub fn is_write(url: &Url) -> bool {
    url.query_pairs().any(|(key, value)| {
        key == "rtoken" || (key == "cmd" && WRITE_COMMANDS.contains(&value.as_ref()))
    })
}

impl Client {
    /// Adds `item` to the ILIAS favourites or removes it, whichever its action menu offers.
    /// ILIAS does this with a plain GET of the menu's link. Returns `false` in dry-run mode,
    /// where nothing is sent.
    pub fn toggle_favorite(&self, item: &Item) -> Result<bool> {
        let Some(link) = &item.favorite_toggle else {
            bail!("ILIAS offers no favourites option for {}", item.title);
        };
        let url = Url::parse(link)?;
        let param = |key: &str| {
            url.query_pairs()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.into_owned())
        };
        // Only ever send what this function is for: a favourites link for this very item.
        let expected = url.scheme() == self.base.scheme()
            && url.host_str() == self.base.host_str()
            && matches!(
                param("cmd").as_deref(),
                Some("addToDesk" | "removeFromDesk")
            )
            && param("item_ref_id") == Some(item.ref_id.to_string());
        if !expected {
            bail!("unexpected favourites link for {}: {url}", item.title);
        }
        if self.dry_run.load(Ordering::Relaxed) {
            return Ok(false);
        }
        self.authed(&url, true)?;
        Ok(true)
    }

    /// Hands in the file at `path` for an assignment, through the upload form of its
    /// submission page `submission`. Returns the name ILIAS lists it under, or `None` in
    /// dry-run mode, where nothing is sent.
    pub fn hand_in(&self, submission: &str, path: &Path) -> Result<Option<String>> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| eyre!("{} has no file name", path.display()))?;
        let mut form_url = Url::parse(submission)?;
        let assignment = self.assignment_of(&form_url, "ilExSubmissionFileGUI")?;
        let pairs: Vec<(String, String)> = form_url
            .query_pairs()
            .filter(|(k, _)| k != "cmd")
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        form_url
            .query_pairs_mut()
            .clear()
            .extend_pairs(pairs)
            .append_pair("cmd", "uploadForm");

        let page = self.get_authed(&form_url)?;
        let form = parse::parse_upload_form(&page.html, &page.url)
            .ok_or_else(|| eyre!("ILIAS takes no files for this assignment right now"))?;
        // Only ever send what this function is for: to this very assignment.
        let handler = self.assignment_of(&form.upload, "ilRepoStandardUploadHandlerGUI")?;
        let finish = self.assignment_of(&form.finish, "ilExSubmissionFileGUI")?;
        let command = |url: &Url| {
            url.query_pairs()
                .find(|(k, _)| k == "cmd")
                .map(|(_, v)| v.into_owned())
        };
        if handler != assignment
            || finish != assignment
            || command(&form.upload).as_deref() != Some("upload")
            || command(&form.finish).as_deref() != Some("addUpload")
        {
            bail!("unexpected upload form for this assignment");
        }
        if self.dry_run.load(Ordering::Relaxed) {
            return Ok(None);
        }

        self.ensure_online()?;
        let uploaded = self.pages.post(
            &form.upload,
            &[Form::File {
                name: "file[0]",
                path,
            }],
        )?;
        // Status 1 is ILIAS' "OK" for an upload; anything but this answer means it failed.
        #[derive(Deserialize)]
        struct Answer {
            status: i64,
            #[serde(default)]
            message: String,
            #[serde(default)]
            filename: String,
        }
        let answer: Answer = serde_json::from_str(&uploaded.html)
            .map_err(|_| eyre!("ILIAS did not take {name}: unexpected answer"))?;
        if answer.status != 1 {
            bail!("ILIAS did not take {name}: {}", answer.message);
        }
        let listed = if answer.filename.is_empty() {
            name
        } else {
            answer.filename
        };
        let done = self.pages.post(
            &form.finish,
            &[Form::Field {
                name: &form.field,
                value: &listed,
            }],
        )?;
        if !parse::shows(&done.html, &listed) {
            bail!("{listed} was sent, but ILIAS doesn't list it: check the assignment in ILIAS");
        }
        Ok(Some(listed))
    }

    /// Deletes a handed-in `file` (a file listed on an assignment's submission page) as
    /// ILIAS' own Delete button does: tick it, press Delete, then confirm. ILIAS only allows
    /// this before the deadline. Returns `false` in dry-run mode, where nothing is sent.
    pub fn delete_submission(&self, file: &Item) -> Result<bool> {
        let link = Url::parse(&file.url)?;
        let assignment = self.assignment_of(&link, "ilExSubmissionFileGUI")?;
        let delivered = link
            .query_pairs()
            .find(|(k, _)| k == "delivered")
            .map(|(_, v)| v.into_owned())
            .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
            .ok_or_else(|| eyre!("{} is not a handed-in file", file.title))?;
        let mut screen = link.clone();
        let pairs: Vec<(String, String)> = link
            .query_pairs()
            .filter(|(k, _)| k != "cmd" && k != "delivered")
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        screen
            .query_pairs_mut()
            .clear()
            .extend_pairs(pairs)
            .append_pair("cmd", "submissionScreen");

        // Only ever post to this very assignment's submission page.
        let checked = |url: Option<Url>| -> Result<Url> {
            let url = url.ok_or_else(|| {
                eyre!(
                    "ILIAS doesn't let you delete {}: the deadline has likely passed",
                    file.title
                )
            })?;
            let command = url
                .query_pairs()
                .find(|(k, _)| k == "cmd")
                .map(|(_, v)| v.into_owned());
            if self.assignment_of(&url, "ilExSubmissionFileGUI")? != assignment
                || command.as_deref() != Some("post")
            {
                bail!("unexpected delete form for this assignment");
            }
            Ok(url)
        };
        let page = self.get_authed(&screen)?;
        let tick = checked(parse::delete_form(
            &page.html,
            &page.url,
            &delivered,
            "confirmDeleteDelivered",
        ))?;
        if self.dry_run.load(Ordering::Relaxed) {
            return Ok(false);
        }

        self.ensure_online()?;
        let ask = self.pages.post(
            &tick,
            &[
                Form::Field {
                    name: "delivered[]",
                    value: &delivered,
                },
                Form::Field {
                    name: "cmd[confirmDeleteDelivered]",
                    value: "Delete",
                },
            ],
        )?;
        let confirm = checked(parse::delete_form(
            &ask.html,
            &ask.url,
            &delivered,
            "deleteDelivered",
        ))
        .map_err(|_| {
            eyre!(
                "ILIAS did not ask to confirm; {} was not deleted",
                file.title
            )
        })?;
        let done = self.pages.post(
            &confirm,
            &[
                Form::Field {
                    name: "delivered[]",
                    value: &delivered,
                },
                Form::Field {
                    name: "cmd[deleteDelivered]",
                    value: "Delete",
                },
            ],
        )?;
        if parse::lists_delivered(&done.html, &delivered) {
            bail!(
                "ILIAS still lists {}: check the assignment in ILIAS",
                file.title
            );
        }
        Ok(true)
    }

    /// `ref_id` and `ass_id` of a link to `class` on this very ILIAS.
    fn assignment_of(&self, url: &Url, class: &str) -> Result<(String, String)> {
        let param = |key: &str| {
            url.query_pairs()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.into_owned())
        };
        let expected = url.scheme() == self.base.scheme()
            && url.host_str() == self.base.host_str()
            && param("cmdClass").is_some_and(|c| c.eq_ignore_ascii_case(class));
        match (expected, param("ref_id"), param("ass_id")) {
            (true, Some(exercise), Some(assignment)) => Ok((exercise, assignment)),
            _ => bail!("unexpected link for handing in: {url}"),
        }
    }

    /// Ends the ILIAS session on the server via its logout link (which carries a one-time
    /// token, so it has to be read from a page). Returns whether a session was ended.
    pub fn logout(&self) -> Result<bool> {
        let fetched = self.get(&self.url(MEMBERSHIPS)?)?;
        if !self.is_logged_in(&fetched) {
            return Ok(false);
        }
        let Some(url) = parse::logout_link(&fetched.html, &fetched.url) else {
            bail!("no logout link found on the ILIAS page");
        };
        if url.host_str() != self.base.host_str() {
            bail!("unexpected logout link: {url}");
        }
        self.request(&url, true)?;
        let after = self.get(&self.url(MEMBERSHIPS)?)?;
        Ok(!self.is_logged_in(&after))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::ilias::pages::{FakeIlias, LOGGED_OUT, reply};

    fn base() -> Url {
        Url::parse("https://ilias.example/ilias/").unwrap()
    }

    fn course(link: &str) -> Item {
        Item {
            ref_id: 7,
            title: "DEMO".into(),
            favorite_toggle: Some(link.into()),
            ..Item::default()
        }
    }

    const ADD: &str = "https://ilias.example/ilias/ilias.php?cmd=addToDesk&type=crs&item_ref_id=7";

    #[test]
    fn a_favourites_change_sends_exactly_its_link() {
        let ilias = Arc::new(FakeIlias::new(|url| reply(url, "<p>ok</p>")));
        let client = Client::with_pages(base(), ilias.clone());
        assert!(client.toggle_favorite(&course(ADD)).unwrap());
        assert_eq!(
            ilias.requests(&base()),
            ["ilias.php?cmd=addToDesk&type=crs&item_ref_id=7"]
        );
    }

    #[test]
    fn a_dry_run_sends_nothing() {
        let ilias = Arc::new(FakeIlias::new(|url| reply(url, "<p>ok</p>")));
        let client = Client::with_pages(base(), ilias.clone());
        client.set_dry_run(true);
        assert!(!client.toggle_favorite(&course(ADD)).unwrap());
        assert!(ilias.requests(&base()).is_empty());
    }

    #[test]
    fn logout_follows_the_pages_logout_link() {
        let logged_out = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let state = logged_out.clone();
        let ilias = Arc::new(FakeIlias::new(move |url| {
            if url.query().is_some_and(|q| q.contains("cmd=doLogout")) {
                state.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            match state.load(std::sync::atomic::Ordering::SeqCst) {
                true => reply(url, LOGGED_OUT),
                false => reply(
                    url,
                    r#"<a href="ilias.php?baseClass=ilstartupgui&amp;cmd=doLogout&amp;rtoken=abc">Abmelden</a>"#,
                ),
            }
        }));
        let client = Client::with_pages(base(), ilias.clone());
        assert!(client.logout().unwrap());
        assert_eq!(
            ilias.requests(&base()),
            [
                MEMBERSHIPS,
                "ilias.php?baseClass=ilstartupgui&cmd=doLogout&rtoken=abc",
                MEMBERSHIPS,
            ]
        );
    }

    const SUBMISSION: &str = "https://ilias.example/ilias/ilias.php?cmdClass=ilExSubmissionFileGUI&cmd=submissionScreen&ref_id=7&ass_id=1";

    /// The form for assignment 1, unless `upload` says otherwise.
    fn upload_form(upload: &str) -> String {
        format!(
            r##"<form action="ilias.php?cmdClass=ilExSubmissionFileGUI&amp;cmd=addUpload&amp;ref_id=7&amp;ass_id=1">
              <input type="hidden" name="form/file" /></form>
            <script>il.UI.Input.File.init('id', '{upload}', 'x');</script>"##
        )
    }

    const UPLOAD: &str =
        "ilias.php?cmdClass=ilRepoStandardUploadHandlerGUI&cmd=upload&ref_id=7&ass_id=1";

    /// ILIAS with a working upload form: answers the three steps by their command.
    fn hand_in_ilias(answer: &'static str) -> Arc<FakeIlias> {
        Arc::new(FakeIlias::new(move |url| {
            let cmd = url
                .query_pairs()
                .find(|(k, _)| k == "cmd")
                .map(|(_, v)| v.into_owned());
            match cmd.as_deref() {
                Some("uploadForm") => reply(url, &upload_form(UPLOAD)),
                Some("upload") => reply(url, answer),
                _ => reply(url, "<table><td>sheet &amp; notes.pdf</td></table>"),
            }
        }))
    }

    fn sheet(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ilias-tui-write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, "x").unwrap();
        path
    }

    #[test]
    fn hands_in_a_file_in_two_steps() {
        let ilias = hand_in_ilias(r#"{"status":1,"message":"","filename":"sheet & notes.pdf"}"#);
        let client = Client::with_pages(base(), ilias.clone());
        let listed = client
            .hand_in(SUBMISSION, &sheet("sheet & notes.pdf"))
            .unwrap();
        assert_eq!(listed.as_deref(), Some("sheet & notes.pdf"));
        assert_eq!(
            ilias.posts(&base()),
            [
                format!("{UPLOAD} <- file[0]=@sheet & notes.pdf"),
                "ilias.php?cmdClass=ilExSubmissionFileGUI&cmd=addUpload&ref_id=7&ass_id=1 <- form/file=sheet & notes.pdf".to_string(),
            ]
        );
    }

    #[test]
    fn a_dry_run_hands_in_nothing() {
        let ilias = hand_in_ilias(r#"{"status":1}"#);
        let client = Client::with_pages(base(), ilias.clone());
        client.set_dry_run(true);
        assert_eq!(client.hand_in(SUBMISSION, &sheet("a.pdf")).unwrap(), None);
        assert!(ilias.posts(&base()).is_empty());
    }

    #[test]
    fn says_when_ilias_refuses_the_file() {
        let ilias = hand_in_ilias(r#"{"status":2,"message":"Too many files"}"#);
        let client = Client::with_pages(base(), ilias.clone());
        let error = client.hand_in(SUBMISSION, &sheet("a.pdf")).unwrap_err();
        assert!(error.to_string().contains("Too many files"), "{error}");
        assert_eq!(ilias.posts(&base()).len(), 1, "the form isn't finished");

        // A login page instead of the answer.
        let ilias = hand_in_ilias("<html>Log in</html>");
        let client = Client::with_pages(base(), ilias);
        assert!(client.hand_in(SUBMISSION, &sheet("a.pdf")).is_err());
    }

    #[test]
    fn sends_nothing_without_a_form_for_this_assignment() {
        // The deadline has passed: no form.
        let ilias = Arc::new(FakeIlias::new(|url| reply(url, "<p>Time is up.</p>")));
        let client = Client::with_pages(base(), ilias.clone());
        assert!(client.hand_in(SUBMISSION, &sheet("a.pdf")).is_err());
        assert!(ilias.posts(&base()).is_empty());

        // The form's file goes to another assignment or another site.
        for upload in [
            "ilias.php?cmdClass=ilRepoStandardUploadHandlerGUI&cmd=upload&ref_id=7&ass_id=2",
            "https://evil.example/ilias.php?cmdClass=ilRepoStandardUploadHandlerGUI&cmd=upload&ref_id=7&ass_id=1",
            "ilias.php?cmdClass=ilRepoStandardUploadHandlerGUI&cmd=remove&ref_id=7&ass_id=1",
        ] {
            let form = upload_form(upload);
            let ilias = Arc::new(FakeIlias::new(move |url| reply(url, &form)));
            let client = Client::with_pages(base(), ilias.clone());
            assert!(
                client.hand_in(SUBMISSION, &sheet("a.pdf")).is_err(),
                "{upload}"
            );
            assert!(ilias.posts(&base()).is_empty(), "{upload}");
        }

        // Not a submission page in the first place.
        let client = Client::with_pages(base(), hand_in_ilias(""));
        assert!(client.hand_in(ADD, &sheet("a.pdf")).is_err());
    }

    const POST: &str =
        "ilias.php?cmdClass=ilExSubmissionFileGUI&cmd=post&ref_id=7&ass_id=1&rtoken=t";

    /// Handed-in file 2 of assignment 1, as listed on its submission page.
    fn handed_in() -> Item {
        Item {
            kind: "file".into(),
            title: "Bericht.pdf".into(),
            url: "https://ilias.example/ilias/ilias.php?cmdClass=ilExSubmissionFileGUI&cmd=download&ref_id=7&ass_id=1&delivered=2".into(),
            ..Item::default()
        }
    }

    /// The submission page with file 2 and a Delete button whose form posts to `action`.
    fn listing(action: &str) -> String {
        format!(
            r#"<form action="{action}"><input type="checkbox" name="delivered[]" value="2">
            <input type="submit" name="cmd[confirmDeleteDelivered]" value="Delete"></form>"#
        )
    }

    /// ILIAS that lists file 2, asks to confirm, then lists `after`.
    fn delete_ilias(after: &'static str) -> Arc<FakeIlias> {
        let posts = std::sync::atomic::AtomicUsize::new(0);
        Arc::new(FakeIlias::new(move |url| {
            if !url.as_str().contains("cmd=post") {
                return reply(url, &listing(&POST.replace('&', "&amp;")));
            }
            match posts.fetch_add(1, Ordering::SeqCst) {
                0 => reply(
                    url,
                    &format!(
                        r#"<form action="{}"><input type="hidden" name="delivered[]" value="2">
                        <input type="submit" name="cmd[deleteDelivered]" value="Delete"></form>"#,
                        POST.replace('&', "&amp;")
                    ),
                ),
                _ => reply(url, after),
            }
        }))
    }

    #[test]
    fn deletes_a_handed_in_file_after_confirming_like_ilias_asks() {
        let ilias = delete_ilias("<p>No files</p>");
        let client = Client::with_pages(base(), ilias.clone());
        assert!(client.delete_submission(&handed_in()).unwrap());
        assert_eq!(
            ilias.requests(&base()),
            ["ilias.php?cmdClass=ilExSubmissionFileGUI&ref_id=7&ass_id=1&cmd=submissionScreen"]
        );
        assert_eq!(
            ilias.posts(&base()),
            [
                format!("{POST} <- delivered[]=2&cmd[confirmDeleteDelivered]=Delete"),
                format!("{POST} <- delivered[]=2&cmd[deleteDelivered]=Delete"),
            ]
        );
    }

    #[test]
    fn a_dry_run_deletes_nothing() {
        let ilias = delete_ilias("");
        let client = Client::with_pages(base(), ilias.clone());
        client.set_dry_run(true);
        assert!(!client.delete_submission(&handed_in()).unwrap());
        assert!(ilias.posts(&base()).is_empty());
    }

    #[test]
    fn says_when_ilias_still_lists_the_file() {
        let still = r#"<input type="checkbox" name="delivered[]" value="2">"#;
        let client = Client::with_pages(base(), delete_ilias(still));
        let error = client.delete_submission(&handed_in()).unwrap_err();
        assert!(error.to_string().contains("still lists"), "{error}");
    }

    #[test]
    fn deletes_nothing_without_a_delete_button_for_this_file() {
        // After the deadline ILIAS shows the file without a Delete button.
        let page =
            r#"<form action="x"><input type="checkbox" name="delivered[]" value="2"></form>"#;
        let ilias = Arc::new(FakeIlias::new(move |url| reply(url, page)));
        let client = Client::with_pages(base(), ilias.clone());
        let error = client.delete_submission(&handed_in()).unwrap_err();
        assert!(error.to_string().contains("doesn't let you"), "{error}");
        assert!(ilias.posts(&base()).is_empty());

        // The form posts to another assignment, another site or another command.
        for action in [
            "ilias.php?cmdClass=ilExSubmissionFileGUI&amp;cmd=post&amp;ref_id=7&amp;ass_id=9",
            "https://evil.example/ilias/ilias.php?cmdClass=ilExSubmissionFileGUI&amp;cmd=post&amp;ref_id=7&amp;ass_id=1",
            "ilias.php?cmdClass=ilExSubmissionFileGUI&amp;cmd=deleteAll&amp;ref_id=7&amp;ass_id=1",
        ] {
            let page = listing(action);
            let ilias = Arc::new(FakeIlias::new(move |url| reply(url, &page)));
            let client = Client::with_pages(base(), ilias.clone());
            assert!(client.delete_submission(&handed_in()).is_err(), "{action}");
            assert!(ilias.posts(&base()).is_empty(), "{action}");
        }

        // Not a handed-in file: an attachment of the assignment.
        let attachment = Item {
            url: "https://ilias.example/ilias/ilias.php?cmdClass=ilExSubmissionFileGUI&cmd=downloadFile&ref_id=7&ass_id=1".into(),
            ..handed_in()
        };
        let client = Client::with_pages(base(), delete_ilias(""));
        assert!(client.delete_submission(&attachment).is_err());
    }

    #[test]
    fn a_favourites_link_keeps_the_scheme_of_ilias() {
        let ilias = Arc::new(FakeIlias::new(|url| reply(url, "<p>ok</p>")));
        let client = Client::with_pages(base(), ilias.clone());
        let plain = "http://ilias.example/ilias/ilias.php?cmd=addToDesk&type=crs&item_ref_id=7";
        assert!(client.toggle_favorite(&course(plain)).is_err());
        assert!(ilias.requests(&base()).is_empty());

        // A local test ILIAS runs on plain http: there it's the expected scheme.
        let local = Url::parse("http://localhost:8080/").unwrap();
        let ilias = Arc::new(FakeIlias::new(|url| reply(url, "<p>ok</p>")));
        let client = Client::with_pages(local, ilias);
        let link = "http://localhost:8080/ilias.php?cmd=addToDesk&type=crs&item_ref_id=7";
        assert!(client.toggle_favorite(&course(link)).unwrap());
    }

    #[test]
    fn knows_writes_by_command_or_token() {
        let url =
            |q: &str| Url::parse(&format!("https://ilias.example/ilias/ilias.php?{q}")).unwrap();
        assert!(is_write(&url("cmd=removeFromDesk&item_ref_id=1")));
        assert!(is_write(&url("cmd=anythingNew&rtoken=abc")));
        assert!(!is_write(&url("cmd=viewThread&thr_pk=1")));
        assert!(!is_write(&url(
            "cmdClass=ilObjFileGUI&cmd=sendfile&ref_id=1"
        )));
    }
}
