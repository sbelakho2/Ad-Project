# Two-Factor Authentication (2FA) System

## Overview

The platform implements TOTP-based two-factor authentication using the `otplib` library, with encrypted secret storage and backup code support.

## Architecture

```
┌─────────────────────┐     ┌─────────────────────┐
│   Console (React)   │     │   Backend (Express) │
│                     │     │                     │
│  /settings/security │────▶│  /auth/twofa/*     │
│                     │     │                     │
│  - Status display   │     │  - twofa.service.ts│
│  - QR enrollment    │     │  - AES-GCM encrypt │
│  - Code verify      │     │  - bcrypt backup   │
│  - Backup codes     │     │                     │
└─────────────────────┘     └─────────────────────┘
```

## Backend Components

### Service (`backend/src/services/twofa.service.ts`)

| Method | Purpose |
|--------|---------|
| `isEnabled(userId)` | Check if 2FA is enabled for user |
| `getBackupCodesCount(userId)` | Get remaining backup codes count |
| `enroll(userId, email)` | Generate TOTP secret and QR code |
| `verify(userId, token)` | Verify TOTP token and enable 2FA |
| `disable(userId, password, code)` | Disable 2FA with verification |
| `regenerateBackupCodes(userId)` | Generate new backup codes |
| `validateBackupCode(userId, code)` | Use a backup code (one-time) |

### Routes (`backend/src/routes/auth/twofa.routes.ts`)

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/auth/twofa/status` | GET | Get 2FA status and backup codes count |
| `/auth/twofa/enroll` | POST | Start enrollment, returns QR code |
| `/auth/twofa/verify` | POST | Verify code and enable 2FA |
| `/auth/twofa/disable` | POST | Disable 2FA (requires password + code) |
| `/auth/twofa/backup-codes/regenerate` | POST | Generate new backup codes |

### Controller (`backend/src/controllers/twofa.controller.ts`)

Handles request validation, user context extraction, and audit logging.

## Secret Storage

### Encryption

TOTP secrets are encrypted using AES-256-GCM:

```typescript
// Encryption key from environment
const key = process.env.APP_KMS_KEY  // 32 bytes base64

// Encryption format
{
  iv: "base64...",        // 12 bytes IV
  ciphertext: "base64...", // Encrypted secret
  tag: "base64..."        // 16 bytes auth tag
}
```

### Fallback (Development Only)

In development without `APP_KMS_KEY`, secrets are stored in plaintext with a warning logged.

## Backup Codes

- **Generation**: 10 codes, 8 characters each, alphanumeric
- **Storage**: bcrypt hashed (cost factor 10)
- **Usage**: One-time use, marked as consumed after validation
- **Regeneration**: All existing codes invalidated when regenerated

## Console Integration

### Security Settings Page (`console/src/app/settings/security/page.tsx`)

**States:**
1. **Loading** - Fetching 2FA status
2. **Disabled** - Show "Enable 2FA" button
3. **Enrolling** - Show QR code and verification input
4. **Enabled** - Show status, backup codes remaining, disable option

**API Calls:**
```typescript
// Check status
GET /auth/twofa/status → { enabled: boolean, backupCodesRemaining: number }

// Start enrollment
POST /auth/twofa/enroll → { qrDataUrl, otpauthUrl, maskedSecret }

// Verify and enable
POST /auth/twofa/verify { code: "123456" } → { success, backupCodes[] }

// Disable
POST /auth/twofa/disable { password, code } → { success }
```

## Login Flow with 2FA

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│   Login     │────▶│  Password   │────▶│  2FA Code   │
│   Page      │     │   Verify    │     │   Required  │
└─────────────┘     └─────────────┘     └─────────────┘
                           │                   │
                           ▼                   ▼
                    ┌─────────────┐     ┌─────────────┐
                    │  No 2FA     │     │  Verify     │
                    │  → Session  │     │  → Session  │
                    └─────────────┘     └─────────────┘
```

The login page (`console/src/app/login/page.tsx`) handles the 2FA challenge:
1. User submits credentials
2. If 2FA enabled, backend returns `requires2FA: true`
3. Login page shows 2FA code input
4. User enters code from authenticator app (or backup code)
5. On success, session is created

## Audit Trail

All 2FA operations are logged:

```typescript
logger.info('2FA enroll started', { userId, issuer })
logger.info('2FA enabled', { userId })
logger.info('2FA disabled', { userId, actor })
logger.warn('2FA verification failed', { userId, attempts })
```

## Security Considerations

1. **Rate Limiting**: 5 attempts per minute for verification
2. **Brute Force**: Account lockout after 10 failed attempts
3. **Backup Codes**: bcrypt hashed, single-use
4. **Secret Rotation**: New secret on each enrollment attempt
5. **Audit Trail**: All operations logged for compliance

## Testing

### Run 2FA Tests
```bash
cd backend
npm test -- --testPathPattern="twofa"
```

### Manual Testing
1. Enable 2FA in Settings → Security
2. Scan QR with Google Authenticator or Authy
3. Enter 6-digit code
4. Save backup codes
5. Log out and log back in (should prompt for 2FA)
6. Try backup code login
7. Disable 2FA

## Troubleshooting

### "Invalid 2FA code" errors
- Check device time sync (TOTP is time-based, 30-second windows)
- Ensure using correct authenticator account
- Try a backup code

### "2FA not enabling"
- Check `APP_KMS_KEY` environment variable is set
- Verify database TwoFactorAuth table exists
- Check backend logs for encryption errors

### Lost authenticator access
- Use backup code to log in
- Disable 2FA in settings
- Re-enroll with new device
