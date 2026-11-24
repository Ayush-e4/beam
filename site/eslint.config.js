import nextCoreWebVitals from 'eslint-config-next/core-web-vitals';
import nextTypescript from 'eslint-config-next/typescript';
import tseslint from 'typescript-eslint';

import base from '../eslint.config.base.js';

export default tseslint.config(
  { ignores: ['.next', 'out', 'next-env.d.ts', 'coverage'] },
  ...nextCoreWebVitals,
  ...nextTypescript,
  ...base,
  {
    languageOptions: {
      parserOptions: { tsconfigRootDir: import.meta.dirname },
    },
  },
);
