//! The errors of the store actions.

/// A store action that cannot run. The text is the message of Desktop's API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// No chat has the chat ID or the local chat ID.
    ChatNotFound,
    /// The chat has no message with the message ID.
    MessageNotFound,
    /// The message has no reaction of the user with the reaction key.
    ReactionNotFound,
    /// A new draft would replace a draft that is not empty.
    DraftNotEmpty,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ChatNotFound => "Chat not found",
            Self::MessageNotFound => "Message not found",
            Self::ReactionNotFound => "Reaction not found",
            Self::DraftNotEmpty => {
                "Cannot overwrite a non-empty draft. Clear the draft first by setting draft to null or draft.text to an empty string."
            }
        })
    }
}

impl std::error::Error for Error {}
