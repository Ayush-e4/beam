import { colors, radii, type ThemeName } from './tokens';

/** Converts a camelCase token name to a kebab-case CSS custom property suffix. */
function toKebab(name: string): string {
  return name.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`);
}

/** Builds the CSS custom properties for a theme, e.g. `--beam-color-bg`. */
export function cssVariables(theme: ThemeName): Record<string, string> {
  const variables: Record<string, string> = {};

  for (const [name, value] of Object.entries<string>(colors[theme])) {
    variables[`--beam-color-${toKebab(name)}`] = value;
  }
  for (const [name, value] of Object.entries(radii)) {
    variables[`--beam-radius-${toKebab(name)}`] = value;
  }

  return variables;
}
