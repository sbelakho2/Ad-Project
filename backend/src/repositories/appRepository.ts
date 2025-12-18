import { query } from '../utils/postgres'

export type AppRow = {
  id: string
  publisher_id: string
  name: string
  bundle_id: string
  platform: string
  created_at: string
}

export type App = {
  id: string
  publisherId: string
  name: string
  bundleId: string
  platform: string
  createdAt: string
}

export type CreateAppInput = {
  name: string
  bundleId: string
  platform: string
}

export type UpdateAppInput = {
  name?: string
  bundleId?: string
}

const toApp = (row: AppRow): App => ({
  id: row.id,
  publisherId: row.publisher_id,
  name: row.name,
  bundleId: row.bundle_id,
  platform: row.platform,
  createdAt: row.created_at,
})

export async function listByPublisher(publisherId: string): Promise<App[]> {
  const res = await query<AppRow>(
    `SELECT id, publisher_id, name, bundle_id, platform, created_at
       FROM apps
      WHERE publisher_id = $1
      ORDER BY name ASC`,
    [publisherId]
  )
  return res.rows.map(toApp)
}

export async function getById(publisherId: string, appId: string): Promise<App | null> {
  const res = await query<AppRow>(
    `SELECT id, publisher_id, name, bundle_id, platform, created_at
       FROM apps
      WHERE id = $1 AND publisher_id = $2`,
    [appId, publisherId]
  )
  return res.rows[0] ? toApp(res.rows[0]) : null
}

export async function create(publisherId: string, input: CreateAppInput): Promise<App> {
  const res = await query<AppRow>(
    `INSERT INTO apps (publisher_id, name, bundle_id, platform)
     VALUES ($1, $2, $3, $4)
     RETURNING id, publisher_id, name, bundle_id, platform, created_at`,
    [publisherId, input.name, input.bundleId, input.platform]
  )
  return toApp(res.rows[0])
}

export async function update(publisherId: string, appId: string, input: UpdateAppInput): Promise<App | null> {
  const sets: string[] = []
  const values: any[] = []
  let paramIndex = 1

  if (input.name !== undefined) {
    sets.push(`name = $${paramIndex++}`)
    values.push(input.name)
  }
  if (input.bundleId !== undefined) {
    sets.push(`bundle_id = $${paramIndex++}`)
    values.push(input.bundleId)
  }

  if (sets.length === 0) {
    return getById(publisherId, appId)
  }

  values.push(appId, publisherId)

  const res = await query<AppRow>(
    `UPDATE apps SET ${sets.join(', ')}
     WHERE id = $${paramIndex++} AND publisher_id = $${paramIndex}
     RETURNING id, publisher_id, name, bundle_id, platform, created_at`,
    values
  )
  return res.rows[0] ? toApp(res.rows[0]) : null
}

export async function deleteApp(publisherId: string, appId: string): Promise<boolean> {
  const res = await query(
    `DELETE FROM apps WHERE id = $1 AND publisher_id = $2`,
    [appId, publisherId]
  )
  return (res.rowCount ?? 0) > 0
}
