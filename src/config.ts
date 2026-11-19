export interface Config {
  /** Wiki root without a trailing slash, e.g. `https://wiki.example.com` (a base path is kept). */
  baseUrl: string
  /** Empty when the wiki runs without sign-in (`--disable-auth`). */
  username: string
  password: string
  /** Register only the `read_` tools. */
  readOnly: boolean
}

export class ConfigError extends Error {
  override name = 'ConfigError'
}

const LOCAL = ['localhost', '127.0.0.1', '[::1]']
const TRUE = ['1', 'true', 'yes', 'on']
const FALSE = ['0', 'false', 'no', 'off', '']

/** Reads and validates the environment. Throws `ConfigError` listing every problem at once. */
export function loadConfig(env: NodeJS.ProcessEnv = process.env): Config {
  const problems: string[] = []

  const rawUrl = env.LEAFWIKI_URL?.trim()
  let baseUrl = ''
  if (!rawUrl) {
    problems.push('LEAFWIKI_URL is required, e.g. https://wiki.example.com')
  } else {
    try {
      const url = new URL(rawUrl)
      if (url.protocol !== 'https:' && !(url.protocol === 'http:' && LOCAL.includes(url.hostname))) {
        problems.push(`LEAFWIKI_URL must be an https URL (the sign-in cookies travel with every request), got "${rawUrl}"`)
      } else if (/\/api(\/|$)/i.test(url.pathname)) {
        problems.push(`LEAFWIKI_URL must be the wiki root without /api, got "${rawUrl}"`)
      } else if (url.search || url.hash) {
        problems.push('LEAFWIKI_URL must not contain a query string or fragment')
      } else {
        baseUrl = `${url.origin}${url.pathname}`.replace(/\/+$/, '')
      }
    } catch {
      problems.push(`LEAFWIKI_URL is not a valid URL: "${rawUrl}"`)
    }
  }

  const username = env.LEAFWIKI_USERNAME?.trim() ?? ''
  const password = env.LEAFWIKI_PASSWORD ?? ''
  if (Boolean(username) !== Boolean(password)) {
    problems.push('Set both LEAFWIKI_USERNAME and LEAFWIKI_PASSWORD, or neither when the wiki runs without sign-in')
  }

  const rawReadOnly = env.BURROW_READ_ONLY?.trim().toLowerCase() ?? ''
  let readOnly = false
  if (TRUE.includes(rawReadOnly)) readOnly = true
  else if (!FALSE.includes(rawReadOnly)) problems.push(`BURROW_READ_ONLY must be true or false, got "${env.BURROW_READ_ONLY}"`)

  if (problems.length > 0) throw new ConfigError(`Invalid burrow configuration:\n- ${problems.join('\n- ')}`)
  return { baseUrl, username, password, readOnly }
}
