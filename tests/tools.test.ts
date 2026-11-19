import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { InMemoryTransport } from '@modelcontextprotocol/sdk/inMemory.js'
import { describe, expect, test } from 'vitest'
import { loadConfig } from '../src/config.js'
import type { FetchLike } from '../src/http/session.js'
import { createServer } from '../src/server.js'
import { createContext } from '../src/tools/context.js'

const page = { id: 'p1', title: 'Deploy', slug: 'deploy', path: 'deploy', version: 'v1', position: 0, kind: 'page', children: null, content: '# Deploy\n', tags: null, properties: { owner: 'ed' } }
const tree = { id: 'root', title: '', slug: '', path: '', version: '', position: 0, kind: 'section', children: [{ ...page, content: undefined }] }

/** A wiki without sign-in that knows one page; `sent` keeps every change burrow made. */
function fakeWiki() {
  const sent: { method: string; path: string; body: unknown }[] = []
  const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
  const fetch: FetchLike = async (input, init) => {
    const { pathname } = new URL(input)
    const method = init?.method ?? 'GET'
    if (method !== 'GET') {
      const body = init?.body instanceof FormData ? Buffer.from(await (init.body.get('file') as Blob).arrayBuffer()) : JSON.parse(String(init?.body ?? 'null'))
      sent.push({ method, path: pathname, body })
    }
    switch (`${method} ${pathname}`) {
      case 'GET /api/config':
        return json({ authDisabled: true })
      case 'GET /api/tree':
        return json(tree)
      case 'GET /api/pages/p1':
        return json(page)
      case 'PUT /api/pages/p1':
        return json({ ...page, version: 'v2' })
      case 'POST /api/pages/p1/assets':
        return json({ file: '/assets/p1/a.png' })
      default:
        return new Response('Page not found', { status: 404 })
    }
  }
  return { fetch, sent }
}

async function connect(wiki: { fetch: FetchLike }) {
  const server = createServer(createContext(loadConfig({ LEAFWIKI_URL: 'https://wiki.test' }), { fetch: wiki.fetch }))
  const client = new Client({ name: 'test', version: '1' })
  const [a, b] = InMemoryTransport.createLinkedPair()
  await Promise.all([server.connect(a), client.connect(b)])
  return async (name: string, args: Record<string, unknown> = {}) => {
    const r = (await client.callTool({ name, arguments: args })) as { content: { text: string }[]; isError?: boolean }
    return { text: r.content.map((c) => c.text).join('\n'), isError: Boolean(r.isError) }
  }
}

describe('tools', () => {
  test('read_tree under "/" lists the whole wiki', async () => {
    const call = await connect(fakeWiki())
    expect(await call('read_tree', { under: '/' })).toEqual({ text: '- Deploy (deploy) id=p1', isError: false })
  })

  test('write_properties with nothing to set or remove sends nothing', async () => {
    const wiki = fakeWiki()
    const call = await connect(wiki)
    expect(await call('write_properties', { page: 'p1', set: {} })).toMatchObject({ isError: true, text: expect.stringMatching(/^Not sent/) })
    expect(wiki.sent).toEqual([])
  })

  test('write_upload_asset takes a data URL and refuses what is not base64', async () => {
    const wiki = fakeWiki()
    const call = await connect(wiki)
    const bytes = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0, 255])
    expect((await call('write_upload_asset', { page: 'p1', filename: 'a.png', base64: `data:image/png;base64,${bytes.toString('base64')}` })).isError).toBe(false)
    expect(wiki.sent).toEqual([{ method: 'POST', path: '/api/pages/p1/assets', body: bytes }])

    const bad = await call('write_upload_asset', { page: 'p1', filename: 'a.png', base64: 'this is not base64!' })
    expect(bad).toMatchObject({ isError: true, text: expect.stringMatching(/^Not sent: .*base64/) })
    expect(wiki.sent).toHaveLength(1)
  })
})
