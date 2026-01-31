import { Request, Response } from 'express';
import { 
  isTrustedDevice, 
  confirmDeviceByToken, 
  listTrustedDevices, 
  revokeTrustedDeviceById 
} from '../devices';
import { logAudit } from '../logger';
import { getClientIP } from '../auth';

export const confirmDevice = async (req: Request, res: Response) => {
  const { token } = req.query;
  const ip = getClientIP(req);
  const userAgent = req.get('User-Agent');

  if (!token || typeof token !== 'string') {
    res.status(400).send('Invalid token');
    return;
  }

  try {
    const result = await confirmDeviceByToken(token);
    if (result.ok) {
      res.send('Device approved successfully. You can now log in.');
    } else {
      res.status(400).send(`Approval failed: ${result.reason}`);
    }
  } catch (err) {
    res.status(500).send('Internal server error during device confirmation');
  }
};

export const listDevices = async (req: Request, res: Response) => {
  const operatorEmail = (req as any).operator?.email;
  if (!operatorEmail) {
    res.status(401).json({ error: 'Unauthorized' });
    return;
  }

  try {
    const devices = await listTrustedDevices(operatorEmail);
    res.json(devices);
  } catch (err) {
    res.status(500).json({ error: 'Failed to list devices' });
  }
};

export const revokeDevice = async (req: Request, res: Response) => {
  const { id } = req.params;
  const operatorEmail = (req as any).operator?.email;
  const ip = getClientIP(req);

  if (!id) {
    res.status(400).json({ error: 'Device ID required' });
    return;
  }

  try {
    await revokeTrustedDeviceById({
      deviceId: parseInt(id, 10),
      operatorEmail,
      revokedBy: operatorEmail,
      revokedReason: 'Manual revocation from UI',
    });
    logAudit({
      actor: operatorEmail,
      action: 'device_revoked',
      result: 'success',
      ip,
    });
    res.json({ success: true });
  } catch (err) {
    res.status(500).json({ error: 'Failed to revoke device' });
  }
};
