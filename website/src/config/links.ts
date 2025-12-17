/**
 * Canonical external links used across the website.
 * Keep all public repo links in one place.
 */

export const REPO_URL = 'https://github.com/sbelakho2/apex-mediation-platform' as const;

/** Build a deep link into the repo tree (main branch). */
export function repoPath(path: string) {
  const clean = path.replace(/^\/+/, '');
  return `${REPO_URL}/tree/main/${clean}`;
}
