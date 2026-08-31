import wordmarkDark from "./assets/wordmark.svg";
import wordmarkLight from "./assets/wordmark-light.svg";

export type Theme = "dark" | "light";

/**
 * The app is dark-only today.
 *
 * This is deliberately not wired to `prefers-color-scheme`: the light wordmark
 * has dark lettering, so following the OS while the chrome stays dark would put
 * near-invisible text in the caption bar. It follows the *app's* theme, and when
 * a light theme lands this constant becomes state.
 */
export const THEME: Theme = "dark";

/** The wordmark that reads on the given theme's chrome. */
export function wordmarkFor(theme: Theme = THEME): string {
  return theme === "light" ? wordmarkLight : wordmarkDark;
}
