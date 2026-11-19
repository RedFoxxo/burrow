import type { Logger } from '../log.js'
import { err, ok, type Err, type Result } from './result.js'

export type FetchLike = (input: string, init?: RequestInit) => Promise<Response>

export interface SessionOptions {
  baseUrl: string
  username: string
  password: string
  fetch?: FetchLike
  log?: Logger
  timeoutMs?: number
  /** Clock in ms, for tests. */
  now?: () => number
}

export type QueryValue = string | number | boolean | undefined | null

export interface RequestSpec {
  method: 'GET' | 'POST' | 'PUT' | 'DELETE'
  /** Path below `/api`, starting with `/`. */
  path: string
  query?: Record<string, QueryValue | QueryValue[]>
  json?: unknown
  form?: FormData
}

const CSRF_COOKIE = 'leafwiki_csrf'
const DEFAULT_TIMEOUT_MS = 60_000
/** Refresh this long before the access token runs out (LeafWiki's default lifetime is 15 minutes). */
const REFRESH_MARGIN_MS = 60_000

/**
 * The single place where burrow talks to LeafWiki. Signs in the way the web app
 * does: session cookies plus a CSRF token echoed in `X-CSRF-Token` on every
 * change. Refreshes the access token before it runs out, and signs in again
 * once when LeafWiki answers 401 (nothing was done then, so any request can be
 * repeated). Never follows redirects, so cookies and the password only go to
 * `baseUrl`. Every outcome is a `Result`.
 */
export class Session {
  readonly baseUrl: string
  private readonly username: string
  private readonly password: string
  private readonly fetchImpl: FetchLike
  private readonly log: Logger
  private readonly timeoutMs: number
  private readonly now: () => number
  private readonly cookies = new Map<string, string>()
  private csrf = ''
  /** When the access token runs out (ms); 0 when not signed in. */
  private expiresAt = 0
  private authDisabled = false
  private signingIn: Promise<Err | undefined> | undefined
  /** Counts successful sign-ins and refreshes, to tell a stale 401 from a lost session. */
  private signIns = 0
  /**
   * A sign-in failure that repeats until the configuration changes (wrong
   * password, TOTP account): not retried, since every failed attempt counts
   * toward LeafWiki's account lockout.
   */
  private rejected: Err | undefined

  constructor(options: SessionOptions) {
    this.baseUrl = options.baseUrl
    this.username = options.username
    this.password = options.password
    this.fetchImpl = options.fetch ?? ((input, init) => fetch(input, init))
    this.log = options.log ?? (() => {})
    this.timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS
    this.now = options.now ?? Date.now
  }

  /** Absolute URL of a path LeafWiki returns relative to its root (`/assets/…`). */
  absolute(path: string): string {
    return `${this.baseUrl}${path.startsWith('/') ? path : `/${path}`}`
  }

  async request<T>(spec: RequestSpec): Promise<Result<T>> {
    const target = `${this.baseUrl}/api${spec.path}`
    if (URL.parse(target)?.href !== target) {
      return err(0, `Not sent: "${spec.path}" would not reach the intended LeafWiki route`, '', { request: `${spec.method} /api${spec.path}` })
    }
    const ready = await this.ensureSignedIn()
    if (ready) return ready
    const signIns = this.signIns
    const first = await this.send<T>(spec)
    if (first.ok || first.status !== 401 || this.authDisabled) return first
    // A sign-in finished while this call was under way: its cookies were old, the session is fine.
    if (signIns === this.signIns) {
      this.expiresAt = 0
      const again = await this.signIn(true)
      if (again) return again
    }
    return this.send<T>(spec)
  }

  private async ensureSignedIn(): Promise<Err | undefined> {
    if (this.authDisabled) return undefined
    if (this.expiresAt && this.now() < this.expiresAt - REFRESH_MARGIN_MS) return undefined
    return this.signIn(Boolean(this.expiresAt))
  }

  /** One sign-in at a time: parallel tool calls wait for the same one. */
  private signIn(refreshFirst: boolean): Promise<Err | undefined> {
    if (this.rejected) return Promise.resolve(this.rejected)
    this.signingIn ??= this.doSignIn(refreshFirst).finally(() => {
      this.signingIn = undefined
    })
    return this.signingIn
  }

  private async doSignIn(refreshFirst: boolean): Promise<Err | undefined> {
    if (refreshFirst && this.cookies.size) {
      const refreshed = await this.send<{ accessTokenExpiresAt?: number }>({ method: 'POST', path: '/auth/refresh-token' })
      if (refreshed.ok) {
        this.setExpiry(refreshed.data.accessTokenExpiresAt)
        this.signIns++
        return undefined
      }
      this.log(`refresh failed (${refreshed.status}), signing in again`)
    }

    // `/config` issues the CSRF cookie and says whether the wiki has sign-in at all.
    const config = await this.send<{ authDisabled?: boolean }>({ method: 'GET', path: '/config' })
    if (!config.ok) return config
    if (config.data.authDisabled) {
      this.authDisabled = true
      return undefined
    }
    if (!this.username) {
      this.rejected = err(0, 'This wiki needs sign-in: set LEAFWIKI_USERNAME and LEAFWIKI_PASSWORD, then restart the MCP client')
      return this.rejected
    }

    const login = await this.send<{ requiresTotp?: boolean; accessTokenExpiresAt?: number }>({
      method: 'POST',
      path: '/auth/login',
      json: { identifier: this.username, password: this.password },
    })
    if (!login.ok) {
      if (login.code !== 'auth_invalid_credentials') return login
      this.rejected = {
        ...login,
        message: `Sign-in as "${this.username}" failed: ${login.message}. Check LEAFWIKI_USERNAME and LEAFWIKI_PASSWORD, then restart the MCP client`,
      }
      return this.rejected
    }
    if (login.data.requiresTotp) {
      this.rejected = err(
        0,
        `The account "${this.username}" uses two-factor codes (TOTP), which burrow cannot enter. Use an account without TOTP`,
      )
      return this.rejected
    }
    this.setExpiry(login.data.accessTokenExpiresAt)
    this.signIns++
    this.log(`signed in as ${this.username}`)
    return undefined
  }

  private setExpiry(seconds: number | undefined): void {
    // Unknown lifetime: assume LeafWiki's default 15 minutes.
    this.expiresAt = seconds ? seconds * 1000 : this.now() + 15 * 60_000
  }

  private url(spec: RequestSpec): string {
    const params = new URLSearchParams()
    for (const [key, value] of Object.entries(spec.query ?? {})) {
      for (const v of Array.isArray(value) ? value : [value]) {
        if (v === undefined || v === null || v === '') continue
        params.append(key, String(v))
      }
    }
    const qs = params.toString()
    return `${this.baseUrl}/api${spec.path}${qs ? `?${qs}` : ''}`
  }

  private async send<T>(spec: RequestSpec): Promise<Result<T>> {
    const request = `${spec.method} /api${spec.path}`
    const headers: Record<string, string> = { accept: 'application/json' }
    if (this.cookies.size) headers.cookie = [...this.cookies].map(([k, v]) => `${k}=${v}`).join('; ')
    if (spec.method !== 'GET' && this.csrf) headers['x-csrf-token'] = this.csrf
    let body: string | FormData | undefined
    if (spec.json !== undefined) {
      headers['content-type'] = 'application/json'
      body = JSON.stringify(spec.json)
    } else if (spec.form) {
      body = spec.form
    }

    let response: Response
    let text: string
    try {
      response = await this.fetchImpl(this.url(spec), {
        method: spec.method,
        headers,
        body,
        redirect: 'manual',
        signal: AbortSignal.timeout(this.timeoutMs),
      })
      this.remember(response)
      text = await response.text()
    } catch (error) {
      const reason = reasonOf(error)
      this.log(`${request} failed: ${reason}`)
      return err(0, `LeafWiki did not answer: ${reason}`, reason, { request })
    }

    if (response.ok) {
      if (!text) return ok(undefined as T, response.status)
      try {
        return ok(JSON.parse(text) as T, response.status)
      } catch {
        return err(response.status, 'LeafWiki answered with something that is not JSON', text.slice(0, 500), { request })
      }
    }
    this.log(`${request} -> ${response.status}`)
    const location = response.headers.get('location')
    if (response.status >= 300 && response.status < 400 && location) {
      const to = location.split(/[?#]/)[0]
      return err(response.status, `LeafWiki redirected to ${to}; burrow does not follow redirects: set LEAFWIKI_URL to the address the wiki answers on`, '', { request })
    }
    return failureOf(response.status, text, request)
  }

  private remember(response: Response): void {
    for (const line of response.headers.getSetCookie()) {
      const [pair, ...attrs] = line.split(';')
      const at = pair?.indexOf('=') ?? -1
      if (!pair || at <= 0) continue
      const name = pair.slice(0, at).trim()
      const value = pair.slice(at + 1).trim()
      const expired = attrs.some((a) => /^\s*max-age=(0|-)/i.test(a)) || value === ''
      if (expired) this.cookies.delete(name)
      else this.cookies.set(name, value)
    }
    this.csrf = response.headers.get('x-csrf-token') ?? this.cookies.get(CSRF_COOKIE) ?? this.cookies.get(`__Host-${CSRF_COOKIE}`) ?? this.csrf
  }
}

/** undici reports `fetch failed` and keeps the reason (refused, DNS, TLS) in `cause`. */
function reasonOf(error: unknown): string {
  if (!(error instanceof Error)) return String(error)
  const { cause } = error
  // A refused dual-stack connection is an AggregateError with an empty message and only a `code`.
  const why = cause instanceof Error ? cause.message || ('code' in cause ? String(cause.code) : '') : ''
  return why && !error.message.includes(why) ? `${error.message} (${why})` : error.message
}

/** LeafWiki answers `{"error":"text"}`, `{"error":{"code","message"}}` or plain text. */
function failureOf(status: number, text: string, request: string): Err {
  let message = text.trim().split('\n')[0] || `HTTP ${status}`
  let code: string | undefined
  try {
    const body = JSON.parse(text) as { error?: string | { code?: string; message?: string }; message?: string }
    if (typeof body.error === 'string') message = body.error
    else if (body.error?.message) {
      message = body.error.message
      code = body.error.code
    } else if (body.message) message = body.message
  } catch {
    // plain text
  }
  return err(status, message.slice(0, 300), text.slice(0, 2000), { request, ...(code ? { code } : {}) })
}
