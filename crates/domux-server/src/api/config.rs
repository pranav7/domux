//! `config.reload`: rereads domux.toml. A bad file keeps the previous good config and
//! reports the line; the keymap follows the config.

use super::{ok, Ctx};
use crate::load_config;
use domux_core::api::{ApiError, ConfigReloadResult, Event};
use serde_json::Value;

pub fn reload(ctx: &mut Ctx) -> Result<Value, ApiError> {
    let loaded = load_config(&ctx.config.path);
    let error = loaded.error.as_ref().map(|e| e.to_string());
    let warnings = loaded.warnings.clone();
    if loaded.error.is_none() {
        *ctx.config = loaded;
        // The config that made the respawn guard trip is gone, so the block on the
        // workspaces it stopped goes with it.
        ctx.release_respawn_blocks = true;
    } else {
        ctx.config.error = loaded.error;
        ctx.config.warnings = loaded.warnings;
    }
    ctx.events.push(Event::ConfigReloaded {
        error: error.clone(),
    });
    // The top bar shows a config error and the keys the keymap names.
    ctx.view_dirty = true;
    ok(ConfigReloadResult { error, warnings })
}
