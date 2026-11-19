import { readFileSync } from 'node:fs'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { InMemoryTransport } from '@modelcontextprotocol/sdk/inMemory.js'
import { describe, expect, test } from 'vitest'
import { ConfigError, loadConfig } from '../src/config.js'
import { lineDiff } from '../src/format.js'
import { createServer } from '../src/server.js'
import { createContext } from '../src/tools/context.js'
import { toolsFor } from '../src/tools/index.js'
import { VERSION } from '../src/version.js'

describe('config', () => {
  const base = { LEAFWIKI_URL: 'https://wiki.example.com/', LEAFWIKI_USERNAME: 'ed', LEAFWIKI_PASSWORD: 'pw' }

  test('keeps the wiki root and a base path, without trailing slashes', () => {
    expect(loadConfig(base)).toEqual({ baseUrl: 'https://wiki.example.com', username: 'ed', password: 'pw', readOnly: false })
    expect(loadConfig({ ...base, LEAFWIKI_URL: 'https://example.com/wiki/' }).baseUrl).toBe('https://example.com/wiki')
    expect(loadConfig({ ...base, LEAFWIKI_URL: 'http://localhost:8080' }).baseUrl).toBe('http://localhost:8080')
  })

  test('refuses plain http off localhost, an /api URL, half the credentials, and an unclear read-only value', () => {
    const problems = (env: Record<string, string>) => {
      try {
        loadConfig(env)
        return ''
      } catch (e) {
        expect(e).toBeInstanceOf(ConfigError)
        return (e as Error).message
      }
    }
    expect(problems({ ...base, LEAFWIKI_URL: 'http://wiki.example.com' })).toMatch(/must be an https URL/)
    expect(problems({ ...base, LEAFWIKI_URL: 'https://wiki.example.com/api' })).toMatch(/without \/api/)
    expect(problems({ LEAFWIKI_URL: base.LEAFWIKI_URL, LEAFWIKI_USERNAME: 'ed' })).toMatch(/Set both/)
    expect(problems({ ...base, BURROW_READ_ONLY: 'maybe' })).toMatch(/BURROW_READ_ONLY/)
    expect(problems({})).toMatch(/LEAFWIKI_URL is required/)
    for (const url of ['https://ed:s3cret@wiki.example.com', 'http://ed:s3cret@wiki.example.com/api']) {
      const message = problems({ ...base, LEAFWIKI_URL: url })
      expect(message).toMatch(/LEAFWIKI_URL must not contain a user name or password/)
      expect(message).not.toContain('s3cret')
    }
    expect(problems({ ...base, LEAFWIKI_PASSWORD: '${LEAFWIKI_PASSWORD}' })).toMatch(/LEAFWIKI_PASSWORD is the unexpanded "\$\{LEAFWIKI_PASSWORD\}"/)
    expect(problems({ ...base, LEAFWIKI_USERNAME: '{env:LEAFWIKI_USERNAME}' })).toMatch(/LEAFWIKI_USERNAME is the unexpanded/)
  })

  test('read-only accepts the usual spellings', () => {
    expect(loadConfig({ ...base, BURROW_READ_ONLY: 'Yes' }).readOnly).toBe(true)
    expect(loadConfig({ ...base, BURROW_READ_ONLY: '0' }).readOnly).toBe(false)
  })
})

describe('lineDiff', () => {
  test('shows changed lines with context and elides the rest', () => {
    const before = ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'].join('\n')
    const after = ['a', 'b', 'c', 'D', 'e', 'f', 'g', 'h', 'i'].join('\n')
    expect(lineDiff(before, after, 1)).toBe(['c', '-d', '+D', 'e', '…', 'h', '+i'].map((l) => (/^[-+]/.test(l) || l === '…' ? l : ` ${l}`)).join('\n'))
    expect(lineDiff('same', 'same')).toBe('(no changes)')
  })
})

describe('server', () => {
  test('version matches package.json; read-only exposes exactly the read_ tools', async () => {
    expect(JSON.parse(readFileSync('package.json', 'utf8')).version).toBe(VERSION)
    const names = toolsFor(false).map((t) => t.name)
    expect(new Set(names).size).toBe(names.length)
    expect(names.every((n) => /^(read|write|delete)_/.test(n))).toBe(true)
    expect(toolsFor(true).map((t) => t.name)).toEqual(names.filter((n) => n.startsWith('read_')))
    createServer(createContext(loadConfig({ LEAFWIKI_URL: 'https://wiki.example.com' })))
  })

  test('lists tools with their hints and lean schemas that still validate the same arguments', async () => {
    const server = createServer(createContext(loadConfig({ LEAFWIKI_URL: 'https://wiki.example.com' })))
    const client = new Client({ name: 'test', version: '1' })
    const [a, b] = InMemoryTransport.createLinkedPair()
    await Promise.all([server.connect(a), client.connect(b)])
    const { tools } = await client.listTools()
    expect(tools.map((t) => t.name)).toEqual(toolsFor(false).map((t) => t.name))
    const byName = Object.fromEntries(tools.map((t) => [t.name, t]))
    expect(byName.read_page?.annotations).toEqual({ readOnlyHint: true, openWorldHint: false })
    expect(byName.delete_page?.annotations).toEqual({ destructiveHint: true, openWorldHint: false })
    expect(byName.write_move?.inputSchema).toMatchObject({ type: 'object', required: ['page', 'parent'], properties: { position: { type: 'integer', minimum: 0 } } })
    const listed = JSON.stringify(tools)
    expect(listed).not.toMatch(/\$schema|9007199254740991|propertyNames|"execution"/)
    const refused = await client.callTool({ name: 'write_move', arguments: { page: 'p', parent: 'q', position: -1 } })
    expect(refused.isError).toBe(true)
  })
})
