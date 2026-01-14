module.exports = {
  root: true,
  env: {
    es2021: true,
    node: true,
  },
  parser: '@typescript-eslint/parser',
  parserOptions: {
    project: ['./tsconfig.eslint.json'],
    tsconfigRootDir: __dirname,
    ecmaVersion: 2021,
    sourceType: 'module',
  },
  plugins: ['@typescript-eslint'],
  extends: [
    'eslint:recommended',
    'plugin:@typescript-eslint/recommended',
  ],
  ignorePatterns: ['dist', 'node_modules'],
  rules: {
    '@typescript-eslint/require-await': 'off',
    '@typescript-eslint/no-explicit-any': 'off',
    '@typescript-eslint/no-misused-promises': [
      'error',
      {
        checksConditionals: true,
        checksVoidReturn: false,
        checksSpreads: true,
      },
    ],
    '@typescript-eslint/no-unused-vars': 'off',
    // Allow empty catch blocks to avoid noisy try/catch metrics guards
    'no-empty': ['error', { allowEmptyCatch: true }],
    // Quiet minor formatting nits that are auto-fixable
    'no-extra-semi': 'off',
    // Allow legacy patterns across the existing codebase
    'no-useless-catch': 'off',
    'no-constant-condition': 'off',
    '@typescript-eslint/no-this-alias': 'off',
    '@typescript-eslint/no-var-requires': 'off',
    '@typescript-eslint/ban-types': 'off',
  },
  overrides: [
    {
      files: ['scripts/**/*.js'],
      // These are plain Node.js scripts; don't run typed TS linting on them.
      parser: 'espree',
      parserOptions: {
        ecmaVersion: 2021,
        sourceType: 'script',
      },
      rules: {
        '@typescript-eslint/no-unused-vars': 'off',
        '@typescript-eslint/no-var-requires': 'off',
      },
    },
    {
      files: ['src/controllers/auth.controller.ts'],
      rules: {
        '@typescript-eslint/no-unsafe-assignment': 'off',
        '@typescript-eslint/no-unsafe-argument': 'off',
        '@typescript-eslint/no-unsafe-member-access': 'off',
      },
    },
    {
      files: ['**/__tests__/**/*.ts', '**/__tests__/**/*.tsx'],
      env: {
        jest: true,
      },
      rules: {
        '@typescript-eslint/no-unsafe-assignment': 'off',
        '@typescript-eslint/no-unsafe-member-access': 'off',
        '@typescript-eslint/no-unsafe-return': 'off',
        '@typescript-eslint/no-unsafe-call': 'off',
        '@typescript-eslint/no-unsafe-argument': 'off',
        '@typescript-eslint/unbound-method': 'off',
        '@typescript-eslint/no-explicit-any': 'off',
        '@typescript-eslint/no-unused-vars': 'off',
        '@typescript-eslint/triple-slash-reference': 'off',
        '@typescript-eslint/ban-ts-comment': 'off',
        'no-empty': 'off',
        'no-extra-semi': 'off',
        '@typescript-eslint/no-var-requires': 'off',
      },
    },
    {
      files: [
        'src/services/**/*.ts',
        'src/queues/**/*.ts',
        'src/routes/**/*.ts',
        'src/controllers/**/*.ts',
      ],
      rules: {
        '@typescript-eslint/no-explicit-any': 'off',
      },
    },
  ],
};
