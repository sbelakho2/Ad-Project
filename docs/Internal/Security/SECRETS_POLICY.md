# Secrets Management Policy

**Status**: Active  
**Owner**: Security Team  
**Last Updated**: 2024

---

## Overview

This document defines the secrets management policy for the ApexMediation platform. All team members must follow these guidelines to protect sensitive credentials, API keys, and other secrets from unauthorized access.

---

## Policy

### 1. Never Commit Secrets to Version Control

**Prohibited Actions:**
- Hardcoding API keys, passwords, tokens, or certificates in source code
- Committing `.env` files containing production secrets
- Storing secrets in configuration files tracked by Git
- Including secrets in code comments or documentation

**Allowed Practices:**
- Using environment variables injected at runtime
- Storing secrets in GitHub Secrets for CI/CD workflows
- Using dedicated secrets management tools (AWS Secrets Manager, HashiCorp Vault, etc.)
- Providing `.env.example` files with placeholder values

### 2. Use Environment-Specific Secrets

**Development:**
- Use local `.env` files (added to `.gitignore`)
- Never use production secrets in development environments
- Generate mock/test credentials where possible

**Staging:**
- Store secrets in GitHub Secrets with `staging-` prefix
- Use separate AWS Secrets Manager namespaces
- Rotate secrets quarterly

**Production:**
- Store secrets in GitHub Secrets with `prod-` prefix
- Use AWS Secrets Manager with encryption at rest
- Rotate secrets monthly (API keys quarterly minimum)
- Implement secret version control and rollback capability

### 3. Secret Types and Handling

#### API Keys
- **Examples**: OpenAI API keys, payment provider keys, third-party integrations
- **Storage**: Environment variables, GitHub Secrets, AWS Secrets Manager
- **Rotation**: Every 90 days minimum
- **Access**: Restrict to services that require them

#### Database Credentials
- **Examples**: PostgreSQL passwords, ClickHouse credentials, Redis auth tokens
- **Storage**: AWS Secrets Manager with automatic rotation
- **Rotation**: Every 30 days
- **Access**: IAM role-based access only

#### TLS/SSL Certificates
- **Examples**: HTTPS certificates, mTLS client certs
- **Storage**: AWS Certificate Manager or encrypted S3 buckets
- **Rotation**: Automatic via Let's Encrypt or ACM
- **Access**: Infrastructure team only

#### Signing Keys
- **Examples**: JWT signing keys, webhook signature secrets
- **Storage**: AWS KMS or dedicated key management service
- **Rotation**: Every 180 days
- **Access**: Limited to authentication/authorization services

#### OAuth Tokens
- **Examples**: GitHub tokens, Google OAuth client secrets
- **Storage**: GitHub Secrets, encrypted environment variables
- **Rotation**: When team members leave or every 90 days
- **Access**: CI/CD pipelines and authorized services only

---

## Secret Scanning

### GitHub Secret Scanning

**Enabled Features:**
- GitHub Advanced Security secret scanning (enabled for all repositories)
- Push protection to block accidental commits containing secrets
- Partner patterns for detecting AWS, Azure, Google, Mollie, and other provider secrets

**Workflow:**
1. GitHub automatically scans commits for known secret patterns
2. Alerts are sent to repository administrators and security team
3. Secrets must be rotated immediately upon detection
4. Pull requests containing secrets are blocked from merging

### Pre-Commit Hooks

Install `detect-secrets` for local pre-commit scanning:

```bash
# Install pre-commit framework
pip install pre-commit detect-secrets

# Install hooks
pre-commit install

# Scan for secrets in staged files
detect-secrets scan --baseline .secrets.baseline
```

### CI/CD Secret Scanning

All pull requests are scanned with:
- **Trivy**: Detects secrets in Docker images and IaC files
- **CodeQL**: Identifies hardcoded credentials in source code
- **Custom regex patterns**: Project-specific secret formats

**CI Gate**: Pull requests fail if secrets are detected.

---

## Incident Response

### If You Accidentally Commit a Secret

**Immediate Actions (within 1 hour):**
1. **Rotate the secret immediately** in the provider's dashboard
2. Notify the security team via Slack (`#security-alerts`)
3. Open an incident ticket with details (service, secret type, commit SHA)
4. Remove the secret from Git history:
   ```bash
   # For recent commits
   git filter-repo --invert-paths --path <file-with-secret>
   
   # Force push (coordinate with team)
   git push --force-with-lease origin main
   ```

**Follow-Up Actions (within 24 hours):**
1. Audit access logs for unauthorized usage of the compromised secret
2. Update documentation to reflect new credentials
3. Conduct a post-mortem to prevent future incidents
4. Add detection patterns to secret scanning tools if not already covered

### If a Secret is Exposed Publicly

**Critical Response (within 15 minutes):**
1. **Revoke the secret immediately** in all environments
2. Alert the security team and engineering leadership
3. Create a GitHub security advisory if applicable
4. Monitor for unauthorized access attempts
5. Rotate all related secrets as a precaution

**Investigation (within 48 hours):**
1. Determine scope of exposure (duration, access logs, affected systems)
2. Assess impact on customers and data
3. File a security incident report
4. Implement additional controls to prevent recurrence

---

## Access Control

### GitHub Secrets

**Repository Secrets:**
- Accessible to all workflows in the repository
- Use for non-sensitive CI/CD credentials (npm tokens, test API keys)

**Organization Secrets:**
- Shared across multiple repositories
- Require admin approval to access
- Use for shared infrastructure credentials

**Environment Secrets:**
- Tied to deployment environments (staging, production)
- Require manual approval for production deployments
- Use for environment-specific sensitive credentials

**Permissions:**
- Only repository admins can create/modify secrets
- Audit logs are reviewed monthly
- Remove secrets when no longer needed

### AWS Secrets Manager

**Access via IAM Roles:**
- Services use IAM roles to retrieve secrets at runtime
- No hardcoded AWS credentials in applications
- Least-privilege principle enforced

**Secret Policies:**
```json
{
  "Version": "2012-10-17",
  "Statement": [{
    "Effect": "Allow",
    "Principal": {
      "AWS": "arn:aws:iam::ACCOUNT_ID:role/backend-service"
    },
    "Action": "secretsmanager:GetSecretValue",
    "Resource": "arn:aws:secretsmanager:us-east-1:ACCOUNT_ID:secret:prod/database/*"
  }]
}
```

---

## Developer Guidelines

### Setting Up Local Development

1. **Copy the example environment file:**
   ```bash
   cp .env.example .env
   ```

2. **Request development secrets:**
   - Contact the DevOps team for non-production secrets
   - Use mock values for third-party APIs when possible
   - Document required secrets in `.env.example`

3. **Never commit your `.env` file:**
   ```bash
   # Verify .env is in .gitignore
   git check-ignore .env
   ```

### Using Secrets in Code

**Backend (Node.js/TypeScript):**
```typescript
// ✅ Correct: Load from environment variables
const apiKey = process.env.OPENAI_API_KEY;

if (!apiKey) {
  throw new Error('OPENAI_API_KEY is not set');
}

// ❌ Wrong: Hardcoded secret
const apiKey = 'sk-proj-abc123...';
```

**Kubernetes Deployments:**
```yaml
# ✅ Correct: Reference external secret
env:
  - name: DATABASE_PASSWORD
    valueFrom:
      secretKeyRef:
        name: postgres-credentials
        key: password

# ❌ Wrong: Hardcoded in manifest
env:
  - name: DATABASE_PASSWORD
    value: "my-secret-password"
```

**CI/CD Workflows:**
```yaml
# ✅ Correct: Use GitHub Secrets
env:
  DEPLOY_KEY: ${{ secrets.PROD_DEPLOY_KEY }}

# ❌ Wrong: Hardcoded in workflow
env:
  DEPLOY_KEY: ghp_abc123xyz...
```

---

## Compliance

### Audit Requirements

- **Quarterly Secret Rotation Audit**: Verify all production secrets were rotated within policy timeframes
- **Monthly Access Review**: Review IAM policies and GitHub secret access logs
- **Annual Security Training**: All engineers must complete secrets management training

### Reporting

- Security team publishes a monthly secrets hygiene report
- Incidents involving secrets are tracked in the security incident log
- Compliance metrics are reviewed in quarterly security reviews

---

## Tools and Resources

### Recommended Tools

- **GitHub Advanced Security**: Built-in secret scanning and push protection
- **AWS Secrets Manager**: Centralized secrets storage with automatic rotation
- **detect-secrets**: Pre-commit hook for local secret scanning
- **git-secrets**: Prevents committing secrets to Git repositories
- **Trivy**: Container and IaC secret scanning

### Training Resources

- [OWASP Secrets Management Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Secrets_Management_Cheat_Sheet.html)
- [GitHub Secret Scanning Documentation](https://docs.github.com/en/code-security/secret-scanning)
- [AWS Secrets Manager Best Practices](https://docs.aws.amazon.com/secretsmanager/latest/userguide/best-practices.html)

### Contact

- **Security Team**: security@rivaladplatform.com
- **Slack Channel**: `#security`
- **Incident Hotline**: File a P0 incident via PagerDuty

---

## Revision History

| Date       | Version | Changes                              | Author        |
|------------|---------|--------------------------------------|---------------|
| 2024-01-15 | 1.0     | Initial policy creation              | Security Team |
