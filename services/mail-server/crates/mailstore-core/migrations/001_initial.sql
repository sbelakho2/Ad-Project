-- Mailstore Core Database Migrations
-- Initial schema for mail storage

-- Accounts table
CREATE TABLE IF NOT EXISTS mail_accounts (
    id UUID PRIMARY KEY,
    email VARCHAR(255) NOT NULL UNIQUE,
    password_hash VARCHAR(255) NOT NULL,
    display_name VARCHAR(255),
    quota_bytes BIGINT NOT NULL DEFAULT 5368709120, -- 5GB
    used_bytes BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    active BOOLEAN NOT NULL DEFAULT true
);

CREATE INDEX IF NOT EXISTS idx_mail_accounts_email ON mail_accounts(email);

-- Mailboxes table
CREATE TABLE IF NOT EXISTS mail_mailboxes (
    id UUID PRIMARY KEY,
    account_id UUID NOT NULL REFERENCES mail_accounts(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    delimiter CHAR(1) NOT NULL DEFAULT '/',
    special_use VARCHAR(50),  -- \Sent, \Drafts, \Trash, \Junk
    uidvalidity BIGINT NOT NULL,
    uidnext BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(account_id, name)
);

CREATE INDEX IF NOT EXISTS idx_mail_mailboxes_account ON mail_mailboxes(account_id);

-- Messages table
CREATE TABLE IF NOT EXISTS mail_messages (
    id UUID PRIMARY KEY,
    account_id UUID NOT NULL REFERENCES mail_accounts(id) ON DELETE CASCADE,
    mailbox_id UUID NOT NULL REFERENCES mail_mailboxes(id) ON DELETE CASCADE,
    uid BIGINT NOT NULL,
    blob_hash VARCHAR(64) NOT NULL,
    size BIGINT NOT NULL,
    envelope JSONB,
    seen BOOLEAN NOT NULL DEFAULT false,
    answered BOOLEAN NOT NULL DEFAULT false,
    flagged BOOLEAN NOT NULL DEFAULT false,
    deleted BOOLEAN NOT NULL DEFAULT false,
    draft BOOLEAN NOT NULL DEFAULT false,
    internal_date TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(mailbox_id, uid)
);

CREATE INDEX IF NOT EXISTS idx_mail_messages_mailbox ON mail_messages(mailbox_id);
CREATE INDEX IF NOT EXISTS idx_mail_messages_blob ON mail_messages(blob_hash);
CREATE INDEX IF NOT EXISTS idx_mail_messages_date ON mail_messages(internal_date);
CREATE INDEX IF NOT EXISTS idx_mail_messages_flags ON mail_messages(mailbox_id, seen, deleted);

-- Outbound queue table
CREATE TABLE IF NOT EXISTS mail_queue (
    id UUID PRIMARY KEY,
    account_id UUID REFERENCES mail_accounts(id),
    sender VARCHAR(255) NOT NULL,
    recipients JSONB NOT NULL,
    blob_hash VARCHAR(64) NOT NULL,
    priority INT NOT NULL DEFAULT 2,
    status VARCHAR(20) NOT NULL DEFAULT 'pending',
    retry_count INT NOT NULL DEFAULT 0,
    next_retry TIMESTAMPTZ,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_mail_queue_status ON mail_queue(status, next_retry);
CREATE INDEX IF NOT EXISTS idx_mail_queue_account ON mail_queue(account_id);

-- Rate limiting table
CREATE TABLE IF NOT EXISTS mail_rate_limits (
    account_id UUID NOT NULL REFERENCES mail_accounts(id) ON DELETE CASCADE,
    window_start TIMESTAMPTZ NOT NULL,
    message_count INT NOT NULL DEFAULT 0,
    recipient_count INT NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, window_start)
);

-- DKIM keys table
CREATE TABLE IF NOT EXISTS mail_dkim_keys (
    id UUID PRIMARY KEY,
    domain VARCHAR(255) NOT NULL,
    selector VARCHAR(255) NOT NULL,
    private_key TEXT NOT NULL,
    public_key TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    active BOOLEAN NOT NULL DEFAULT true,
    UNIQUE(domain, selector)
);

-- Spam learning table (track what we've trained on)
CREATE TABLE IF NOT EXISTS mail_spam_learned (
    blob_hash VARCHAR(64) PRIMARY KEY,
    learned_as VARCHAR(10) NOT NULL,  -- 'spam' or 'ham'
    account_id UUID REFERENCES mail_accounts(id),
    learned_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
