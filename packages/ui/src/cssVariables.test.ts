import { cssVariables } from './cssVariables';
import { colors } from './tokens';

describe('cssVariables', () => {
  it('maps every color and radius token to a custom property', () => {
    const vars = cssVariables('light');

    expect(vars['--beam-color-bg']).toBe(colors.light.bg);
    expect(vars['--beam-color-accent']).toBe(colors.light.accent);
    expect(vars['--beam-radius-full']).toBe('9999px');
  });

  it('uses the requested theme', () => {
    expect(cssVariables('dark')['--beam-color-bg']).toBe(colors.dark.bg);
  });

  it('gives both themes the same set of variables', () => {
    expect(Object.keys(cssVariables('dark'))).toEqual(Object.keys(cssVariables('light')));
  });
});
