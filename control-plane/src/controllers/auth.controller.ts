import { Request, Response } from 'express';
import { 
  verifyOperatorCredentials, 
  issueOperatorToken, 
  recordLoginAttempt, 
  clearLoginAttempts, 
  getClientIP 
} from '../auth';
import { 
  createChallengeToken, 
  hashDeviceId, 
  isTrustedDevice, 
  countDeviceHistory, 
  trustDevice, 
  createDeviceChallenge,
  touchDeviceSeen
} from '../devices';
import { getConfig } from '../config';
import { sendDeviceApprovalEmail } from '../mailer';
import { logAudit } from '../logger';

export const login = async (req: Request, res: Response) => {
  const { username, password, deviceId, deviceLabel } = req.body;
  const ip = getClientIP(req);
  const userAgent = req.get('User-Agent');
  const cfg = getConfig();
  const deviceApprovalEnabled = cfg.DEVICE_APPROVAL_ENABLED === 'true';
  
  if (!username || !password) {
    res.status(400).json({ error: 'Username and password required' });
    return;
  }

  if (deviceApprovalEnabled) {
    if (!deviceId || typeof deviceId !== 'string' || deviceId.length < 16 || deviceId.length > 256) {
      res.status(400).json({ error: 'Device ID required' });
      return;
    }
  }

  const safeDeviceLabel = typeof deviceLabel === 'string' && deviceLabel.trim() ? deviceLabel.trim().slice(0, 120) : undefined;
  
  const verified = await verifyOperatorCredentials(username, password, ip, userAgent);

  if (!verified) {
    recordLoginAttempt(ip);
    res.status(401).json({ error: 'Authentication failed' });
    return;
  }

  clearLoginAttempts(ip);

  const operatorEmail = verified.email;
  const deviceHash = deviceId ? hashDeviceId(deviceId) : '';

  if (deviceApprovalEnabled) {
    const isTrusted = await isTrustedDevice(operatorEmail, deviceHash);

    if (!isTrusted) {
      const existing = await countDeviceHistory(operatorEmail);

      if (existing === 0) {
        await trustDevice({
          operatorEmail,
          deviceHash,
          deviceLabel: safeDeviceLabel,
          userAgent,
          addedVia: 'bootstrap',
        });

        const confirmUrl = `${cfg.PUBLIC_ORIGIN}/api/v1/devices/confirm?token=bootstrap`;
        await sendDeviceApprovalEmail({
          operatorEmail,
          deviceLabel: safeDeviceLabel,
          userAgent,
          requestedIp: ip,
          confirmUrl,
          kind: 'bootstrap-notice',
        });
      } else {
        const token = createChallengeToken();
        await createDeviceChallenge({
          operatorEmail,
          deviceHash,
          deviceLabel: safeDeviceLabel,
          userAgent,
          requestedIp: ip,
          token,
        });

        const confirmUrl = `${cfg.PUBLIC_ORIGIN}/api/v1/devices/confirm?token=${encodeURIComponent(token)}`;

        try {
          await sendDeviceApprovalEmail({
            operatorEmail,
            deviceLabel: safeDeviceLabel,
            userAgent,
            requestedIp: ip,
            confirmUrl,
            kind: 'approval-required',
          });
        } catch (err) {
          logAudit({
            actor: operatorEmail,
            action: 'device_approval_email_failed',
            result: 'failure',
            reason: (err as Error).message,
            ip,
            userAgent,
          });
          res.status(503).json({
            error: 'DEVICE_APPROVAL_UNAVAILABLE',
            message: 'Cannot send device approval email. Please contact support.',
          });
          return;
        }

        logAudit({
          actor: operatorEmail,
          action: 'login_attempt',
          result: 'denied',
          reason: 'Device not approved',
          ip,
          userAgent,
        });

        res.status(403).json({
          error: 'DEVICE_CONFIRMATION_REQUIRED',
          message: `New device requires approval. A confirmation email was sent to ${cfg.DEVICE_APPROVAL_EMAIL_TO}.`,
        });
        return;
      }
    }
    
    await touchDeviceSeen(operatorEmail, deviceHash);
  }

  const issued = issueOperatorToken({ email: operatorEmail, ip, userAgent });
  res.json({ token: issued.token, expiresIn: `${issued.expiresIn}s` });
};
