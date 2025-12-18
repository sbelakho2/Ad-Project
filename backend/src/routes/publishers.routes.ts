import { Router } from 'express'
import { authenticate } from '../middleware/auth'
import { query } from '../utils/postgres'

const router = Router()

router.use(authenticate)

// GET /api/v1/publishers/me - Get current publisher and user info
router.get('/me', async (req, res, next) => {
  try {
    const publisherId = req.user?.publisherId
    const userId = req.user?.userId
    
    if (!publisherId || !userId) {
      return res.status(401).json({ success: false, error: 'Unauthorized' })
    }

    // Get publisher info
    const publisherResult = await query<{ id: string; company_name: string; created_at: string }>(
      `SELECT id, company_name, created_at FROM publishers WHERE id = $1`,
      [publisherId]
    )

    // Get user info
    const userResult = await query<{ id: string; email: string; role: string; created_at: string }>(
      `SELECT id, email, role, created_at FROM users WHERE id = $1`,
      [userId]
    )

    const publisher = publisherResult.rows[0]
    const user = userResult.rows[0]

    if (!publisher || !user) {
      return res.status(404).json({ success: false, error: 'Not found' })
    }

    return res.json({
      success: true,
      data: {
        publisher: {
          id: publisher.id,
          companyName: publisher.company_name,
          createdAt: publisher.created_at,
        },
        user: {
          id: user.id,
          email: user.email,
          role: user.role || 'publisher',
          createdAt: user.created_at,
        },
      },
    })
  } catch (e) {
    next(e)
  }
})

export default router
