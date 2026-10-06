//! Original production patch-chain opener, moved out of UI consumers.

use benilla_formats::Chain;
use std::sync::OnceLock;
use tracing::{debug, error, warn};

pub(super) fn read(req: &str) -> Option<Vec<u8>> {
    let chain = chain()?;
    match chain.read(req) {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            debug!("ui_script: {req} is not in the patch chain: {e:#}");
            None
        }
    }
}

fn chain() -> Option<&'static Chain> {
    static CHAIN: OnceLock<Option<Chain>> = OnceLock::new();
    CHAIN.get_or_init(|| {
        let Some(data) = benilla_formats::wow_data() else {
            warn!("ui_script: no client data — every interface file this client SOURCES off the player's install (benilla.toc's `Interface\\…` entries) is absent, so the windows they build do not exist and addons that call their globals will raise");
            return None;
        };
        match benilla_formats::open_chain(&data) {
            Ok(chain) => Some(chain),
            Err(e) => {
                error!("ui_script: opening the patch chain to source the reference UI: {e:#}");
                None
            }
        }
    }).as_ref()
}

pub(super) fn try_read(req: &str) -> anyhow::Result<Vec<u8>> {
    chain()
        .ok_or_else(|| anyhow::anyhow!("no installed patch chain"))?
        .read(req)
}

pub(super) fn contains(req: &str) -> anyhow::Result<bool> {
    Ok(chain()
        .ok_or_else(|| anyhow::anyhow!("no installed patch chain"))?
        .contains(req))
}
