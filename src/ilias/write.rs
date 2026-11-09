//! Every write (a request that changes data on ILIAS) lives here, so they are easy to review
//! in one place (see "Rules for features that write" in `docs/roadmap.md`). All other page
//! requests pass the write guard ([`is_write`]), which refuses anything that looks like a
//! write. The TUI asks before each favourites change and each file it hands in
//! ([`Action::Ask`](crate::action::Action::Ask)), and `ILIAS_DRY_RUN=1` turns those into
//! no-ops.

use std::sync::atomic::Ordering;

use color_eyre::{Result, eyre::bail};
use url::Url;

use super::{Client, Item, MEMBERSHIPS, parse};

mod container;
mod exercise;
mod forum;

pub use container::megabytes;

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
    "saveVersion",
    "deleteVersions",
    "rollbackVersion",
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
            && param("item_ref_id") == Some(item.key().to_string());
        if !expected {
            bail!("unexpected favourites link for {}: {url}", item.title);
        }
        if self.dry_run.load(Ordering::Relaxed) {
            return Ok(false);
        }
        self.authed(&url, true)?;
        Ok(true)
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
        self.request(&url, None, true)?;
        let after = self.get(&self.url(MEMBERSHIPS)?)?;
        Ok(!self.is_logged_in(&after))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::ilias::Id;
    use crate::ilias::pages::{FakeIlias, LOGGED_OUT, reply};

    fn base() -> Url {
        Url::parse("https://ilias.example/ilias/").unwrap()
    }

    fn course(link: &str) -> Item {
        Item {
            id: Id::Ref(7),
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
