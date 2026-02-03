# Integration Tests

This directory contains integration tests that validate full request→response flows with a real database.

## Overview

Integration tests use:
- **supertest**: HTTP assertion library for testing Express apps
- **Real database**: PostgreSQL connection for end-to-end validation
- **Test fixtures**: Helper functions to create test data
- **Database cleanup**: Each test runs with a clean database state

## Setup

### 1. Test Database

Integration tests use a separate test database to avoid interfering with development data:

```bash
# Create test database
createdb apexmediation_test

# Or use environment variable to point to test DB
export TEST_DATABASE_URL=postgresql://localhost:5432/apexmediation_test
```

### 2. Run Migrations

Apply schema migrations to the test database:

```bash
# Set database URL to test database
DATABASE_URL=postgresql://localhost:5432/apexmediation_test npm run migrate
```

## Running Tests

```bash
# Run all tests (unit + integration)
npm test

# Run only unit tests
npm run test:unit

# Run only integration tests
npm run test:integration

# Run tests in watch mode
npm run test:watch
```

## Test Structure

### Test Helpers

- **`testDatabase.ts`**: Database setup, teardown, and cleanup utilities
- **`testFixtures.ts`**: Functions to create test data (publishers, users, adapters, etc.)

### Test Suites

- **`auth.integration.test.ts`**: Authentication endpoints (login, register, refresh)
- **`adapterConfig.integration.test.ts`**: Adapter configuration CRUD operations

## Writing Integration Tests

Example structure:

\`\`\`typescript
import request from 'supertest';
import { Pool } from 'pg';
import app from '../../index';
import { setupTestDatabase, teardownTestDatabase, cleanDatabase } from '../helpers/testDatabase';
import { createTestPublisher, createTestUser } from '../helpers/testFixtures';

describe('My Integration Tests', () => {
  let pool: Pool;

  beforeAll(async () => {
    pool = await setupTestDatabase();
  });

  afterAll(async () => {
    await teardownTestDatabase();
  });

  beforeEach(async () => {
    await cleanDatabase(pool); // Clean slate for each test
  });

  it('should test something', async () => {
    // Setup test data
    const publisher = await createTestPublisher(pool);
    const user = await createTestUser(pool, publisher.id);

    // Make HTTP request
    const response = await request(app)
      .get('/api/v1/some-endpoint')
      .set('Authorization', `Bearer ${token}`)
      .expect(200);

    // Assertions
    expect(response.body.success).toBe(true);
  });
});
\`\`\`

## Best Practices

1. **Clean Database**: Always clean the database between tests to ensure isolation
2. **Test Fixtures**: Use helper functions to create consistent test data
3. **Real Database**: Integration tests use real PostgreSQL, not mocks
4. **Auth Flow**: Authenticate via login endpoint to get real tokens
5. **Fast Execution**: Keep tests focused and avoid unnecessary setup
6. **Error Cases**: Test both success and failure scenarios

## Troubleshooting

### Connection Issues

If tests fail with connection errors:

```bash
# Verify test database exists
psql -l | grep apexmediation_test

# Check DATABASE_URL or TEST_DATABASE_URL
echo $TEST_DATABASE_URL
```

### Migration Issues

If schema is out of sync:

```bash
# Drop and recreate test database
dropdb apexmediation_test
createdb apexmediation_test
DATABASE_URL=postgresql://localhost:5432/apexmediation_test npm run migrate
```

### Slow Tests

Integration tests are slower than unit tests due to database operations. To speed up:

- Run unit tests during development: `npm run test:unit`
- Run integration tests before commits: `npm run test:integration`
- Use `--testNamePattern` to run specific tests
