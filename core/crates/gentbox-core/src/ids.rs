//! Identifier newtypes. Ids are hub-assigned ULIDs; the hub generates them
//! (randomness and the clock are IO), the core only carries them.

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! string_id {
	($(#[$meta:meta])* $name:ident) => {
		$(#[$meta])*
		#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
		#[serde(transparent)]
		pub struct $name(String);

		impl $name {
			/// Wraps an id string produced by the hub.
			pub fn new(value: impl Into<String>) -> Self {
				Self(value.into())
			}

			/// The id as a string slice.
			pub fn as_str(&self) -> &str {
				&self.0
			}
		}

		impl fmt::Display for $name {
			fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
				formatter.write_str(&self.0)
			}
		}
	};
}

string_id!(
    /// The hub-assigned id of a message: the only message id in URLs and in
    /// `actionResponse.messageId`.
    MessageId
);
string_id!(
    /// The hub-assigned id of a topic.
    TopicId
);
string_id!(
    /// The hub-assigned id of a thread.
    ThreadId
);

/// A conversation is one agent session; its id is only unique per agent.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConversationKey {
    /// `source.agent`.
    pub agent: String,
    /// `conversation.id`.
    pub conversation_id: String,
}

impl ConversationKey {
    /// Builds a key from its two parts.
    pub fn new(agent: impl Into<String>, conversation_id: impl Into<String>) -> Self {
        Self {
            agent: agent.into(),
            conversation_id: conversation_id.into(),
        }
    }
}
