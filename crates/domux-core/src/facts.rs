//! Facts: what domux observed and cached with a time to live, as against state, which is
//! what domux decided and persists (architecture spec section 2). A fact that did not
//! arrive is absent. Nothing here fetches anything: the providers live in the server.

use crate::ids::{ProjectId, WorkspaceId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use std::time::Duration;

/// The branch a workspace is on. Dropped from the row when it equals the handle.
pub const FACT_BRANCH: &str = "branch";
/// The pull request for a workspace's branch: `PR#212`, coloured by its state.
pub const FACT_PR: &str = "pr";

/// What a fact is about. A provider declares which scope it wants and the registry walks
/// the model to build one target per object in that scope (architecture spec section 8).
///
/// Adjacently tagged, for the same reason M1's `Focus` is: `Workspace` and `Project` are
/// newtype variants over a string id, and serde cannot represent a newtype variant with an
/// internal tag. Internal tagging compiles and then fails at run time on the first
/// `serialize`. `tag`/`content` gives `{"scope":"workspace","value":"w_c3a1"}`, which
/// serializes and round trips.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "scope", content = "value", rename_all = "snake_case")]
pub enum FactScope {
    Workspace(WorkspaceId),
    Project(ProjectId),
    Server,
}

/// One fact slot: what it is about and which provider fills it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FactKey {
    pub scope: FactScope,
    /// The provider's name and the second half of the key: `branch`, `pr`, or whatever an
    /// extension registers.
    pub name: String,
}

impl FactKey {
    pub fn workspace(id: &WorkspaceId, name: &str) -> FactKey {
        FactKey {
            scope: FactScope::Workspace(id.clone()),
            name: name.to_string(),
        }
    }

    pub fn project(id: &ProjectId, name: &str) -> FactKey {
        FactKey {
            scope: FactScope::Project(id.clone()),
            name: name.to_string(),
        }
    }

    pub fn server(name: &str) -> FactKey {
        FactKey {
            scope: FactScope::Server,
            name: name.to_string(),
        }
    }

    /// The workspace this key is about, when it is about one.
    pub fn workspace_id(&self) -> Option<&WorkspaceId> {
        match &self.scope {
            FactScope::Workspace(id) => Some(id),
            _ => None,
        }
    }
}

impl fmt::Display for FactKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.scope {
            FactScope::Workspace(id) => write!(f, "{id}/{}", self.name),
            FactScope::Project(id) => write!(f, "{id}/{}", self.name),
            FactScope::Server => write!(f, "server/{}", self.name),
        }
    }
}

impl FromStr for FactKey {
    type Err = String;

    fn from_str(s: &str) -> Result<FactKey, String> {
        let (scope, name) = s
            .split_once('/')
            .ok_or_else(|| "a fact key is <scope>/<name>, for example w_c3a1/pr".to_string())?;
        if name.is_empty() {
            return Err("a fact key is <scope>/<name>, for example w_c3a1/pr".to_string());
        }
        if scope == "server" {
            return Ok(FactKey::server(name));
        }
        if let Ok(id) = scope.parse::<WorkspaceId>() {
            return Ok(FactKey::workspace(&id, name));
        }
        if let Ok(id) = scope.parse::<ProjectId>() {
            return Ok(FactKey::project(&id, name));
        }
        Err(format!(
            "{scope} is not a workspace id, a project id or the word server"
        ))
    }
}

impl Serialize for FactKey {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for FactKey {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<FactKey, D::Error> {
        let text = String::deserialize(d)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for FactKey {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "FactKey".into()
    }

    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        String::json_schema(g)
    }
}

/// The state a provider reports beside its text. The pull request provider uses the four
/// git forge states, in V1's spelling; an extension may report anything.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FactState {
    Open,
    Merged,
    Closed,
    Draft,
    Other(String),
}

impl fmt::Display for FactState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FactState::Open => f.write_str("OPEN"),
            FactState::Merged => f.write_str("MERGED"),
            FactState::Closed => f.write_str("CLOSED"),
            FactState::Draft => f.write_str("DRAFT"),
            FactState::Other(s) => f.write_str(s),
        }
    }
}

impl FromStr for FactState {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<FactState, Self::Err> {
        Ok(match s.trim().to_ascii_uppercase().as_str() {
            "OPEN" => FactState::Open,
            "MERGED" => FactState::Merged,
            "CLOSED" => FactState::Closed,
            "DRAFT" => FactState::Draft,
            other => FactState::Other(other.to_string()),
        })
    }
}

impl Serialize for FactState {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for FactState {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<FactState, D::Error> {
        let text = String::deserialize(d)?;
        Ok(text.parse().expect("FactState::from_str never fails"))
    }
}

impl JsonSchema for FactState {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "FactState".into()
    }

    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        String::json_schema(g)
    }
}

/// One observation. `text` is what renders; `state` colours it; `url` is where it leads.
/// `fetched_at` is RFC 3339 from the server's clock, and `ttl` is how long the value is
/// worth showing after a restart. Freshness takes the age as an argument so this crate
/// stays free of clocks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Fact {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<FactState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub fetched_at: String,
    #[serde(with = "ttl_secs")]
    #[schemars(with = "u64")]
    pub ttl: Duration,
}

impl Fact {
    pub fn new(
        text: impl Into<String>,
        state: Option<FactState>,
        fetched_at: impl Into<String>,
        ttl: Duration,
    ) -> Fact {
        Fact {
            text: text.into(),
            state,
            url: None,
            fetched_at: fetched_at.into(),
            ttl,
        }
    }

    pub fn with_url(mut self, url: impl Into<String>) -> Fact {
        self.url = Some(url.into());
        self
    }

    /// True while `age` (now minus `fetched_at`, computed by the caller) is inside the time
    /// to live.
    pub fn is_fresh(&self, age: Duration) -> bool {
        age < self.ttl
    }
}

/// Seconds on the wire, a `Duration` in memory.
mod ttl_secs {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_secs())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        Ok(Duration::from_secs(u64::deserialize(d)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ProjectId, WorkspaceId};
    use std::time::Duration;

    #[test]
    fn fact_keys_print_and_parse_back_for_every_scope() {
        let w = FactKey::workspace(&WorkspaceId("w_c3a1".into()), FACT_PR);
        assert_eq!(w.to_string(), "w_c3a1/pr");
        assert_eq!("w_c3a1/pr".parse::<FactKey>().unwrap(), w);
        let p = FactKey::project(&ProjectId("pr_19f0".into()), "ci");
        assert_eq!(p.to_string(), "pr_19f0/ci");
        assert_eq!("pr_19f0/ci".parse::<FactKey>().unwrap(), p);
        let s = FactKey::server("usage");
        assert_eq!(s.to_string(), "server/usage");
        assert_eq!("server/usage".parse::<FactKey>().unwrap(), s);
    }

    #[test]
    fn fact_key_parse_rejects_a_missing_name_or_an_unknown_prefix() {
        assert_eq!(
            "w_c3a1".parse::<FactKey>().unwrap_err(),
            "a fact key is <scope>/<name>, for example w_c3a1/pr"
        );
        assert_eq!(
            "x_0001/pr".parse::<FactKey>().unwrap_err(),
            "x_0001 is not a workspace id, a project id or the word server"
        );
    }

    #[test]
    fn fact_scopes_round_trip_through_json_for_every_variant() {
        // The regression this pins: an internally tagged enum with a newtype variant
        // compiles and then panics on the first serialize. Every variant must survive a
        // round trip, and the adjacent tag must be visible in the text.
        for scope in [
            FactScope::Workspace(WorkspaceId("w_c3a1".into())),
            FactScope::Project(ProjectId("pr_19f0".into())),
            FactScope::Server,
        ] {
            let text = serde_json::to_string(&scope).unwrap();
            assert_eq!(serde_json::from_str::<FactScope>(&text).unwrap(), scope);
        }
        assert_eq!(
            serde_json::to_string(&FactScope::Workspace(WorkspaceId("w_c3a1".into()))).unwrap(),
            r#"{"scope":"workspace","value":"w_c3a1"}"#
        );
        assert_eq!(
            serde_json::to_string(&FactScope::Server).unwrap(),
            r#"{"scope":"server"}"#
        );
    }

    #[test]
    fn pull_request_states_round_trip_and_keep_an_unknown_one() {
        assert_eq!("OPEN".parse::<FactState>().unwrap(), FactState::Open);
        assert_eq!("merged".parse::<FactState>().unwrap(), FactState::Merged);
        assert_eq!("CLOSED".parse::<FactState>().unwrap(), FactState::Closed);
        assert_eq!("DRAFT".parse::<FactState>().unwrap(), FactState::Draft);
        assert_eq!(
            "QUEUED".parse::<FactState>().unwrap(),
            FactState::Other("QUEUED".into())
        );
        assert_eq!(FactState::Draft.to_string(), "DRAFT");
        assert_eq!(FactState::Other("QUEUED".into()).to_string(), "QUEUED");
    }

    #[test]
    fn a_fact_is_fresh_until_its_time_to_live_passes() {
        let f = Fact::new(
            "PR#212",
            Some(FactState::Open),
            "2026-09-05T10:00:00+01:00",
            Duration::from_secs(600),
        );
        assert!(f.is_fresh(Duration::from_secs(0)));
        assert!(f.is_fresh(Duration::from_secs(599)));
        assert!(!f.is_fresh(Duration::from_secs(600)));
        assert_eq!(
            f.url, None,
            "a fact without a url has none, not an empty string"
        );
    }

    #[test]
    fn facts_round_trip_through_json_with_the_time_to_live_in_seconds() {
        let f = Fact::new(
            "feat/auth-cleanup",
            None,
            "2026-09-05T10:00:00+01:00",
            Duration::from_secs(5),
        );
        let text = serde_json::to_string(&f).unwrap();
        assert!(text.contains(r#""ttl":5"#), "{text}");
        assert!(
            !text.contains("state"),
            "an absent state is omitted: {text}"
        );
        // The cache file is written from this shape and read back by another run of the
        // server, so what a fact leaves out is a contract, not a formatting preference. An
        // absent value is absent; `null` in the file is a value that says a fact arrived and
        // carried nothing, which is not what happened (principle 4).
        assert!(
            !text.contains("url"),
            "an absent url is omitted too: {text}"
        );
        assert_eq!(serde_json::from_str::<Fact>(&text).unwrap(), f);
        let with_url = f.with_url("https://forge.invalid/audrey-app/pull/212");
        let text = serde_json::to_string(&with_url).unwrap();
        assert!(
            text.contains("/pull/212"),
            "and a url that is there is written: {text}"
        );
        assert_eq!(serde_json::from_str::<Fact>(&text).unwrap(), with_url);
    }
}
