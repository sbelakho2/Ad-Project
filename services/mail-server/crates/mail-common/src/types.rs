//! Common types used across mail services.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Email address with optional display name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailAddress {
    pub name: Option<String>,
    pub address: String,
}

impl EmailAddress {
    /// Parse an email address string.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        
        // Handle "Name <email@domain>" format
        if let Some(start) = s.find('<') {
            if let Some(end) = s.find('>') {
                let name = s[..start].trim().trim_matches('"').to_string();
                let address = s[start + 1..end].trim().to_string();
                return Some(Self {
                    name: if name.is_empty() { None } else { Some(name) },
                    address,
                });
            }
        }
        
        // Simple email format
        if s.contains('@') && !s.contains(' ') {
            return Some(Self {
                name: None,
                address: s.to_string(),
            });
        }
        
        None
    }
    
    /// Get the domain part.
    pub fn domain(&self) -> Option<&str> {
        self.address.split('@').nth(1)
    }
    
    /// Get the local part.
    pub fn local_part(&self) -> Option<&str> {
        self.address.split('@').next()
    }
}

impl std::fmt::Display for EmailAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.name {
            Some(name) => write!(f, "\"{}\" <{}>", name, self.address),
            None => write!(f, "{}", self.address),
        }
    }
}

/// Message envelope (SMTP transaction info).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub mail_from: EmailAddress,
    pub rcpt_to: Vec<EmailAddress>,
    pub received_at: DateTime<Utc>,
    pub client_ip: Option<String>,
    pub helo: Option<String>,
    pub auth_user: Option<String>,
}

/// Message headers parsed from RFC 5322.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageHeaders {
    pub message_id: Option<String>,
    pub from: Vec<EmailAddress>,
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    pub bcc: Vec<EmailAddress>,
    pub subject: Option<String>,
    pub date: Option<DateTime<Utc>>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub content_type: Option<String>,
    pub extra: Vec<(String, String)>,
}

/// Computed hash of a message blob.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlobHash(pub String);

impl BlobHash {
    /// Compute SHA-256 hash of message bytes.
    pub fn compute(data: &[u8]) -> Self {
        let hash = Sha256::digest(data);
        Self(base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            hash,
        ))
    }
    
    /// Get as string reference.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BlobHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Account ID wrapper.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccountId(pub Uuid);

impl AccountId {
    /// Generate a new random account ID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    
    /// Parse from string.
    pub fn parse(s: &str) -> Option<Self> {
        Uuid::parse_str(s).ok().map(Self)
    }
}

impl Default for AccountId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for AccountId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Message flags (IMAP style).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageFlags {
    pub seen: bool,
    pub answered: bool,
    pub flagged: bool,
    pub deleted: bool,
    pub draft: bool,
    pub recent: bool,
    pub custom: Vec<String>,
}

impl MessageFlags {
    /// Check if any system flag is set.
    pub fn any_set(&self) -> bool {
        self.seen || self.answered || self.flagged || self.deleted || self.draft || self.recent
    }
    
    /// Convert to IMAP flag string list.
    pub fn to_imap_strings(&self) -> Vec<&'static str> {
        let mut flags = Vec::new();
        if self.seen { flags.push("\\Seen"); }
        if self.answered { flags.push("\\Answered"); }
        if self.flagged { flags.push("\\Flagged"); }
        if self.deleted { flags.push("\\Deleted"); }
        if self.draft { flags.push("\\Draft"); }
        if self.recent { flags.push("\\Recent"); }
        flags
    }
}

/// Delivery status notification type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DsnStatus {
    Success,
    Failure,
    Delay,
}

/// Queue item status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueueStatus {
    Pending,
    Sending,
    Sent,
    Failed,
    Bounced,
    Cancelled,
}

/// Spam scan result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpamResult {
    pub score: f32,
    pub is_spam: bool,
    pub action: SpamAction,
    pub symbols: Vec<SpamSymbol>,
    pub dkim_result: Option<String>,
    pub spf_result: Option<String>,
    pub dmarc_result: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpamAction {
    NoAction,
    Greylist,
    AddHeader,
    RewriteSubject,
    SoftReject,
    Reject,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpamSymbol {
    pub name: String,
    pub score: f32,
    pub description: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_email_address_parse() {
        let addr = EmailAddress::parse("test@example.com").unwrap();
        assert_eq!(addr.address, "test@example.com");
        assert!(addr.name.is_none());
        
        let addr = EmailAddress::parse("John Doe <john@example.com>").unwrap();
        assert_eq!(addr.address, "john@example.com");
        assert_eq!(addr.name, Some("John Doe".to_string()));
        
        let addr = EmailAddress::parse("\"Jane Doe\" <jane@example.com>").unwrap();
        assert_eq!(addr.address, "jane@example.com");
        assert_eq!(addr.name, Some("Jane Doe".to_string()));
    }
    
    #[test]
    fn test_blob_hash() {
        let data = b"Hello, World!";
        let hash = BlobHash::compute(data);
        assert!(!hash.0.is_empty());
        
        // Same data should produce same hash
        let hash2 = BlobHash::compute(data);
        assert_eq!(hash, hash2);
    }
}
