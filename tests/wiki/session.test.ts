import { setTimeout as sleep } from 'node:timers/promises'
import { afterAll, beforeAll, expect, test } from 'vitest'
import { connect } from './client.js'
import { EDITOR, startLeafwiki, type RunningWiki } from './leafwiki.js'

// A long-running MCP server outlives LeafWiki's 15-minute access token and,
// over days, the refresh token. This wiki shortens both so the real refresh
// call (cookies, CSRF) and the fallback to a new sign-in run for real.
let wiki: RunningWiki

beforeAll(async () => {
  wiki = await startLeafwiki(['--access-token-timeout=3s', '--refresh-token-timeout=6s'])
})
afterAll(() => wiki.stop())

test('keeps working after the access token and then the refresh token run out', async () => {
  const log: string[] = []
  const ed = await connect(EDITOR, { url: wiki.url, log: (l) => log.push(l) })
  try {
    const page = (await ed.must('write_create_page', { title: 'Session' })).json()
    expect(log.filter((l) => l.startsWith('signed in')).length).toBe(1)

    // Access token gone, refresh token valid: one refresh, no new sign-in.
    await sleep(4000)
    await ed.must('read_page', { page: page.id })
    await ed.must('write_append', { page: page.id, text: 'after refresh' })
    expect(log.filter((l) => l.startsWith('signed in')).length).toBe(1)
    expect(log.some((l) => l.startsWith('refresh failed'))).toBe(false)

    // Both gone: the refresh fails and burrow signs in again.
    await sleep(7000)
    await ed.must('write_append', { page: page.id, text: 'after sign-in' })
    expect(log.some((l) => l.startsWith('refresh failed'))).toBe(true)
    expect(log.filter((l) => l.startsWith('signed in')).length).toBe(2)
    expect((await ed.must('read_page', { page: page.id })).json().content).toMatch(/after refresh\n\nafter sign-in$/)
  } finally {
    await ed.close()
  }
})
