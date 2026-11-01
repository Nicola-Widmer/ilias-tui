//! Every request that changes data on ILIAS lives here, so they are easy to review in one
//! place (see "Rules for features that write" in `docs/roadmap.md`). The TUI asks before
//! each one ([`Action::Ask`](crate::action::Action::Ask)), and `ILIAS_DRY_RUN=1` turns them
//! all into no-ops.

use color_eyre::{Result, eyre::bail};
use url::Url;

use super::{Client, Item};

/// `ILIAS_DRY_RUN=1`: pretend, never send a write.
pub fn dry_run() -> bool {
    matches!(
        std::env::var("ILIAS_DRY_RUN").as_deref(),
        Ok("1" | "true" | "yes" | "on")
    )
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
        let expected = url.scheme() == "https"
            && url.host_str() == self.base.host_str()
            && matches!(
                param("cmd").as_deref(),
                Some("addToDesk" | "removeFromDesk")
            )
            && param("item_ref_id") == Some(item.ref_id.to_string());
        if !expected {
            bail!("unexpected favourites link for {}: {url}", item.title);
        }
        if dry_run() {
            return Ok(false);
        }
        self.get_authed(&url)?;
        Ok(true)
    }
}
