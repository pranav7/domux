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
    } else {
        ctx.config.error = loaded.error;
        ctx.config.warnings = loaded.warnings;
    }
    ctx.events.push(Event::ConfigReloaded {
        error: error.clone(),
    });
    ok(ConfigReloadResult { error, warnings })
}
