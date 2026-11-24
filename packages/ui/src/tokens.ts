/**
 * Design tokens shared by the web app and the marketing site. Components read them through CSS
 * custom properties so light and dark themes switch without re-rendering.
 */

export type ThemeName = 'light' | 'dark';

export type ColorName = 'bg' | 'surface' | 'fg' | 'muted' | 'accent' | 'danger';

export type ColorTokens = Readonly<Record<ColorName, string>>;

export const colors: Readonly<Record<ThemeName, ColorTokens>> = {
  light: {
    bg: '#f7f7fb',
    surface: '#ffffff',
    fg: '#111118',
    muted: '#5b5b6b',
    accent: '#4f46e5',
    danger: '#dc2626',
  },
  dark: {
    bg: '#0b0b10',
    surface: '#16161f',
    fg: '#f2f2f7',
    muted: '#a0a0b2',
    accent: '#818cf8',
    danger: '#f87171',
  },
};

export const radii = { sm: '6px', md: '12px', lg: '20px', full: '9999px' } as const;

export const durations = { fast: 150, normal: 250, slow: 400 } as const;
