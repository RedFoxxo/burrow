import { describe, expect, test } from 'vitest'
import { Session, type FetchLike } from '../src/http/session.js'

interface Seen {
  method: string
  path: string
  cookie: string
  csrf: string
}

/**
 * A tiny LeafWiki sign-in: `/config` issues the CSRF cookie, login and refresh
 * issue an access cookie valid for `ttl` seconds of the fake clock, and
 * `/tree` needs a valid access cookie. Cookies are named as on https.
 * `hold(path)` keeps a call to `path` waiting until the returned function runs.
 */
function fakeWiki(options: { ttl?: number; totp?: boolean; refreshWorks?: boolean } = {}) {
  let clock = 1_000_000_000_000
  let issued = 0
  const valid = new Map<string, number>()
  const seen: Seen[] = []
  const held = new Map<string, Promise<void>>()
  const reply = (status: number, body: unknown, cookies: string[] = [], csrf?: string) => {
    const headers = new Headers({ 'content-type': 'application/json' })
    for (const c of cookies) headers.append('set-cookie', c)
    if (csrf) headers.set('x-csrf-token', csrf)
    return new Response(JSON.stringify(body), { status, headers })
  }
  const access = () => {
    const token = `at${++issued}`
    const expires = clock + (options.ttl ?? 900) * 1000
    valid.set(token, expires)
    return { cookie: `leafwiki_at=${token}; Path=/; HttpOnly`, expiresAt: Math.floor(expires / 1000) }
  }
  const fetch: FetchLike = async (input, init) => {
    const url = new URL(input)
    if (init?.redirect !== 'manual') throw new Error('burrow must not follow redirects')
    const headers = new Headers(init?.headers)
    const cookie = headers.get('cookie') ?? ''
    seen.push({ method: init?.method ?? 'GET', path: url.pathname, cookie, csrf: headers.get('x-csrf-token') ?? '' })
    const token = /leafwiki_at=([^;]+)/.exec(cookie)?.[1]
    switch (url.pathname) {
      case '/api/config':
        return reply(200, { authDisabled: false }, ['__Host-leafwiki_csrf=c1; Path=/; Secure'], 'c1')
      case '/api/auth/login': {
        if (JSON.parse(String(init?.body)).password !== 'pw') {
          return reply(401, { error: { code: 'auth_invalid_credentials', message: 'Invalid credentials' } })
        }
        if (options.totp) return reply(200, { requiresTotp: true, loginChallengeToken: 'x' })
        const a = access()
        return reply(200, { accessTokenExpiresAt: a.expiresAt }, [a.cookie, 'leafwiki_rt=rt; Path=/'], 'c1')
      }
      case '/api/auth/refresh-token': {
        if (options.refreshWorks === false) return reply(401, { error: 'expired' })
        const a = access()
        return reply(200, { accessTokenExpiresAt: a.expiresAt }, [a.cookie], 'c1')
      }
      default:
        await held.get(url.pathname)
        if (!token || (valid.get(token) ?? 0) < clock) return reply(401, { error: 'Missing or invalid access token' })
        return reply(200, { ok: true })
    }
  }
  return {
    fetch,
    seen,
    paths: () => seen.map((s) => s.path.replace('/api', '')),
    advance: (seconds: number) => (clock += seconds * 1000),
    expireAll: () => valid.clear(),
    hold: (path: string) => {
      const { promise, resolve } = Promise.withResolvers<void>()
      held.set(`/api${path}`, promise)
      return resolve
    },
    now: () => clock,
  }
}

const session = (wiki: { fetch: FetchLike; now: () => number }, user = 'ed', password = user ? 'pw' : '') =>
  new Session({ baseUrl: 'https://wiki.test', username: user, password, fetch: wiki.fetch, now: wiki.now })

describe('Session', () => {
  test('signs in once, sends cookies, and the CSRF token only on changes', async () => {
    const wiki = fakeWiki()
    const s = session(wiki)
    await Promise.all([s.request({ method: 'GET', path: '/tree' }), s.request({ method: 'GET', path: '/tree' })])
    await s.request({ method: 'PUT', path: '/pages/1', json: {} })
    expect(wiki.paths()).toEqual(['/config', '/auth/login', '/tree', '/tree', '/pages/1'])
    const [, , get, , put] = wiki.seen
    expect(get?.cookie).toMatch(/leafwiki_at=at1/)
    expect(get?.csrf).toBe('')
    expect(put?.csrf).toBe('c1')
  })

  test('refreshes the access token before it runs out', async () => {
    const wiki = fakeWiki({ ttl: 900 })
    const s = session(wiki)
    await s.request({ method: 'GET', path: '/tree' })
    wiki.advance(870)
    const r = await s.request({ method: 'GET', path: '/tree' })
    expect(r.ok).toBe(true)
    expect(wiki.paths()).toEqual(['/config', '/auth/login', '/tree', '/auth/refresh-token', '/tree'])
  })

  test('signs in again and repeats the call once when LeafWiki answers 401', async () => {
    const wiki = fakeWiki({ refreshWorks: false })
    const s = session(wiki)
    await s.request({ method: 'GET', path: '/tree' })
    wiki.expireAll()
    const r = await s.request({ method: 'POST', path: '/pages', json: {} })
    expect(r.ok).toBe(true)
    expect(wiki.paths()).toEqual(['/config', '/auth/login', '/tree', '/pages', '/auth/refresh-token', '/config', '/auth/login', '/pages'])
  })

  test('explains a TOTP account and missing credentials instead of failing on every call', async () => {
    const totp = await session(fakeWiki({ totp: true })).request({ method: 'GET', path: '/tree' })
    expect(totp.ok).toBe(false)
    expect(!totp.ok && totp.message).toMatch(/two-factor codes/)
    const none = await session(fakeWiki(), '').request({ method: 'GET', path: '/tree' })
    expect(!none.ok && none.message).toMatch(/set LEAFWIKI_USERNAME and LEAFWIKI_PASSWORD/)
  })

  test('a network failure is a result naming its cause, not an exception', async () => {
    const refused = new TypeError('fetch failed', { cause: new Error('connect ECONNREFUSED 127.0.0.1:443') })
    const s = new Session({ baseUrl: 'https://wiki.test', username: 'ed', password: 'pw', fetch: async () => Promise.reject(refused) })
    const r = await s.request({ method: 'GET', path: '/tree' })
    expect(r).toMatchObject({ ok: false, status: 0, message: 'LeafWiki did not answer: fetch failed (connect ECONNREFUSED 127.0.0.1:443)' })
  })

  test('a connection lost while reading the answer is a result, not an exception', async () => {
    const broken = () => new Response(new ReadableStream({ pull: (c) => c.error(new Error('socket hang up')) }), { status: 200 })
    const s = new Session({ baseUrl: 'https://wiki.test', username: '', password: '', fetch: async () => broken() })
    const r = await s.request({ method: 'GET', path: '/tree' })
    expect(r).toMatchObject({ ok: false, status: 0, message: 'LeafWiki did not answer: socket hang up' })
  })

  test('stops signing in once LeafWiki rejects the password, so the account is not locked', async () => {
    const wiki = fakeWiki()
    const s = session(wiki, 'ed', 'wrong')
    for (let i = 0; i < 6; i++) {
      const r = await s.request({ method: 'GET', path: '/tree' })
      expect(!r.ok && r.message).toMatch(/Sign-in as "ed" failed: Invalid credentials.*restart/)
    }
    expect(wiki.paths().filter((p) => p === '/auth/login')).toHaveLength(1)
  })

  test('a 401 for a call sent before a sign-in finished repeats it without signing in again', async () => {
    const wiki = fakeWiki()
    const s = session(wiki)
    await s.request({ method: 'GET', path: '/tree' })
    wiki.expireAll()
    const release = wiki.hold('/slow')
    const slow = s.request({ method: 'GET', path: '/slow' })
    expect((await s.request({ method: 'GET', path: '/tree' })).ok).toBe(true)
    release()
    expect((await slow).ok).toBe(true)
    expect(wiki.paths()).toEqual(['/config', '/auth/login', '/tree', '/slow', '/tree', '/auth/refresh-token', '/tree', '/slow'])
  })

  test('never follows a redirect, and says where it pointed', async () => {
    const calls: string[] = []
    const s = new Session({
      baseUrl: 'https://wiki.test',
      username: 'ed',
      password: 'pw',
      fetch: async (input, init) => {
        calls.push(`${init?.redirect} ${input}`)
        return new Response('', { status: 302, headers: { location: 'https://sso.example/login?state=abc' } })
      },
    })
    const r = await s.request({ method: 'GET', path: '/tree' })
    expect(calls).toEqual(['manual https://wiki.test/api/config'])
    expect(r).toMatchObject({ ok: false, status: 302 })
    expect(!r.ok && r.message).toBe('LeafWiki redirected to https://sso.example/login; burrow does not follow redirects: set LEAFWIKI_URL to the address the wiki answers on')
  })

  test('refuses a path that URL parsing would rewrite, before sending anything', async () => {
    const wiki = fakeWiki()
    const r = await session(wiki).request({ method: 'DELETE', path: '/pages/p1/assets/..' })
    expect(r).toMatchObject({ ok: false, status: 0, message: expect.stringMatching(/Not sent/) })
    expect(wiki.seen).toEqual([])
  })
})
