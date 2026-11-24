import reactHooks from 'eslint-plugin-react-hooks';
import reactRefresh from 'eslint-plugin-react-refresh';
import globals from 'globals';
import tseslint from 'typescript-eslint';

import base from '../eslint.config.base.js';

export default tseslint.config({ ignores: ['dist', 'coverage'] }, ...base, {
  languageOptions: {
    globals: globals.browser,
    parserOptions: { tsconfigRootDir: import.meta.dirname },
  },
  plugins: { 'react-hooks': reactHooks, 'react-refresh': reactRefresh },
  rules: {
    ...reactHooks.configs.recommended.rules,
    'react-refresh/only-export-components': ['error', { allowConstantExport: true }],
  },
});
