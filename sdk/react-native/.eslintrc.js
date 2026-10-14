module.exports = {
  root: true,
  parser: '@typescript-eslint/parser',
  parserOptions: { ecmaVersion: 2021, sourceType: 'module', ecmaFeatures: { jsx: true } },
  plugins: ['@typescript-eslint'],
  extends: ['eslint:recommended', 'plugin:@typescript-eslint/recommended'],
  rules: {
    // The native bridge surface legitimately carries `any` payloads.
    '@typescript-eslint/no-explicit-any': 'off',
    // Underscore-prefixed names are deliberately unused test doubles.
    '@typescript-eslint/no-unused-vars': [
      'error',
      { argsIgnorePattern: '^_', varsIgnorePattern: '^_', caughtErrorsIgnorePattern: '^_' },
    ],
  },
  env: { node: true, es2021: true },
  ignorePatterns: ['lib/', 'node_modules/', 'example/', 'coverage/'],
};
