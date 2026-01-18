/**
 * Team Management Routes
 * 
 * Handles team member invitations, role management, and access control.
 */

import { Router, Request, Response, NextFunction } from 'express';
import { v4 as uuid } from 'uuid';
import { authenticate, authorize } from '../middleware/auth';
import { query } from '../utils/postgres';
import { sendEmail } from '../services/email.service';
import logger from '../utils/logger';

const router = Router();

const isMissingRelationError = (err: any, relation: string) => {
  const code = typeof err?.code === 'string' ? err.code : '';
  const message = typeof err?.message === 'string' ? err.message : '';
  return code === '42P01' && message.toLowerCase().includes(relation.toLowerCase());
};

const isMissingColumnError = (err: any, columnFragment: string) => {
  const code = typeof err?.code === 'string' ? err.code : '';
  const message = typeof err?.message === 'string' ? err.message : '';
  return code === '42703' && message.toLowerCase().includes(columnFragment.toLowerCase());
};

const findExistingUserIdByEmail = async (email: string): Promise<string | null> => {
  const normalized = email.toLowerCase();

  try {
    const r = await query('SELECT id FROM users WHERE email = $1', [normalized]);
    return r.rows[0]?.id ?? null;
  } catch (err) {
    if (!isMissingRelationError(err, 'users')) throw err;
  }

  try {
    const r = await query('SELECT id FROM core.users WHERE email = $1', [normalized]);
    return r.rows[0]?.id ?? null;
  } catch {
    return null;
  }
};

// Invite token expiration in milliseconds (7 days)
const INVITE_TOKEN_EXPIRATION_MS = 7 * 24 * 60 * 60 * 1000;

/**
 * POST /team/invite/accept
 * Accept a team invitation (no authentication required)
 */
router.post('/invite/accept', async (req: Request, res: Response) => {
  try {
    const { token, password, name } = req.body;

    if (!token) {
      return res.status(400).json({ success: false, error: 'Invite token is required' });
    }

    // Find the invite by token
    let invite;
    try {
      invite = await query(
        `SELECT id, publisher_id, email, role, status, created_at
         FROM team_members
         WHERE invite_token = $1`,
        [token]
      );
    } catch (err) {
      if (isMissingColumnError(err, 'invite_token')) {
        return res.status(400).json({ success: false, error: 'Invite system not available' });
      }
      throw err;
    }

    if (invite.rows.length === 0) {
      return res.status(404).json({ success: false, error: 'Invalid or expired invitation' });
    }

    const inviteRecord = invite.rows[0];

    // Check if already accepted
    if (inviteRecord.status === 'active') {
      return res.status(400).json({ success: false, error: 'Invitation already accepted' });
    }

    // Check token expiration (7 days from creation)
    const createdAt = new Date(inviteRecord.created_at).getTime();
    if (Date.now() - createdAt > INVITE_TOKEN_EXPIRATION_MS) {
      return res.status(410).json({ success: false, error: 'Invitation has expired' });
    }

    // Check if user already exists
    const existingUserId = await findExistingUserIdByEmail(inviteRecord.email);

    if (existingUserId) {
      // User exists - just activate the team membership
      await query(
        `UPDATE team_members
         SET status = 'active', user_id = $1, invite_token = NULL, updated_at = NOW()
         WHERE id = $2`,
        [existingUserId, inviteRecord.id]
      );

      return res.json({
        success: true,
        data: {
          message: 'Invitation accepted. Please log in to access your team.',
          email: inviteRecord.email,
          requiresLogin: true,
        },
      });
    }

    // New user - require password
    if (!password || password.length < 8) {
      return res.status(400).json({ success: false, error: 'Password is required (min 8 characters)' });
    }

    // Create new user
    const bcrypt = await import('bcryptjs');
    const hashedPassword = await bcrypt.hash(password, 12);
    const newUserId = uuid();

    try {
      await query(
        `INSERT INTO users (id, email, password, name, created_at, updated_at)
         VALUES ($1, $2, $3, $4, NOW(), NOW())`,
        [newUserId, inviteRecord.email.toLowerCase(), hashedPassword, name || inviteRecord.email.split('@')[0]]
      );
    } catch (err) {
      // Try core.users schema
      try {
        await query(
          `INSERT INTO core.users (id, email, password, name, created_at, updated_at)
           VALUES ($1, $2, $3, $4, NOW(), NOW())`,
          [newUserId, inviteRecord.email.toLowerCase(), hashedPassword, name || inviteRecord.email.split('@')[0]]
        );
      } catch {
        throw err; // Re-throw original error
      }
    }

    // Update team member with user_id and activate
    await query(
      `UPDATE team_members
       SET status = 'active', user_id = $1, invite_token = NULL, updated_at = NOW()
       WHERE id = $2`,
      [newUserId, inviteRecord.id]
    );

    logger.info('Team invite accepted', {
      memberId: inviteRecord.id,
      userId: newUserId,
      publisherId: inviteRecord.publisher_id,
    });

    res.status(201).json({
      success: true,
      data: {
        message: 'Account created and invitation accepted. Please log in.',
        email: inviteRecord.email,
        requiresLogin: true,
      },
    });
  } catch (error) {
    logger.error('Error accepting team invite:', error);
    res.status(500).json({ success: false, error: 'Failed to accept invitation' });
  }
});

// Require authentication for remaining team routes
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

    // Get all team members including invitations.
    // Production has seen at least two table shapes:
    //  1) "new" schema: team_members.user_id + invite_token (+ join to users/core.users)
    //  2) "old" schema: no user_id/invite_token; has name + invited_at/joined_at
    // Prefer richer schema when available; fall back safely.
    let result;

    const runLegacyMembersQuery = async () => {
      return query(
        `SELECT
          tm.id,
          tm.email,
          tm.name,
          tm.role,
          tm.status,
          tm.permissions,
          tm.invited_at,
          tm.joined_at,
          tm.last_active_at,
          tm.created_at,
          tm.updated_at
        FROM team_members tm
        WHERE tm.publisher_id = $1
        ORDER BY COALESCE(tm.invited_at, tm.created_at) DESC`,
        [publisherId]
      );
    };

    try {
      result = await query(
        `SELECT 
          tm.id,
          tm.user_id,
          tm.email,
          tm.role,
          tm.status,
          tm.created_at as invited_at,
          tm.permissions,
          u.email as user_email,
          u.created_at as joined_at
        FROM team_members tm
        LEFT JOIN users u ON tm.user_id = u.id
        WHERE tm.publisher_id = $1
        ORDER BY tm.created_at DESC`,
        [publisherId]
      );
    } catch (err) {
      if (isMissingColumnError(err, 'tm.user_id') || isMissingColumnError(err, 'team_members.user_id')) {
        result = await runLegacyMembersQuery();
      } else if (isMissingRelationError(err, 'users')) {
        try {
          result = await query(
            `SELECT 
              tm.id,
              tm.user_id,
              tm.email,
              tm.role,
              tm.status,
              tm.created_at as invited_at,
              tm.permissions,
              u.email as user_email,
              u.created_at as joined_at
            FROM team_members tm
            LEFT JOIN core.users u ON tm.user_id = u.id
            WHERE tm.publisher_id = $1
            ORDER BY tm.created_at DESC`,
            [publisherId]
          );
        } catch (err2) {
          if (isMissingColumnError(err2, 'tm.user_id') || isMissingColumnError(err2, 'team_members.user_id')) {
            result = await runLegacyMembersQuery();
          } else {
            // Fall back to no user join.
            result = await query(
              `SELECT 
                tm.id,
                tm.user_id,
                tm.email,
                tm.role,
                tm.status,
                tm.created_at as invited_at,
                tm.permissions,
                NULL::text as user_email,
                NULL::timestamptz as joined_at
              FROM team_members tm
              WHERE tm.publisher_id = $1
              ORDER BY tm.created_at DESC`,
              [publisherId]
            );
          }
        }
      } else {
        throw err;
      }
    }

    const members = result.rows.map((row: any) => ({
      id: row.id,
      email: row.user_email || row.email,
      name: row.name || row.user_email?.split('@')[0] || row.email?.split('@')[0],
      role: row.role,
      status: row.status,
      invitedAt: row.invited_at,
      joinedAt: row.joined_at,
      permissions:
        (Array.isArray(row.permissions) ? row.permissions : undefined) ||
        ROLE_DEFINITIONS.find((r) => r.id === row.role)?.permissions ||
        [],
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
    const err = error as any;
    const code = typeof err?.code === 'string' ? err.code : '';
    const message = typeof err?.message === 'string' ? err.message : '';

    // Production safety: if the table isn't migrated yet, don't break the console.
    // Return the current user as the sole "owner" member.
    if (code === '42P01' && message.includes('team_members')) {
      const fallback = [
        {
          id: req.user?.userId,
          email: req.user?.email,
          name: req.user?.email?.split('@')[0],
          role: 'owner',
          status: 'active',
          invitedAt: undefined,
          joinedAt: undefined,
          permissions: ['*'],
        },
      ].filter((m) => Boolean(m.email));

      return res.json({ success: true, data: fallback });
    }

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

    // Check if user already has an account (public.users or core.users).
    const existingUserId = await findExistingUserIdByEmail(email);

    const memberId = uuid();
    const inviteToken = uuid();

    const normalizedEmail = email.toLowerCase();
    const status = existingUserId ? 'active' : 'invited';

    try {
      await query(
        `INSERT INTO team_members (id, publisher_id, email, role, status, invite_token, user_id, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())`,
        [memberId, publisherId, normalizedEmail, role, status, inviteToken, existingUserId]
      );
    } catch (err) {
      if (isMissingColumnError(err, 'invite_token') || isMissingColumnError(err, 'user_id')) {
        // Legacy schema: no invite_token/user_id; store name + invited_at/joined_at instead.
        await query(
          `INSERT INTO team_members (
            id,
            publisher_id,
            email,
            name,
            role,
            status,
            permissions,
            invited_at,
            joined_at,
            created_at,
            updated_at
          ) VALUES ($1,$2,$3,$4,$5,$6,$7,NOW(),$8,NOW(),NOW())`,
          [
            memberId,
            publisherId,
            normalizedEmail,
            normalizedEmail.split('@')[0],
            role,
            status,
            JSON.stringify(roleDefinition.permissions),
            status === 'active' ? new Date().toISOString() : null,
          ]
        );
      } else {
        throw err;
      }
    }

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
      status,
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
    let existing;
    try {
      existing = await query('SELECT id, role, user_id, email FROM team_members WHERE id = $1 AND publisher_id = $2', [
        id,
        publisherId,
      ]);
    } catch (err) {
      if (isMissingColumnError(err, 'user_id')) {
        existing = await query('SELECT id, role, email FROM team_members WHERE id = $1 AND publisher_id = $2', [
          id,
          publisherId,
        ]);
      } else {
        throw err;
      }
    }

    if (existing.rows.length === 0) {
      return res.status(404).json({ success: false, error: 'Team member not found' });
    }

    // Cannot remove owner
    if (existing.rows[0].role === 'owner') {
      return res.status(403).json({ success: false, error: 'Cannot remove organization owner' });
    }

    // Cannot remove yourself
    if (existing.rows[0].user_id && existing.rows[0].user_id === req.user?.userId) {
      return res.status(403).json({ success: false, error: 'Cannot remove yourself' });
    }
    if (!existing.rows[0].user_id && existing.rows[0].email && req.user?.email) {
      if (String(existing.rows[0].email).toLowerCase() === String(req.user.email).toLowerCase()) {
        return res.status(403).json({ success: false, error: 'Cannot remove yourself' });
      }
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
      let existing;
      try {
        existing = await query(
          `SELECT id, email, role, status, invite_token 
           FROM team_members 
           WHERE id = $1 AND publisher_id = $2`,
          [id, publisherId]
        );
      } catch (err) {
        if (isMissingColumnError(err, 'invite_token')) {
          existing = await query(
            `SELECT id, email, role, status
             FROM team_members
             WHERE id = $1 AND publisher_id = $2`,
            [id, publisherId]
          );
        } else {
          throw err;
        }
      }

      if (existing.rows.length === 0) {
        return res.status(404).json({ success: false, error: 'Team member not found' });
      }

      if (existing.rows[0].status !== 'invited') {
        return res.status(400).json({ success: false, error: 'Member is already active' });
      }

      const member = existing.rows[0];
      const roleDefinition = ROLE_DEFINITIONS.find((r) => r.id === member.role);

      // If invite_token exists, regenerate it. Otherwise, best-effort resend without a token.
      let inviteUrl = `${process.env.FRONTEND_URL || 'https://console.apexmediation.ee'}/signup`;
      if ('invite_token' in member) {
        const newToken = uuid();
        await query('UPDATE team_members SET invite_token = $1, updated_at = NOW() WHERE id = $2', [newToken, id]);
        inviteUrl = `${process.env.FRONTEND_URL || 'https://console.apexmediation.ee'}/invite/${newToken}`;
      }

      // Resend email
      await sendEmail({
        to: member.email,
        subject: 'Reminder: You have been invited to join a team on Apex Mediation',
        template: 'team-invite',
        data: {
          inviterEmail: req.user?.email,
          role: roleDefinition?.label || member.role,
          inviteUrl,
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
