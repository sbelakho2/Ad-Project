//! Resend API Client
//!
//! Integration with Resend for outbound email delivery.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

const RESEND_API_URL: &str = "https://api.resend.com/emails";

/// Resend API client
pub struct ResendClient {
    api_key: String,
    client: reqwest::Client,
    default_from: String,
}

#[derive(Debug, Serialize)]
pub struct SendEmailRequest {
    pub from: String,
    pub to: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cc: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bcc: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    pub subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
pub struct Attachment {
    pub filename: String,
    pub content: String, // Base64 encoded
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SendEmailResponse {
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct ResendError {
    pub message: String,
    #[serde(default)]
    pub name: String,
}

impl ResendClient {
    /// Create a new Resend client
    pub fn new(api_key: String, default_from: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            api_key,
            client,
            default_from,
        }
    }

    /// Send an email via Resend API
    pub async fn send(&self, mut request: SendEmailRequest) -> Result<String> {
        // Use default from if not specified
        if request.from.is_empty() {
            request.from = self.default_from.clone();
        }

        debug!(
            to = ?request.to,
            subject = %request.subject,
            "Sending email via Resend"
        );

        let response = self
            .client
            .post(RESEND_API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;

        if status.is_success() {
            let resp: SendEmailResponse = serde_json::from_str(&body)?;
            info!(email_id = %resp.id, to = ?request.to, "Email sent successfully");
            Ok(resp.id)
        } else {
            let error: ResendError = serde_json::from_str(&body).unwrap_or_else(|_| ResendError {
                message: body.clone(),
                name: "UnknownError".to_string(),
            });

            warn!(
                status = %status,
                error = %error.message,
                "Resend API error"
            );

            Err(anyhow!(
                "Resend API error: {} - {}",
                error.name,
                error.message
            ))
        }
    }
}

/// One request builder for queue items, so the operator send path and any
/// future queue drain cannot drift apart.
impl From<&crate::queue::QueueItem> for SendEmailRequest {
    fn from(item: &crate::queue::QueueItem) -> Self {
        Self {
            from: item.from.clone(),
            to: item.to.clone(),
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: None,
            subject: item.subject.clone(),
            text: item.body_text.clone(),
            html: item.body_html.clone(),
            attachments: Vec::new(),
            headers: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_serialization() {
        let req = SendEmailRequest {
            from: "test@example.com".to_string(),
            to: vec!["dest@example.com".to_string()],
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: None,
            subject: "Test".to_string(),
            text: Some("Hello".to_string()),
            html: None,
            attachments: Vec::new(),
            headers: None,
        };

        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("test@example.com"));
        assert!(!json.contains("cc")); // Empty arrays should be skipped
    }
}

#[cfg(test)]
mod queue_conversion_tests {
    use super::*;
    use crate::queue::QueueItem;

    #[test]
    fn queue_item_maps_to_request_without_dropping_content() {
        let item = QueueItem::new(
            "from@example.com".to_string(),
            vec!["to@example.com".to_string()],
            "Subject".to_string(),
        )
        .with_text("body text");
        let request = SendEmailRequest::from(&item);
        assert_eq!(request.from, "from@example.com");
        assert_eq!(request.to, vec!["to@example.com".to_string()]);
        assert_eq!(request.subject, "Subject");
        assert_eq!(request.text.as_deref(), Some("body text"));
        assert!(request.html.is_none());
        let json = serde_json::to_string(&request).unwrap();
        assert!(!json.contains("\"cc\""), "empty cc must be skipped");
    }
}
