use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! typed_counter {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(u64);

        impl $name {
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            pub const fn value(self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

typed_counter!(AgentSessionId);
typed_counter!(AgentTurnId);
typed_counter!(AgentGeneration);
typed_counter!(AgentRevision);
typed_counter!(AgentEventSequence);
typed_counter!(AgentCompactionId);
typed_counter!(AgentCompactionGeneration);
typed_counter!(AgentActivityId);
typed_counter!(AgentUserInputRequestId);
typed_counter!(AgentMessageId);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentContextWindowId(uuid::Uuid);

impl AgentContextWindowId {
    pub fn new() -> Self {
        Self(uuid::Uuid::now_v7())
    }

    pub const fn from_uuid(value: uuid::Uuid) -> Self {
        Self(value)
    }

    pub const fn value(self) -> uuid::Uuid {
        self.0
    }

    pub fn is_v7(self) -> bool {
        self.0.get_version_num() == 7
    }
}

impl Default for AgentContextWindowId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for AgentContextWindowId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentSteeringSequence(u64);

impl AgentSteeringSequence {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentModelSessionGeneration(u64);

impl AgentModelSessionGeneration {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}
