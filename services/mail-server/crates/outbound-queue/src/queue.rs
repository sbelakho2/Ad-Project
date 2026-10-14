//! Queue management for outbound emails

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueItem {
    pub id: Uuid,
    pub from: String,
    pub to: Vec<String>,
    pub subject: String,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub status: QueueStatus,
    pub retry_count: u32,
    pub next_retry: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueueStatus {
    Pending,
    Sending,
    Sent,
    Failed,
    Bounced,
}

impl QueueItem {
    pub fn new(from: String, to: Vec<String>, subject: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            from,
            to,
            subject,
            body_text: None,
            body_html: None,
            status: QueueStatus::Pending,
            retry_count: 0,
            next_retry: None,
            last_error: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn with_text(mut self, text: &str) -> Self {
        self.body_text = Some(text.to_string());
        self
    }

    pub fn with_html(mut self, html: &str) -> Self {
        self.body_html = Some(html.to_string());
        self
    }
}
