import tseslint from 'typescript-eslint';

import base from '../eslint.config.base.js';

export default tseslint.config({ ignores: ['dist', '.wrangler', 'coverage'] }, ...base, {
  languageOptions: {
    parserOptions: { tsconfigRootDir: import.meta.dirname },
  },
});
