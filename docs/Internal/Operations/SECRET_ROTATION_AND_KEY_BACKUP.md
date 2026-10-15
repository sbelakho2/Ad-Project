# Secret rotation and key backup

Two things keep the product safe: the secrets never leak, and the signing keys
survive a disk loss. This note covers both, plus the cleanup after a leak.

## Rotate every credential that was ever committed

The deployment host, the database, the mail relay and the test password in the
old docs are treated as public. Rotate in this order.

1. Move the repository to a private one, then rotate the platform keys.
2. Rotate the max management key and the report key, the levelplay secret key
   and refresh token, and the admob oauth client.
3. Rotate the adyen api key, merchant account and hmac key, then re-check the
   webhook signature in the sandbox.
4. Rotate the host ssh keys and the mail relay password, and disable the old
   accounts.
5. Rotate the vault key and re-encrypt the account blobs under the new key.
6. Rotate the ledger signing key only with a documented cut-over, because it is
   the anchor a publisher verifies against.

## Scan the history before the repository goes private

Run a history scan, not just a working-tree scan.

```bash
git log -p --all | grep -iE 'password|secret|api[_-]?key|token' | head
```
Then run the same scan with a dedicated tool such as gitleaks over the full
history. Deleting a file does not remove it from history, so a leaked value
must be rotated even after the file is gone. Treat any hit as leaked.

## Back up the keys

Two key classes need an offline backup.

- The vault key encrypts the per-publisher account blobs. Without it the blobs
  are unreadable.
- The ledger signing key (an ed25519 seed) is the anchor of every proof. A
  published ledger is verifiable without it, but no new record can be signed
  without it.

Backup rules:

1. Keep each key in a password manager entry and on an offline medium, in two
   separate physical locations.
2. Store the ledger key as a 64-character hex seed and record the public key
   next to it, so a restore can be checked.
3. Rehearse a restore quarterly into an isolated directory: open a ledger with
   the backed-up seed and verify it.
4. Never store a backup next to the encrypted data it protects.

## What a restore needs

A restore of one publisher needs three things: the ledger file, the signing
key seed, and the vault key. With those, `Ledger::open` re-verifies the whole
chain and `Vault` decrypts the account. A restore that cannot verify the chain
is a failed restore, not a partial one.
