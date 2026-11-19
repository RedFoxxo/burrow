import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { InMemoryTransport } from '@modelcontextprotocol/sdk/inMemory.js'
import { inject } from 'vitest'
import { loadConfig } from '../../src/config.js'
import { createServer } from '../../src/server.js'
import { createContext } from '../../src/tools/context.js'

export interface Answer {
  text: string
  isError: boolean
  json: () => any
}

export interface TestClient {
  client: Client
  call: (name: string, args?: Record<string, unknown>) => Promise<Answer>
  /** Like `call`, but fails the test when the tool reports an error. */
  must: (name: string, args?: Record<string, unknown>) => Promise<Answer>
  close: () => Promise<void>
}

/**
 * An MCP client connected to a burrow server signed in to the test wiki as
 * `user`. `url` defaults to the shared test wiki; `log` receives burrow's log lines.
 */
export async function connect(
  user?: { username: string; password: string },
  options: { env?: Record<string, string>; url?: string; log?: (line: string) => void } = {},
): Promise<TestClient> {
  const config = loadConfig({
    LEAFWIKI_URL: options.url ?? inject('wikiUrl'),
    ...(user ? { LEAFWIKI_USERNAME: user.username, LEAFWIKI_PASSWORD: user.password } : {}),
    ...options.env,
  })
  const server = createServer(createContext(config, options.log ? { log: options.log } : {}))
  const client = new Client({ name: 'burrow-test', version: '1' })
  const [a, b] = InMemoryTransport.createLinkedPair()
  await Promise.all([server.connect(a), client.connect(b)])
  const call = async (name: string, args: Record<string, unknown> = {}): Promise<Answer> => {
    const r = (await client.callTool({ name, arguments: args })) as { content: { type: string; text: string }[]; isError?: boolean }
    const text = r.content.map((c) => c.text).join('\n')
    return { text, isError: Boolean(r.isError), json: () => JSON.parse(text) }
  }
  const must = async (name: string, args: Record<string, unknown> = {}) => {
    const r = await call(name, args)
    if (r.isError) throw new Error(`${name} failed:\n${r.text}`)
    return r
  }
  return { client, call, must, close: () => client.close() }
}
