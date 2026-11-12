//! The current organization: its key in Telemost settings, its name and avatar, and whether a
//! chat belongs to it.

use leptos::prelude::*;
use mostik_core::*;

use super::state::Ctx;

pub(super) const ORG_KEY: &str = "mostik.org";

/// The key of the current organization in Telemost settings such as chat folders: its id, "0"
/// without organizations.
pub fn org_key(s: &Snapshot, chosen: Option<u64>) -> String {
    s.organizations
        .iter()
        .find(|o| Some(o.id) == chosen)
        .or(s.organizations.first())
        .map_or_else(|| "0".to_owned(), |o| o.id.to_string())
}

/// Current organization: the selected one, or the first if none is selected.
pub(crate) fn current_org(c: Ctx) -> Option<Organization> {
    let chosen = c.org.get();
    c.snap.with(|s| {
        s.organizations
            .iter()
            .find(|o| Some(o.id) == chosen)
            .or(s.organizations.first())
            .cloned()
    })
}

pub(super) fn org_name(c: Ctx) -> String {
    current_org(c)
        .map(|o| o.name)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| c.snap.with(|s| s.organization.clone()))
}

pub(super) fn org_avatar(c: Ctx) -> Option<String> {
    current_org(c)
        .and_then(|o| o.avatar)
        .or_else(|| c.snap.with(|s| s.organization_avatar.clone()))
}

/// The chat is visible in the selected organization: direct chats always, the rest if they belong to it.
/// The filter applies only when there is more than one organization.
pub(super) fn in_current_org(c: Ctx, ch: &Chat) -> bool {
    let many = c.snap.with(|s| s.organizations.len() > 1);
    if !many || ch.organizations.is_empty() {
        return true;
    }
    current_org(c).is_none_or(|o| ch.organizations.contains(&o.id))
}
