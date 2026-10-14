//! Error types for the mail server.

use thiserror::Error;

/// Main error type for mail operations.
#[derive(Error, Debug)]
pub enum MailError {
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    #[error("Account not found: {0}")]
    AccountNotFound(String),

    #[error("Mailbox not found: {0}")]
    MailboxNotFound(String),

    #[error("Message not found: UID {0}")]
    MessageNotFound(u64),

    #[error("Mailbox already exists: {0}")]
    MailboxExists(String),

    #[error("Quota exceeded: {used} / {max} bytes")]
    QuotaExceeded { used: u64, max: u64 },

    #[error("Message too large: {size} bytes (max: {max})")]
    MessageTooLarge { size: usize, max: usize },

    #[error("Too many recipients: {count} (max: {max})")]
    TooManyRecipients { count: usize, max: usize },

    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    #[error("Invalid address: {0}")]
    InvalidAddress(String),

    #[error("Invalid mailbox name: {0}")]
    InvalidMailboxName(String),

    #[error("Protocol error: {0}")]
    ProtocolError(String),

    #[error("TLS required")]
    TlsRequired,

    #[error("Spam rejected: score {score}")]
    SpamRejected { score: f32 },

    #[error("Delivery failed: {0}")]
    DeliveryFailed(String),

    #[error("Temporary failure: {0}")]
    TemporaryFailure(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl MailError {
    /// Get SMTP reply code for this error.
    pub fn smtp_code(&self) -> u16 {
        match self {
            Self::AuthenticationFailed(_) => 535,
            Self::AccountNotFound(_) => 550,
            Self::MailboxNotFound(_) => 550,
            Self::MessageNotFound(_) => 550,
            Self::MailboxExists(_) => 550,
            Self::QuotaExceeded { .. } => 552,
            Self::MessageTooLarge { .. } => 552,
            Self::TooManyRecipients { .. } => 452,
            Self::RateLimitExceeded => 421,
            Self::InvalidAddress(_) => 501,
            Self::InvalidMailboxName(_) => 501,
            Self::ProtocolError(_) => 500,
            Self::TlsRequired => 530,
            Self::SpamRejected { .. } => 550,
            Self::DeliveryFailed(_) => 550,
            Self::TemporaryFailure(_) => 451,
            Self::Database(_) => 451,
            Self::Io(_) => 451,
            Self::Internal(_) => 451,
        }
    }

    /// Check if this is a temporary failure (should retry).
    pub fn is_temporary(&self) -> bool {
        matches!(
            self,
            Self::RateLimitExceeded
                | Self::TemporaryFailure(_)
                | Self::Database(_)
                | Self::Io(_)
                | Self::Internal(_)
        )
    }
}

/// Shorthand Result type.
pub type Result<T> = std::result::Result<T, MailError>;
