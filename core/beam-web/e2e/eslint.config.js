// Plain JavaScript that runs in Node and, inside page.evaluate, in the browser.
import js from '@eslint/js';
import prettier from 'eslint-config-prettier';
import globals from 'globals';

export default [
  js.configs.recommended,
  {
    languageOptions: { globals: { ...globals.node, ...globals.browser } },
    rules: { 'max-lines': ['error', { max: 300 }] },
  },
  prettier,
];
