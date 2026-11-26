import { join } from 'node:path'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js'
import { inject } from 'vitest'

/** The burrow executable under test: `BURROW_BIN`, or the release build `npm run test:wiki` makes. */
export const BURROW_BIN = process.env.BURROW_BIN || join(process.cwd(), 'target', 'release', process.platform === 'win32' ? 'burrow.exe' : 'burrow')

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
 * An MCP client connected over stdio to a burrow process signed in to the test
 * wiki as `user`. `url` defaults to the shared test wiki; `log` receives
 * burrow's log lines.
 */
export async function connect(
  user?: { username: string; password: string },
  options: { env?: Record<string, string>; url?: string; log?: (line: string) => void } = {},
): Promise<TestClient> {
  const env = {
    LEAFWIKI_URL: options.url ?? inject('wikiUrl'),
    ...(user ? { LEAFWIKI_USERNAME: user.username, LEAFWIKI_PASSWORD: user.password } : {}),
    ...options.env,
  }
  const client = new Client({ name: 'burrow-test', version: '1' })
  const transport = new StdioClientTransport({ command: BURROW_BIN, env, stderr: 'pipe' })
  let pending = ''
  transport.stderr?.on('data', (chunk: Buffer) => {
    const lines = (pending + chunk.toString()).split('\n')
    pending = lines.pop() ?? ''
    for (const line of lines) options.log?.(line.replace(/^burrow: /, ''))
  })
  await client.connect(transport)
  if (client.getServerVersion()?.name !== 'burrow') throw new Error(`${BURROW_BIN} is not burrow`)
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
