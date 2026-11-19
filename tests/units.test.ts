import { readFileSync } from 'node:fs'
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
})
