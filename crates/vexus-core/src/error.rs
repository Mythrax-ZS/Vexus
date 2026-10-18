//! Error type for the engine, with user-facing wording kept next to the technical detail.

use std::io;
use std::path::PathBuf;

use serde::Serialize;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the operation was cancelled")]
    Cancelled,

    #[error("another scan or clean is already running")]
    Busy,

    #[error("that scan is no longer available")]
    UnknownScan,

    #[error("unknown category `{0}`")]
    UnknownCategory(String),

    #[error("`{}` is protected and can never be removed", .0.display())]
    Protected(PathBuf),

    #[error("recovery batch `{0}` was not found")]
    UnknownBatch(String),

    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: io::Error,
    },

    #[error("stored data is damaged: {0}")]
    Corrupt(String),

    #[error("{0}")]
    Invalid(String),
}

impl Error {
    pub fn io(context: impl Into<String>, source: io::Error) -> Self {
        Error::Io { context: context.into(), source }
    }

    /// Stable machine-readable code (the UI switches on this, never on the message).
    pub fn code(&self) -> &'static str {
        match self {
            Error::Cancelled => "cancelled",
            Error::Busy => "busy",
            Error::UnknownScan => "unknown-scan",
            Error::UnknownCategory(_) => "unknown-category",
            Error::Protected(_) => "protected",
            Error::UnknownBatch(_) => "unknown-batch",
            Error::Io { source, .. } if source.kind() == io::ErrorKind::PermissionDenied => {
                "access-denied"
            }
            Error::Io { .. } => "io",
            Error::Corrupt(_) => "corrupt",
            Error::Invalid(_) => "invalid",
        }
    }

    /// Plain-language explanation suitable for showing to a person.
    pub fn to_user(&self) -> UserError {
        let (title, detail) = match self {
            Error::Cancelled => {
                ("Cancelled", "Nothing was changed after you stopped it.".to_string())
            }
            Error::Busy => (
                "Vexus is busy",
                "Another scan or clean is still running. Wait for it to finish or cancel it."
                    .into(),
            ),
            Error::UnknownScan => (
                "That scan has expired",
                "Run a new scan so Vexus can show you an up-to-date list before cleaning.".into(),
            ),
            Error::UnknownCategory(id) => {
                ("Unknown category", format!("Vexus doesn't have a cleaner called “{id}”."))
            }
            Error::Protected(p) => (
                "Protected location",
                format!("{} is protected by Vexus and will never be removed.", p.display()),
            ),
            Error::UnknownBatch(_) => {
                ("Recovery copy not found", "It may have expired or been freed already.".into())
            }
            Error::Io { context, source } if source.kind() == io::ErrorKind::PermissionDenied => (
                "Windows blocked access",
                format!("{context}. Restart Vexus as administrator to include protected areas."),
            ),
            Error::Io { context, source } => {
                ("Something went wrong", format!("{context}: {source}"))
            }
            Error::Corrupt(what) => (
                "Stored data is damaged",
                format!("{what}. Your files are untouched; Vexus will ignore the damaged record."),
            ),
            Error::Invalid(msg) => ("Invalid request", msg.clone()),
        };
        UserError { code: self.code(), title: title.to_string(), detail }
    }
}

impl From<io::Error> for Error {
    fn from(source: io::Error) -> Self {
        Error::Io { context: "File system error".into(), source }
    }
}

/// Serializable error shown in the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserError {
    pub code: &'static str,
    pub title: String,
    pub detail: String,
}
