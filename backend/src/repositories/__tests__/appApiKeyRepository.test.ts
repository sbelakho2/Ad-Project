import * as repo from '../appApiKeyRepository'
import * as pg from '../../utils/postgres'

jest.mock('../../utils/postgres')

const mockQuery = pg.query as unknown as jest.Mock

function createQueryResult<T>(rows: T[]) {
  return { rows, rowCount: rows.length } as any
}

describe('appApiKeyRepository', () => {
  const publisherId = 'pub'
  const appId = 'app'

  beforeEach(() => {
    mockQuery.mockReset()
  })

  it('listByApp scopes to publisher', async () => {
    mockQuery.mockResolvedValueOnce(
      createQueryResult([
        {
          id: 'k1',
          publisher_id: publisherId,
          app_id: appId,
          environment: 'production',
          secret: 'hash',
          secret_digest: 'digest',
          prefix: 'ak_live',
          last4: '1234',
          created_at: '2025-12-11T00:00:00Z',
          updated_at: '2025-12-11T00:00:00Z',
          last_used_at: null,
          revoked_at: null,
        },
      ])
    )

    const res = await repo.listByApp(publisherId, appId)
    expect(res).toHaveLength(1)
    expect(mockQuery).toHaveBeenCalledWith(expect.stringContaining('a.publisher_id = $2'), [appId, publisherId])
  })

  it('createForApp returns null when app not owned', async () => {
    mockQuery.mockResolvedValueOnce(createQueryResult([]))
    const created = await repo.createForApp(publisherId, appId, { live: true, environment: 'production' })
    expect(created).toBeNull()
  })
})
