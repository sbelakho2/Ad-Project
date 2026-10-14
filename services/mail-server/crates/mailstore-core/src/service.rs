//! gRPC service implementation for Mailstore.

use crate::db::{Database, MessageFlagsRecord};
use crate::storage::BlobStorage;
use mail_common::Config;
use mail_proto::generated::*;
use tonic::{Request, Response, Status};
use uuid::Uuid;

/// Mailstore gRPC service implementation.
pub struct MailstoreServiceImpl {
    db: Database,
    storage: BlobStorage,
    config: Config,
}

impl MailstoreServiceImpl {
    pub fn new(db: Database, storage: BlobStorage, config: Config) -> Self {
        Self {
            db,
            storage,
            config,
        }
    }
}

#[tonic::async_trait]
impl mailstore_service_server::MailstoreService for MailstoreServiceImpl {
    async fn store_message(
        &self,
        request: Request<StoreMessageRequest>,
    ) -> Result<Response<StoreMessageResponse>, Status> {
        let req = request.into_inner();

        let account_id = Uuid::parse_str(&req.account_id)
            .map_err(|_| Status::invalid_argument("Invalid account_id"))?;

        // Get mailbox
        let mailbox = self
            .db
            .get_mailbox(account_id, &req.mailbox)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found("Mailbox not found"))?;

        // Store blob
        let blob_hash = self
            .storage
            .store(&req.raw_message)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        // Parse envelope (simplified - just extract basic headers)
        let envelope_json = serde_json::json!({
            "size": req.raw_message.len(),
            "received": chrono::Utc::now().to_rfc3339(),
        })
        .to_string();

        let flags = MessageFlagsRecord {
            seen: req.flags.as_ref().is_some_and(|f| f.seen),
            answered: req.flags.as_ref().is_some_and(|f| f.answered),
            flagged: req.flags.as_ref().is_some_and(|f| f.flagged),
            deleted: req.flags.as_ref().is_some_and(|f| f.deleted),
            draft: req.flags.as_ref().is_some_and(|f| f.draft),
        };

        // Store message
        let uid = self
            .db
            .store_message(
                account_id,
                mailbox.id,
                &blob_hash,
                req.raw_message.len() as i64,
                &envelope_json,
                &flags,
            )
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(StoreMessageResponse {
            uid: uid as u64,
            blob_hash,
        }))
    }

    async fn get_message(
        &self,
        request: Request<GetMessageRequest>,
    ) -> Result<Response<GetMessageResponse>, Status> {
        let req = request.into_inner();

        let account_id = Uuid::parse_str(&req.account_id)
            .map_err(|_| Status::invalid_argument("Invalid account_id"))?;

        // Get mailbox
        let mailbox = self
            .db
            .get_mailbox(account_id, &req.mailbox)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found("Mailbox not found"))?;

        // Get message
        let msg = self
            .db
            .get_message(mailbox.id, req.uid as i64)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found("Message not found"))?;

        // Get blob if requested
        let raw_message = if req.include_body {
            self.storage
                .get(&msg.blob_hash)
                .await
                .map_err(|e| Status::internal(e.to_string()))?
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let metadata = MessageMetadata {
            uid: msg.uid as u64,
            blob_hash: msg.blob_hash,
            size: msg.size as u64,
            internal_date: msg.internal_date.timestamp(),
            envelope: Some(Envelope {
                message_id: String::new(),
                from: String::new(),
                to: Vec::new(),
                cc: Vec::new(),
                bcc: Vec::new(),
                subject: String::new(),
                date: msg.internal_date.timestamp(),
                in_reply_to: String::new(),
                references: Vec::new(),
            }),
            flags: Some(MessageFlags {
                seen: msg.seen,
                answered: msg.answered,
                flagged: msg.flagged,
                deleted: msg.deleted,
                draft: msg.draft,
                recent: false,
                custom: Vec::new(),
            }),
        };

        Ok(Response::new(GetMessageResponse {
            metadata: Some(metadata),
            raw_message,
        }))
    }

    async fn delete_message(
        &self,
        request: Request<DeleteMessageRequest>,
    ) -> Result<Response<DeleteMessageResponse>, Status> {
        // Mark messages as deleted (actual deletion happens on expunge)
        let req = request.into_inner();

        let account_id = Uuid::parse_str(&req.account_id)
            .map_err(|_| Status::invalid_argument("Invalid account_id"))?;

        let mailbox = self
            .db
            .get_mailbox(account_id, &req.mailbox)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found("Mailbox not found"))?;

        let uids: Vec<i64> = req.uids.iter().map(|&u| u as i64).collect();

        let flags = MessageFlagsRecord {
            deleted: true,
            ..Default::default()
        };

        let count = self
            .db
            .set_flags(mailbox.id, &uids, &flags)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(DeleteMessageResponse {
            deleted_count: count as u32,
        }))
    }

    async fn list_mailboxes(
        &self,
        request: Request<ListMailboxesRequest>,
    ) -> Result<Response<ListMailboxesResponse>, Status> {
        let req = request.into_inner();

        let account_id = Uuid::parse_str(&req.account_id)
            .map_err(|_| Status::invalid_argument("Invalid account_id"))?;

        let mailboxes = self
            .db
            .list_mailboxes(account_id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let mailboxes = mailboxes
            .into_iter()
            .map(|m| Mailbox {
                name: m.name,
                delimiter: m.delimiter,
                attributes: m.special_use.map(|s| vec![s]).unwrap_or_default(),
                uidvalidity: m.uidvalidity as u64,
                uidnext: m.uidnext as u64,
                exists: m.message_count as u32,
                recent: 0,
                unseen: m.unseen_count as u32,
            })
            .collect();

        Ok(Response::new(ListMailboxesResponse { mailboxes }))
    }

    async fn create_mailbox(
        &self,
        request: Request<CreateMailboxRequest>,
    ) -> Result<Response<CreateMailboxResponse>, Status> {
        let req = request.into_inner();

        let account_id = Uuid::parse_str(&req.account_id)
            .map_err(|_| Status::invalid_argument("Invalid account_id"))?;

        let special_use = if req.special_use.is_empty() {
            None
        } else {
            Some(req.special_use.as_str())
        };

        let mailbox = self
            .db
            .create_mailbox(account_id, &req.name, special_use)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(CreateMailboxResponse {
            mailbox: Some(Mailbox {
                name: mailbox.name,
                delimiter: mailbox.delimiter,
                attributes: mailbox.special_use.map(|s| vec![s]).unwrap_or_default(),
                uidvalidity: mailbox.uidvalidity as u64,
                uidnext: mailbox.uidnext as u64,
                exists: 0,
                recent: 0,
                unseen: 0,
            }),
        }))
    }

    async fn delete_mailbox(
        &self,
        request: Request<DeleteMailboxRequest>,
    ) -> Result<Response<DeleteMailboxResponse>, Status> {
        let req = request.into_inner();

        let account_id = Uuid::parse_str(&req.account_id)
            .map_err(|_| Status::invalid_argument("Invalid account_id"))?;

        let success = self
            .db
            .delete_mailbox(account_id, &req.name)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(DeleteMailboxResponse { success }))
    }

    async fn rename_mailbox(
        &self,
        _request: Request<RenameMailboxRequest>,
    ) -> Result<Response<RenameMailboxResponse>, Status> {
        Err(Status::unimplemented("rename_mailbox not implemented"))
    }

    async fn list_messages(
        &self,
        request: Request<ListMessagesRequest>,
    ) -> Result<Response<ListMessagesResponse>, Status> {
        let req = request.into_inner();

        let account_id = Uuid::parse_str(&req.account_id)
            .map_err(|_| Status::invalid_argument("Invalid account_id"))?;

        let mailbox = self
            .db
            .get_mailbox(account_id, &req.mailbox)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found("Mailbox not found"))?;

        let uid_max = if req.uid_max == 0 {
            i64::MAX
        } else {
            req.uid_max as i64
        };
        let limit = if req.limit == 0 {
            1000
        } else {
            req.limit as i32
        };

        let messages = self
            .db
            .list_messages(mailbox.id, req.uid_min as i64, uid_max, limit)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let messages = messages
            .into_iter()
            .map(|m| MessageMetadata {
                uid: m.uid as u64,
                blob_hash: m.blob_hash,
                size: m.size as u64,
                internal_date: m.internal_date.timestamp(),
                envelope: None,
                flags: Some(MessageFlags {
                    seen: m.seen,
                    answered: m.answered,
                    flagged: m.flagged,
                    deleted: m.deleted,
                    draft: m.draft,
                    recent: false,
                    custom: Vec::new(),
                }),
            })
            .collect();

        Ok(Response::new(ListMessagesResponse { messages }))
    }

    async fn search_messages(
        &self,
        _request: Request<SearchMessagesRequest>,
    ) -> Result<Response<SearchMessagesResponse>, Status> {
        Err(Status::unimplemented("search_messages not implemented"))
    }

    async fn set_flags(
        &self,
        request: Request<SetFlagsRequest>,
    ) -> Result<Response<SetFlagsResponse>, Status> {
        let req = request.into_inner();

        let account_id = Uuid::parse_str(&req.account_id)
            .map_err(|_| Status::invalid_argument("Invalid account_id"))?;

        let mailbox = self
            .db
            .get_mailbox(account_id, &req.mailbox)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found("Mailbox not found"))?;

        let flags = req
            .flags
            .as_ref()
            .map(|f| MessageFlagsRecord {
                seen: f.seen,
                answered: f.answered,
                flagged: f.flagged,
                deleted: f.deleted,
                draft: f.draft,
            })
            .unwrap_or_default();

        let uids: Vec<i64> = req.uids.iter().map(|&u| u as i64).collect();

        let count = self
            .db
            .set_flags(mailbox.id, &uids, &flags)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(SetFlagsResponse {
            updated_count: count as u32,
        }))
    }

    async fn get_flags(
        &self,
        _request: Request<GetFlagsRequest>,
    ) -> Result<Response<GetFlagsResponse>, Status> {
        Err(Status::unimplemented("get_flags not implemented"))
    }

    async fn move_message(
        &self,
        _request: Request<MoveMessageRequest>,
    ) -> Result<Response<MoveMessageResponse>, Status> {
        Err(Status::unimplemented("move_message not implemented"))
    }

    async fn copy_message(
        &self,
        _request: Request<CopyMessageRequest>,
    ) -> Result<Response<CopyMessageResponse>, Status> {
        Err(Status::unimplemented("copy_message not implemented"))
    }

    async fn get_quota(
        &self,
        request: Request<GetQuotaRequest>,
    ) -> Result<Response<GetQuotaResponse>, Status> {
        let req = request.into_inner();

        let account_id = Uuid::parse_str(&req.account_id)
            .map_err(|_| Status::invalid_argument("Invalid account_id"))?;

        let account = self
            .db
            .get_account(account_id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found("Account not found"))?;

        Ok(Response::new(GetQuotaResponse {
            quota: Some(Quota {
                used_bytes: account.used_bytes as u64,
                max_bytes: account.quota_bytes as u64,
                used_messages: 0,
                max_messages: 0,
            }),
        }))
    }

    async fn create_account(
        &self,
        request: Request<CreateAccountRequest>,
    ) -> Result<Response<CreateAccountResponse>, Status> {
        let req = request.into_inner();

        // Hash password (simplified - use proper bcrypt/argon2 in production)
        let password_hash = format!("sha256:{}", sha2_hash(&req.password));

        let quota = req
            .initial_quota
            .as_ref()
            .map(|q| q.max_bytes as i64)
            .unwrap_or(self.config.limits.default_quota_bytes as i64);

        let display_name = if req.display_name.is_empty() {
            None
        } else {
            Some(req.display_name.as_str())
        };

        let id = self
            .db
            .create_account(&req.email, &password_hash, display_name, quota)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        // Create default mailboxes
        let _ = self.db.create_mailbox(id, "INBOX", None).await;
        let _ = self.db.create_mailbox(id, "Sent", Some("\\Sent")).await;
        let _ = self.db.create_mailbox(id, "Drafts", Some("\\Drafts")).await;
        let _ = self.db.create_mailbox(id, "Trash", Some("\\Trash")).await;
        let _ = self.db.create_mailbox(id, "Junk", Some("\\Junk")).await;

        Ok(Response::new(CreateAccountResponse {
            account_id: id.to_string(),
        }))
    }

    async fn get_account(
        &self,
        request: Request<GetAccountRequest>,
    ) -> Result<Response<GetAccountResponse>, Status> {
        let req = request.into_inner();

        let account = if !req.account_id.is_empty() {
            let id = Uuid::parse_str(&req.account_id)
                .map_err(|_| Status::invalid_argument("Invalid account_id"))?;
            self.db.get_account(id).await
        } else {
            self.db.get_account_by_email(&req.email).await
        }
        .map_err(|e| Status::internal(e.to_string()))?
        .ok_or_else(|| Status::not_found("Account not found"))?;

        Ok(Response::new(GetAccountResponse {
            account_id: account.id.to_string(),
            email: account.email,
            display_name: account.display_name.unwrap_or_default(),
            created_at: account.created_at.timestamp(),
            active: account.active,
        }))
    }

    async fn authenticate_account(
        &self,
        request: Request<AuthenticateRequest>,
    ) -> Result<Response<AuthenticateResponse>, Status> {
        let req = request.into_inner();

        let account = self
            .db
            .get_account_by_email(&req.email)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        match account {
            Some(account) => {
                let expected_hash = format!("sha256:{}", sha2_hash(&req.password));
                if account.password_hash == expected_hash && account.active {
                    Ok(Response::new(AuthenticateResponse {
                        success: true,
                        account_id: account.id.to_string(),
                        error: String::new(),
                    }))
                } else {
                    Ok(Response::new(AuthenticateResponse {
                        success: false,
                        account_id: String::new(),
                        error: "Invalid credentials".to_string(),
                    }))
                }
            }
            None => Ok(Response::new(AuthenticateResponse {
                success: false,
                account_id: String::new(),
                error: "Account not found".to_string(),
            })),
        }
    }

    async fn get_mailbox_status(
        &self,
        request: Request<GetMailboxStatusRequest>,
    ) -> Result<Response<GetMailboxStatusResponse>, Status> {
        let req = request.into_inner();

        let account_id = Uuid::parse_str(&req.account_id)
            .map_err(|_| Status::invalid_argument("Invalid account_id"))?;

        let mailbox = self
            .db
            .get_mailbox(account_id, &req.mailbox)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found("Mailbox not found"))?;

        Ok(Response::new(GetMailboxStatusResponse {
            mailbox: Some(Mailbox {
                name: mailbox.name,
                delimiter: mailbox.delimiter,
                attributes: mailbox.special_use.map(|s| vec![s]).unwrap_or_default(),
                uidvalidity: mailbox.uidvalidity as u64,
                uidnext: mailbox.uidnext as u64,
                exists: mailbox.message_count as u32,
                recent: 0,
                unseen: mailbox.unseen_count as u32,
            }),
            highest_modseq: 0,
        }))
    }

    async fn expunge(
        &self,
        request: Request<ExpungeRequest>,
    ) -> Result<Response<ExpungeResponse>, Status> {
        let req = request.into_inner();

        let account_id = Uuid::parse_str(&req.account_id)
            .map_err(|_| Status::invalid_argument("Invalid account_id"))?;

        let mailbox = self
            .db
            .get_mailbox(account_id, &req.mailbox)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found("Mailbox not found"))?;

        let uids = self
            .db
            .expunge(mailbox.id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(ExpungeResponse {
            expunged_uids: uids.into_iter().map(|u| u as u64).collect(),
        }))
    }

    type SubscribeMailboxStream =
        tokio_stream::wrappers::ReceiverStream<Result<MailboxEvent, Status>>;

    async fn subscribe_mailbox(
        &self,
        _request: Request<SubscribeMailboxRequest>,
    ) -> Result<Response<Self::SubscribeMailboxStream>, Status> {
        Err(Status::unimplemented("subscribe_mailbox not implemented"))
    }
}

fn sha2_hash(data: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(data.as_bytes());
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, hash)
}
