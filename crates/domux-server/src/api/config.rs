//! `config.reload`: rereads domux.toml. A bad file keeps the previous good config and
//! reports the line; the keymap follows the config.

use super::{ok, Ctx};
use crate::client::Hint;
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
    // A reload that loaded changes nothing the eye can see, so with no word for it the key
    // looks dead (principle 8). A reload that failed needs no word from here: the config
    // error already stands in the top bar naming the file, the line and the way to fix it,
    // which is more than this line has room to repeat.
    if error.is_none() {
        if let Some(client) = ctx
            .client
            .clone()
            .or_else(|| ctx.model.most_recent_client())
        {
            if let Some(conn) = ctx.clients.get_mut(&client) {
                conn.hint = Some(Hint::action(reload_notice(&warnings)));
            }
        }
    }
    // The top bar shows a config error and the keys the keymap names.
    ctx.view_dirty = true;
    ok(ConfigReloadResult { error, warnings })
}

/// What a reload that loaded has to say, in one line. Warnings are counted rather than
/// quoted: the count is what says whether to go and read them, and this line shares the top
/// bar with the clock.
fn reload_notice(warnings: &[String]) -> String {
    match warnings.len() {
        0 => "config reloaded".into(),
        1 => "config reloaded, 1 warning".into(),
        n => format!("config reloaded, {n} warnings"),
    }
}
