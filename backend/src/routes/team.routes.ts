/**
 * Team Management Routes
 * 
 * Handles team member invitations, role management, and access control.
 */

import { Router, Request, Response, NextFunction } from 'express';
import { v4 as uuid } from 'uuid';
import { authenticate, authorize } from '../middleware/auth';
import { query } from '../utils/postgres';
import logger from '../utils/logger';

const router = Router();

// Stub email service - TODO: implement proper email sending
const sendEmail = async (options: { to: string; subject: string; template: string; data: any }): Promise<void> => {
  logger.info('Email send stub:', { to: options.to, subject: options.subject });
  // In production, this would send via SendGrid, AWS SES, etc.
  return Promise.resolve();
};

// Require authentication for all team routes
router.use(authenticate);

// Role definitions
const ROLE_DEFINITIONS = [
  {
    id: 'admin',
    label: 'Admin',
    description: 'Full access to settings, placements, and team management',
    permissions: ['*'],
    invitable: true,
  },
  {
    id: 'developer',
    label: 'Developer',
    description: 'Manage placements and view analytics, no billing access',
    permissions: ['placements:*', 'analytics:read', 'apps:*', 'adapters:read'],
    invitable: true,
  },
  {
    id: 'finance',
    label: 'Finance',
    description: 'View revenue, manage billing, limited placement access',
    permissions: ['billing:*', 'analytics:read', 'revenue:read', 'placements:read'],
    invitable: true,
  },
  {
    id: 'owner',
    label: 'Owner',
    description: 'Organization owner with all permissions',
    permissions: ['*'],
    invitable: false, // Cannot invite more owners
  },
];

/**
 * GET /team/roles
 * List available role definitions
 */
router.get('/roles', async (req: Request, res: Response) => {
  try {
    res.json({
      success: true,
      data: ROLE_DEFINITIONS,
    });
  } catch (error) {
    logger.error('Error fetching roles:', error);
    res.status(500).json({ success: false, error: 'Failed to fetch roles' });
  }
});

/**
 * GET /team/members
 * List all team members for the publisher
 */
router.get('/members', async (req: Request, res: Response) => {
  try {
    const publisherId = req.user?.publisherId;

    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Unauthorized' });
    }

    // Get all team members including invitations
    const result = await query(
      `SELECT 
        tm.id,
        tm.user_id,
        tm.email,
        tm.role,
        tm.status,
        tm.created_at as invited_at,
        u.email as user_email,
        u.created_at as joined_at
      FROM team_members tm
      LEFT JOIN users u ON tm.user_id = u.id
      WHERE tm.publisher_id = $1
      ORDER BY tm.created_at DESC`,
      [publisherId]
    );

    const members = result.rows.map((row: any) => ({
      id: row.id,
      email: row.user_email || row.email,
      name: row.user_email?.split('@')[0] || row.email?.split('@')[0],
      role: row.role,
      status: row.status,
      invitedAt: row.invited_at,
      joinedAt: row.joined_at,
      permissions: ROLE_DEFINITIONS.find((r) => r.id === row.role)?.permissions || [],
    }));

    // Add current user if not in the list (as owner)
    const currentUserInList = members.some((m: any) => m.email === req.user?.email);
    if (!currentUserInList) {
      members.unshift({
        id: req.user?.userId,
        email: req.user?.email,
        name: req.user?.email?.split('@')[0],
        role: 'owner',
        status: 'active',
        invitedAt: undefined,
        joinedAt: undefined,
        permissions: ['*'],
      });
    }

    res.json({ success: true, data: members });
  } catch (error) {
    logger.error('Error listing team members:', error);
    res.status(500).json({ success: false, error: 'Failed to fetch team members' });
  }
});

/**
 * POST /team/invite
 * Invite a new team member
 */
router.post('/invite', authorize(['admin']), async (req: Request, res: Response) => {
  try {
    const publisherId = req.user?.publisherId;
    const { email, role, permissions } = req.body;

    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Unauthorized' });
    }

    if (!email || !role) {
      return res.status(400).json({ success: false, error: 'Email and role are required' });
    }

    // Check if role is invitable
    const roleDefinition = ROLE_DEFINITIONS.find((r) => r.id === role);
    if (!roleDefinition || roleDefinition.invitable === false) {
      return res.status(400).json({ success: false, error: 'Invalid role for invitation' });
    }

    // Check if member already exists
    const existing = await query(
      'SELECT id FROM team_members WHERE publisher_id = $1 AND email = $2',
      [publisherId, email.toLowerCase()]
    );

    if (existing.rows.length > 0) {
      return res.status(409).json({ success: false, error: 'Team member already exists' });
    }

    // Check if user already has an account
    const existingUser = await query('SELECT id FROM users WHERE email = $1', [email.toLowerCase()]);

    const memberId = uuid();
    const inviteToken = uuid();

    await query(
      `INSERT INTO team_members (id, publisher_id, email, role, status, invite_token, user_id, created_at)
       VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())`,
      [
        memberId,
        publisherId,
        email.toLowerCase(),
        role,
        existingUser.rows.length > 0 ? 'active' : 'invited',
        inviteToken,
        existingUser.rows[0]?.id || null,
      ]
    );

    // Send invitation email (async, don't block)
    sendEmail({
      to: email,
      subject: 'You have been invited to join a team on Apex Mediation',
      template: 'team-invite',
      data: {
        inviterEmail: req.user?.email,
        role: roleDefinition.label,
        inviteUrl: `${process.env.FRONTEND_URL || 'https://console.apexmediation.ee'}/invite/${inviteToken}`,
      },
    }).catch((err) => logger.warn('Failed to send invite email:', err));

    const newMember = {
      id: memberId,
      email: email.toLowerCase(),
      name: email.split('@')[0],
      role,
      status: existingUser.rows.length > 0 ? 'active' : 'invited',
      invitedAt: new Date().toISOString(),
      permissions: roleDefinition.permissions,
    };

    res.status(201).json({ success: true, data: newMember });
  } catch (error) {
    logger.error('Error inviting team member:', error);
    res.status(500).json({ success: false, error: 'Failed to send invitation' });
  }
});

/**
 * PUT /team/members/:id
 * Update a team member's role
 */
router.put('/members/:id', authorize(['admin']), async (req: Request, res: Response) => {
  try {
    const publisherId = req.user?.publisherId;
    const { id } = req.params;
    const { role } = req.body;

    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Unauthorized' });
    }

    // Cannot change owner role
    const roleDefinition = ROLE_DEFINITIONS.find((r) => r.id === role);
    if (!roleDefinition) {
      return res.status(400).json({ success: false, error: 'Invalid role' });
    }

    // Check if member exists and belongs to this publisher
    const existing = await query(
      'SELECT id, role FROM team_members WHERE id = $1 AND publisher_id = $2',
      [id, publisherId]
    );

    if (existing.rows.length === 0) {
      return res.status(404).json({ success: false, error: 'Team member not found' });
    }

    // Cannot demote owner
    if (existing.rows[0].role === 'owner') {
      return res.status(403).json({ success: false, error: 'Cannot change owner role' });
    }

    await query('UPDATE team_members SET role = $1, updated_at = NOW() WHERE id = $2', [role, id]);

    res.json({
      success: true,
      data: {
        id,
        role,
        permissions: roleDefinition.permissions,
      },
    });
  } catch (error) {
    logger.error('Error updating team member:', error);
    res.status(500).json({ success: false, error: 'Failed to update team member' });
  }
});

/**
 * DELETE /team/members/:id
 * Remove a team member
 */
router.delete('/members/:id', authorize(['admin']), async (req: Request, res: Response) => {
  try {
    const publisherId = req.user?.publisherId;
    const { id } = req.params;

    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Unauthorized' });
    }

    // Check if member exists and belongs to this publisher
    const existing = await query(
      'SELECT id, role, user_id FROM team_members WHERE id = $1 AND publisher_id = $2',
      [id, publisherId]
    );

    if (existing.rows.length === 0) {
      return res.status(404).json({ success: false, error: 'Team member not found' });
    }

    // Cannot remove owner
    if (existing.rows[0].role === 'owner') {
      return res.status(403).json({ success: false, error: 'Cannot remove organization owner' });
    }

    // Cannot remove yourself
    if (existing.rows[0].user_id === req.user?.userId) {
      return res.status(403).json({ success: false, error: 'Cannot remove yourself' });
    }

    await query('DELETE FROM team_members WHERE id = $1', [id]);

    res.json({ success: true });
  } catch (error) {
    logger.error('Error removing team member:', error);
    res.status(500).json({ success: false, error: 'Failed to remove team member' });
  }
});

/**
 * POST /team/members/:id/resend
 * Resend invitation email
 */
router.post(
  '/members/:id/resend',
  authorize(['admin']),
  async (req: Request, res: Response) => {
    try {
      const publisherId = req.user?.publisherId;
      const { id } = req.params;

      if (!publisherId) {
        return res.status(401).json({ success: false, error: 'Unauthorized' });
      }

      // Check if member exists and is in invited status
      const existing = await query(
        `SELECT id, email, role, status, invite_token 
         FROM team_members 
         WHERE id = $1 AND publisher_id = $2`,
        [id, publisherId]
      );

      if (existing.rows.length === 0) {
        return res.status(404).json({ success: false, error: 'Team member not found' });
      }

      if (existing.rows[0].status !== 'invited') {
        return res.status(400).json({ success: false, error: 'Member is already active' });
      }

      const member = existing.rows[0];
      const roleDefinition = ROLE_DEFINITIONS.find((r) => r.id === member.role);

      // Regenerate invite token
      const newToken = uuid();
      await query('UPDATE team_members SET invite_token = $1, updated_at = NOW() WHERE id = $2', [
        newToken,
        id,
      ]);

      // Resend email
      await sendEmail({
        to: member.email,
        subject: 'Reminder: You have been invited to join a team on Apex Mediation',
        template: 'team-invite',
        data: {
          inviterEmail: req.user?.email,
          role: roleDefinition?.label || member.role,
          inviteUrl: `${process.env.FRONTEND_URL || 'https://console.apexmediation.ee'}/invite/${newToken}`,
        },
      });

      res.json({ success: true });
    } catch (error) {
      logger.error('Error resending invitation:', error);
      res.status(500).json({ success: false, error: 'Failed to resend invitation' });
    }
  }
);

export default router;
