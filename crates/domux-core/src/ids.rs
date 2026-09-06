//! Opaque ids: a prefix, an underscore and four lowercase hex characters. Message ids are
//! sequential (`m1`, `m2`). Each id is a newtype so the compiler keeps tabs and panes apart.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

macro_rules! id_type {
    ($name:ident, $prefix:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
        )]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = IdError;
            fn from_str(s: &str) -> Result<Self, IdError> {
                match s.strip_prefix(concat!($prefix, "_")) {
                    Some(rest)
                        if rest.len() == 4
                            && rest
                                .chars()
                                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()) =>
                    {
                        Ok($name(s.to_string()))
                    }
                    _ => Err(IdError {
                        expected: concat!($prefix, "_"),
                        got: s.to_string(),
                    }),
                }
            }
        }
    };
}

id_type!(ProjectId, "pr", "A project id, `pr_19f0`.");
id_type!(WorkspaceId, "w", "A workspace id, `w_c3a1`.");
id_type!(TabId, "t", "A tab id, `t_41b2`.");
id_type!(PaneId, "p", "A pane id, `p_8f2a`.");
id_type!(ClientId, "c", "An attached client's id, `c_0d77`.");
id_type!(AgentId, "a", "An agent id, `a_5e21`.");

/// A message id: `m` followed by a sequence number.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct MessageId(pub String);

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for MessageId {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, IdError> {
        match s.strip_prefix('m') {
            Some(rest) if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) => {
                Ok(MessageId(s.to_string()))
            }
            _ => Err(IdError {
                expected: "m",
                got: s.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("expected an id starting with {expected}, got {got:?}")]
pub struct IdError {
    pub expected: &'static str,
    pub got: String,
}

/// Every id prefix, for target resolution and error messages.
pub const PREFIXES: &[&str] = &["pr", "w", "t", "p", "c", "a"];

/// Four random hex characters from a seeded xorshift64* generator. The server seeds it from
/// the operating system; tests seed it with a constant so ids in frames are predictable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdGen {
    state: u64,
}

impl IdGen {
    pub fn from_seed(seed: u64) -> IdGen {
        // Guard the all-zero xorshift state without collapsing adjacent seeds: `seed | 1`
        // would make `from_seed(42)` and `from_seed(43)` produce identical streams.
        IdGen {
            state: if seed == 0 { 1 } else { seed },
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn hex4(&mut self) -> String {
        format!("{:04x}", (self.next_u64() >> 48) as u16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_display_as_prefix_underscore_hex() {
        let id = PaneId("p_8f2a".to_string());
        assert_eq!(id.to_string(), "p_8f2a");
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"p_8f2a\"");
        let back: PaneId = serde_json::from_str("\"p_8f2a\"").unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn message_id_round_trips_through_json_as_a_bare_string() {
        let id = MessageId("m7".to_string());
        assert_eq!(id.to_string(), "m7");
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"m7\"");
        let back: MessageId = serde_json::from_str("\"m7\"").unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn from_str_checks_the_prefix() {
        assert!("p_8f2a".parse::<PaneId>().is_ok());
        assert!("t_8f2a".parse::<PaneId>().is_err());
        assert!("w_c3a1".parse::<WorkspaceId>().is_ok());
        assert!("pr_19f0".parse::<ProjectId>().is_ok());
        assert!("c_0d77".parse::<ClientId>().is_ok());
        assert!("m7".parse::<MessageId>().is_ok());
        assert!("x7".parse::<MessageId>().is_err());
    }

    #[test]
    fn idgen_is_deterministic_for_a_seed_and_yields_four_hex_chars() {
        let mut a = IdGen::from_seed(42);
        let mut b = IdGen::from_seed(42);
        let x = a.hex4();
        assert_eq!(x, b.hex4());
        assert_eq!(x.len(), 4);
        assert!(x
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(a.hex4(), x);
    }
}
