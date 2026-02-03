# Backend Test Structure

This directory contains all tests for the backend service, organized by type.

## Directory Structure

```
tests/
├── setup.ts              # Jest global setup (DB connections, env vars)
├── helpers/              # Shared test utilities and fixtures
│   ├── testApp.ts        # Express app factory for testing
│   ├── testDatabase.ts   # DB setup/teardown helpers
│   └── testFixtures.ts   # Factory functions for test data
├── unit/                 # Unit tests (isolated, no DB required)
│   ├── controllers/      # Controller unit tests
│   ├── services/         # Service unit tests
│   └── queues/           # Queue processor unit tests
├── integration/          # Integration tests (require DB)
│   └── *.integration.test.ts
├── ssai/                 # SSAI-specific tests
└── sales-automation/     # Sales automation tests
```

## Running Tests

### Lightweight Mode (No DB)
```bash
npm run test:backend
```

This runs unit tests only. Integration tests are skipped unless `FORCE_DB_SETUP=true`.

### Full Mode (With DB)
```bash
FORCE_DB_SETUP=true npm run test:backend
```

This runs all tests including DB-backed integration tests.

### Specific Test File
```bash
npm run test:backend -- --testPathPattern=analytics
```

## Writing Tests

### Unit Tests
Place in `tests/unit/` with the pattern `*.test.ts`. These should:
- Mock all external dependencies
- Not require database or network access
- Be fast and deterministic

### Integration Tests
Place in `tests/integration/` with the pattern `*.integration.test.ts`. These should:
- Test real database interactions
- Use `setupTestDatabase()` and `cleanDatabase()` helpers
- Be guarded with `FORCE_DB_SETUP` environment variable

## Migration Note

Tests were consolidated from the legacy `src/__tests__/` location to this directory.
The old location (`src/__tests__/`) is deprecated and ignored by Jest.
All new tests should be placed here.
