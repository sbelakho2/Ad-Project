//! Database layer for mailstore.

use anyhow::Result;
use chrono::{DateTime, Utc};
use mail_common::config::DatabaseConfig;
use sqlx::{postgres::PgPoolOptions, PgPool};
use tracing::info;
use uuid::Uuid;

/// Database connection pool.
#[derive(Clone)]
pub struct Database {
    pool: PgPool,
}

impl Database {
    /// Connect to the database.
    pub async fn connect(config: &DatabaseConfig) -> Result<Self> {
        info!("Connecting to database...");

        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .acquire_timeout(std::time::Duration::from_secs(config.connect_timeout_secs))
            .connect(&config.url)
            .await?;

        info!("Database connected");
        Ok(Self { pool })
    }

    /// Run database migrations.
    pub async fn migrate(&self) -> Result<()> {
        info!("Running database migrations...");
        sqlx::migrate!("./migrations").run(&self.pool).await?;
        info!("Migrations complete");
        Ok(())
    }

    // ========================================================================
    // Account Operations
    // ========================================================================

    /// Create a new account.
    pub async fn create_account(
        &self,
        email: &str,
        password_hash: &str,
        display_name: Option<&str>,
        quota_bytes: i64,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();

        sqlx::query(
            r#"
            INSERT INTO mail_accounts (id, email, password_hash, display_name, quota_bytes, created_at)
            VALUES ($1, $2, $3, $4, $5, NOW())
            "#,
        )
        .bind(id)
        .bind(email)
        .bind(password_hash)
        .bind(display_name)
        .bind(quota_bytes)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    /// Get account by email.
    pub async fn get_account_by_email(&self, email: &str) -> Result<Option<AccountRecord>> {
        let row = sqlx::query_as::<_, AccountRecord>(
            r#"
            SELECT id, email, password_hash, display_name, quota_bytes, used_bytes, created_at, active
            FROM mail_accounts
            WHERE email = $1
            "#,
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Get account by ID.
    pub async fn get_account(&self, id: Uuid) -> Result<Option<AccountRecord>> {
        let row = sqlx::query_as::<_, AccountRecord>(
            r#"
            SELECT id, email, password_hash, display_name, quota_bytes, used_bytes, created_at, active
            FROM mail_accounts
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    // ========================================================================
    // Mailbox Operations
    // ========================================================================

    /// Create a mailbox for an account.
    pub async fn create_mailbox(
        &self,
        account_id: Uuid,
        name: &str,
        special_use: Option<&str>,
    ) -> Result<MailboxRecord> {
        let id = Uuid::new_v4();
        let uidvalidity = rand::random::<u32>() as i64;

        let row = sqlx::query_as::<_, MailboxRecord>(
            r#"
            INSERT INTO mail_mailboxes (id, account_id, name, delimiter, special_use, uidvalidity, uidnext, created_at)
            VALUES ($1, $2, $3, '/', $4, $5, 1, NOW())
            RETURNING id, account_id, name, delimiter, special_use, uidvalidity, uidnext, 
                      0::bigint as message_count, 0::bigint as unseen_count, created_at
            "#,
        )
        .bind(id)
        .bind(account_id)
        .bind(name)
        .bind(special_use)
        .bind(uidvalidity)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// List mailboxes for an account.
    pub async fn list_mailboxes(&self, account_id: Uuid) -> Result<Vec<MailboxRecord>> {
        let rows = sqlx::query_as::<_, MailboxRecord>(
            r#"
            SELECT m.id, m.account_id, m.name, m.delimiter, m.special_use, m.uidvalidity, m.uidnext,
                   COALESCE(msg.cnt, 0) as message_count,
                   COALESCE(msg.unseen, 0) as unseen_count,
                   m.created_at
            FROM mail_mailboxes m
            LEFT JOIN (
                SELECT mailbox_id, COUNT(*) as cnt, COUNT(*) FILTER (WHERE NOT seen) as unseen
                FROM mail_messages
                WHERE NOT deleted
                GROUP BY mailbox_id
            ) msg ON msg.mailbox_id = m.id
            WHERE m.account_id = $1
            ORDER BY m.name
            "#,
        )
        .bind(account_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Get mailbox by name.
    pub async fn get_mailbox(&self, account_id: Uuid, name: &str) -> Result<Option<MailboxRecord>> {
        let row = sqlx::query_as::<_, MailboxRecord>(
            r#"
            SELECT m.id, m.account_id, m.name, m.delimiter, m.special_use, m.uidvalidity, m.uidnext,
                   COALESCE(msg.cnt, 0) as message_count,
                   COALESCE(msg.unseen, 0) as unseen_count,
                   m.created_at
            FROM mail_mailboxes m
            LEFT JOIN (
                SELECT mailbox_id, COUNT(*) as cnt, COUNT(*) FILTER (WHERE NOT seen) as unseen
                FROM mail_messages
                WHERE NOT deleted
                GROUP BY mailbox_id
            ) msg ON msg.mailbox_id = m.id
            WHERE m.account_id = $1 AND m.name = $2
            "#,
        )
        .bind(account_id)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Delete a mailbox.
    pub async fn delete_mailbox(&self, account_id: Uuid, name: &str) -> Result<bool> {
        let result = sqlx::query(
            r#"
            DELETE FROM mail_mailboxes
            WHERE account_id = $1 AND name = $2
            "#,
        )
        .bind(account_id)
        .bind(name)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    // ========================================================================
    // Message Operations
    // ========================================================================

    /// Store a new message.
    pub async fn store_message(
        &self,
        account_id: Uuid,
        mailbox_id: Uuid,
        blob_hash: &str,
        size: i64,
        envelope_json: &str,
        flags: &MessageFlagsRecord,
    ) -> Result<i64> {
        let mut tx = self.pool.begin().await?;

        // Get and increment UID
        let uid: i64 = sqlx::query_scalar(
            r#"
            UPDATE mail_mailboxes
            SET uidnext = uidnext + 1
            WHERE id = $1
            RETURNING uidnext - 1
            "#,
        )
        .bind(mailbox_id)
        .fetch_one(&mut *tx)
        .await?;

        // Insert message
        sqlx::query(
            r#"
            INSERT INTO mail_messages (
                id, account_id, mailbox_id, uid, blob_hash, size, envelope,
                seen, answered, flagged, deleted, draft, internal_date, created_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb, $8, $9, $10, $11, $12, NOW(), NOW())
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(account_id)
        .bind(mailbox_id)
        .bind(uid)
        .bind(blob_hash)
        .bind(size)
        .bind(envelope_json)
        .bind(flags.seen)
        .bind(flags.answered)
        .bind(flags.flagged)
        .bind(flags.deleted)
        .bind(flags.draft)
        .execute(&mut *tx)
        .await?;

        // Update account used bytes
        sqlx::query(
            r#"
            UPDATE mail_accounts
            SET used_bytes = used_bytes + $1
            WHERE id = $2
            "#,
        )
        .bind(size)
        .bind(account_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(uid)
    }

    /// Get message metadata.
    pub async fn get_message(&self, mailbox_id: Uuid, uid: i64) -> Result<Option<MessageRecord>> {
        let row = sqlx::query_as::<_, MessageRecord>(
            r#"
            SELECT id, account_id, mailbox_id, uid, blob_hash, size, envelope,
                   seen, answered, flagged, deleted, draft, internal_date
            FROM mail_messages
            WHERE mailbox_id = $1 AND uid = $2 AND NOT deleted
            "#,
        )
        .bind(mailbox_id)
        .bind(uid)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// List messages in a mailbox.
    pub async fn list_messages(
        &self,
        mailbox_id: Uuid,
        uid_min: i64,
        uid_max: i64,
        limit: i32,
    ) -> Result<Vec<MessageRecord>> {
        let rows = sqlx::query_as::<_, MessageRecord>(
            r#"
            SELECT id, account_id, mailbox_id, uid, blob_hash, size, envelope,
                   seen, answered, flagged, deleted, draft, internal_date
            FROM mail_messages
            WHERE mailbox_id = $1 AND uid >= $2 AND uid <= $3 AND NOT deleted
            ORDER BY uid
            LIMIT $4
            "#,
        )
        .bind(mailbox_id)
        .bind(uid_min)
        .bind(uid_max)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Set message flags.
    pub async fn set_flags(
        &self,
        mailbox_id: Uuid,
        uids: &[i64],
        flags: &MessageFlagsRecord,
    ) -> Result<u64> {
        let result = sqlx::query(
            r#"
            UPDATE mail_messages
            SET seen = $3, answered = $4, flagged = $5, deleted = $6, draft = $7
            WHERE mailbox_id = $1 AND uid = ANY($2)
            "#,
        )
        .bind(mailbox_id)
        .bind(uids)
        .bind(flags.seen)
        .bind(flags.answered)
        .bind(flags.flagged)
        .bind(flags.deleted)
        .bind(flags.draft)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    /// Expunge deleted messages.
    pub async fn expunge(&self, mailbox_id: Uuid) -> Result<Vec<i64>> {
        let rows: Vec<(i64,)> = sqlx::query_as(
            r#"
            DELETE FROM mail_messages
            WHERE mailbox_id = $1 AND deleted
            RETURNING uid
            "#,
        )
        .bind(mailbox_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|(uid,)| uid).collect())
    }
}

// Record types
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AccountRecord {
    pub id: Uuid,
    pub email: String,
    pub password_hash: String,
    pub display_name: Option<String>,
    pub quota_bytes: i64,
    pub used_bytes: i64,
    pub created_at: DateTime<Utc>,
    pub active: bool,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MailboxRecord {
    pub id: Uuid,
    pub account_id: Uuid,
    pub name: String,
    pub delimiter: String,
    pub special_use: Option<String>,
    pub uidvalidity: i64,
    pub uidnext: i64,
    pub message_count: i64,
    pub unseen_count: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MessageRecord {
    pub id: Uuid,
    pub account_id: Uuid,
    pub mailbox_id: Uuid,
    pub uid: i64,
    pub blob_hash: String,
    pub size: i64,
    pub envelope: sqlx::types::Json<serde_json::Value>,
    pub seen: bool,
    pub answered: bool,
    pub flagged: bool,
    pub deleted: bool,
    pub draft: bool,
    pub internal_date: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct MessageFlagsRecord {
    pub seen: bool,
    pub answered: bool,
    pub flagged: bool,
    pub deleted: bool,
    pub draft: bool,
}
