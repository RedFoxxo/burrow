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
 * `/tree` needs a valid access cookie.
 */
function fakeWiki(options: { ttl?: number; totp?: boolean; refreshWorks?: boolean } = {}) {
  let clock = 1_000_000_000_000
  let issued = 0
  const valid = new Map<string, number>()
  const seen: Seen[] = []
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
    const headers = new Headers(init?.headers)
    const cookie = headers.get('cookie') ?? ''
    seen.push({ method: init?.method ?? 'GET', path: url.pathname, cookie, csrf: headers.get('x-csrf-token') ?? '' })
    const token = /leafwiki_at=([^;]+)/.exec(cookie)?.[1]
    switch (url.pathname) {
      case '/api/config':
        return reply(200, { authDisabled: false }, ['leafwiki_csrf=c1; Path=/'], 'c1')
      case '/api/auth/login': {
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
    now: () => clock,
  }
}

const session = (wiki: { fetch: FetchLike; now: () => number }, user = 'ed') =>
  new Session({ baseUrl: 'https://wiki.test', username: user, password: user ? 'pw' : '', fetch: wiki.fetch, now: wiki.now })

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

  test('a network failure is a result, not an exception', async () => {
    const s = new Session({ baseUrl: 'https://wiki.test', username: 'ed', password: 'pw', fetch: async () => Promise.reject(new Error('ECONNREFUSED')) })
    const r = await s.request({ method: 'GET', path: '/tree' })
    expect(r).toMatchObject({ ok: false, status: 0, message: expect.stringMatching(/did not answer: ECONNREFUSED/) })
  })
})
