//! `stay_awake.enable`, `stay_awake.disable` and `stay_awake.toggle`: whether domux holds
//! this machine awake. One hold for the server, so none of the three names a target and none
//! takes a client.

use super::{ok, Ctx};
use crate::toast::Toast;
use domux_core::api::{ApiError, Event, StayAwakeResult};
use serde_json::Value;

pub fn enable(ctx: &mut Ctx) -> Result<Value, ApiError> {
    set(ctx, true)
}

pub fn disable(ctx: &mut Ctx) -> Result<Value, ApiError> {
    set(ctx, false)
}

pub fn toggle(ctx: &mut Ctx) -> Result<Value, ApiError> {
    let want = !ctx.stay_awake.on();
    set(ctx, want)
}

/// A machine this domux cannot hold awake answers `on: false` with the reason rather than an
/// error: there is nothing the reader can do about the platform (principle 9), and a startup
/// script that turns stay awake on should not fail on a machine where the feature does not
/// exist.
fn set(ctx: &mut Ctx, want: bool) -> Result<Value, ApiError> {
    let was = ctx.stay_awake.on();
    let mode = ctx.config.config.stay_awake.mode;
    let platform = ctx.deps.platform.clone();
    let runner = ctx.deps.runner.clone();
    let outcome = if want {
        ctx.stay_awake.enable(mode, &platform, runner.as_ref())
    } else {
        ctx.stay_awake.disable(mode, &platform, runner.as_ref())
    };
    let note = match outcome {
        Ok(note) => note,
        Err(reason) => Some(reason),
    };
    let on = ctx.stay_awake.on();
    let now = ctx.deps.clock.now();
    if on != was {
        // The model remembers the switch, so the next server takes the hold again. It follows
        // the hold rather than the request: a hold that could not be taken is not a hold, and
        // a state file saying otherwise would keep trying at every start.
        ctx.model.stay_awake = on;
        ctx.events.push(Event::StayAwakeChanged { on });
        let mut toast = Toast::new(
            if on {
                "Stay awake turned on"
            } else {
                "Stay awake turned off"
            },
            now,
        );
        if let Some(note) = &note {
            toast = toast.and(note.clone());
        }
        ctx.toasts.push(toast);
        ctx.view_dirty = true;
    } else if let Some(note) = &note {
        // Nothing changed and there is a reason. The key that asked has no other answer:
        // the dot is where it was and a reply reaches nobody who pressed a key (principle 8).
        ctx.toasts.push(Toast::new(note.clone(), now));
        ctx.view_dirty = true;
    }
    ok(StayAwakeResult { on, note })
}
