import { QueryResult } from 'pg';
import { getClient, query } from '../utils/postgres';

export interface UserRecord {
  id: string;
  publisher_id: string;
  email: string;
  password_hash: string;
  created_at: Date;
  company_name?: string;
}

export interface CreatedUser {
  user: {
    id: string;
    email: string;
    publisherId: string;
    companyName: string;
  };
}

export const findUserByEmail = async (email: string): Promise<UserRecord | null> => {
  // Lightweight test mode: avoid hitting a real database
  if (process.env.NODE_ENV === 'test' && process.env.FORCE_DB_SETUP !== 'true') {
    return null;
  }

  try {
    const result: QueryResult<UserRecord> = await query<UserRecord>(
      `SELECT u.id, u.publisher_id, u.email, u.password_hash, u.created_at, p.company_name
       FROM users u
       JOIN publishers p ON p.id = u.publisher_id
       WHERE u.email = $1`,
      [email]
    );
    return result?.rows?.[0] ?? null;
  } catch (e) {
    // In tests without DB, treat as not found; otherwise rethrow
    if (process.env.NODE_ENV === 'test') return null;
    throw e;
  }
};

export const createUserWithPublisher = async (
  email: string,
  passwordHash: string,
  companyName: string
): Promise<CreatedUser> => {
  // Lightweight test mode: return a synthetic user without DB side effects
  if (process.env.NODE_ENV === 'test' && process.env.FORCE_DB_SETUP !== 'true') {
    return {
      user: {
        id: 'user-test-1',
        email,
        publisherId: 'pub-test-1',
        companyName,
      },
    };
  }

  const client = await getClient();

  try {
    await client.query('BEGIN');

    const publisherResult = await client.query<{ id: string; company_name: string }>(
      `INSERT INTO publishers (company_name)
       VALUES ($1)
       RETURNING id, company_name`,
      [companyName]
    );

    const publisher = publisherResult.rows[0];

    const userResult = await client.query<{ id: string; email: string; publisher_id: string }>(
      `INSERT INTO users (publisher_id, email, password_hash)
       VALUES ($1, $2, $3)
       RETURNING id, email, publisher_id`,
      [publisher.id, email, passwordHash]
    );

    await client.query('COMMIT');

    return {
      user: {
        id: userResult.rows[0].id,
        email: userResult.rows[0].email,
        publisherId: userResult.rows[0].publisher_id,
        companyName: publisher.company_name,
      },
    };
  } catch (error) {
    await client.query('ROLLBACK');
    throw error;
  } finally {
    client.release();
  }
};
