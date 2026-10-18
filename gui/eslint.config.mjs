import js from '@eslint/js'
import tseslint from 'typescript-eslint'
import globals from 'globals'
import reactHooks from 'eslint-plugin-react-hooks'

// TypeScript owns type checks; ESLint checks syntax and unsafe source patterns.
export default tseslint.config(
  { ignores: ['dist/**', 'dist-electron/**', 'node_modules/**'] },
  {
    files: ['src/**/*.{ts,tsx}', 'electron/**/*.ts'],
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    languageOptions: { ecmaVersion: 2022, sourceType: 'module' },
  },
  {
    files: ['src/**/*.{ts,tsx}'],
    languageOptions: { globals: globals.browser },
    plugins: { 'react-hooks': reactHooks },
    rules: {
      'react-hooks/rules-of-hooks': 'error',
      'react-hooks/exhaustive-deps': 'error',
    },
  },
  {
    files: ['electron/**/*.ts'],
    languageOptions: { globals: globals.node },
  },
)
