// SPDX-License-Identifier: Apache-2.0

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const EMBEDDING_DIM: usize = 1024;

#[derive(Debug, thiserror::Error)]
#[error("unknown {kind} `{value}`; expected one of: {expected}")]
pub struct ParseError {
    kind: &'static str,
    value: String,
    expected: String,
}

macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident, $kind:literal, { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name { $($variant),+ }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $($name::$variant => $text),+ }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = ParseError;

            fn from_str(s: &str) -> Result<Self, ParseError> {
                match s {
                    $($text => Ok($name::$variant),)+
                    _ => Err(ParseError {
                        kind: $kind,
                        value: s.to_owned(),
                        expected: [$($text),+].join(", "),
                    }),
                }
            }
        }
    };
}

string_enum!(
    /// The one advertised set of memory types; the CLI and MCP surface this enum.
    MemoryType, "memory type", {
        Fact => "fact",
        Decision => "decision",
        Problem => "problem",
        Fix => "fix",
        Procedure => "procedure",
        Gotcha => "gotcha",
    }
);

string_enum!(
    /// Typed, directed relationship from one memory to another.
    LinkType, "link type", {
        Supersedes => "SUPERSEDES",
        Solves => "SOLVES",
        Causes => "CAUSES",
        Contradicts => "CONTRADICTS",
    }
);

#[derive(Debug, Clone, PartialEq)]
pub struct Embedding {
    pub vector: Vec<f32>,
    pub model_id: String,
    pub model_digest: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewMemory {
    pub memory_type: MemoryType,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub project: String,
    pub importance: f32,
    pub embedding: Option<Embedding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Memory {
    pub id: Uuid,
    pub memory_type: MemoryType,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub project: String,
    pub importance: f32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub embedding_model: Option<String>,
    pub embedding_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub from_id: Uuid,
    pub to_id: Uuid,
    pub link_type: LinkType,
    pub strength: f32,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_round_trip_and_reject_unknown() {
        for t in MemoryType::ALL {
            assert_eq!(t.as_str().parse::<MemoryType>().unwrap(), *t);
        }
        for t in LinkType::ALL {
            assert_eq!(t.as_str().parse::<LinkType>().unwrap(), *t);
        }
        let err = "general".parse::<MemoryType>().unwrap_err().to_string();
        assert!(err.contains("fact, decision, problem, fix, procedure, gotcha"));
    }
}
