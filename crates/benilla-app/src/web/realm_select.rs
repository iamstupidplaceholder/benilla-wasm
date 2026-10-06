use bevy::prelude::*;

/// Login only needs to know whether the realm overlay owns input.  The full
/// production realm-list module replaces this adapter with the web transport.
#[derive(Resource, Default)]
pub(crate) struct Realms {
    pub(crate) shown: bool,
}

pub(crate) fn pvp_rp(realm_type: u32) -> (bool, bool) {
    match realm_type {
        0 | 2 | 4 => (false, false),
        1 | 3 | 5 => (true, false),
        6 | 7 => (false, true),
        8 => (true, true),
        _ => (false, false),
    }
}

pub(crate) const fn change_realm_available() -> bool {
    false
}

pub(crate) fn open_over_char_select(realms: &mut Realms, choice: &crate::net::RealmChoice) {
    // The current gateway has one fixed world upstream. Keep the production
    // button contract, but do not fabricate a second realm screen here.
    realms.shown = false;
    let _ = choice.0.send(crate::net::RealmRequest::Refresh);
}
