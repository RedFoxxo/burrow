#!/usr/bin/env node
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js'
import { ConfigError, loadConfig } from './config.js'
import { stderrLogger } from './log.js'
import { createServer } from './server.js'
import { createContext } from './tools/context.js'

async function main(): Promise<void> {
  let config
  try {
    config = loadConfig()
  } catch (error) {
    if (error instanceof ConfigError) {
      process.stderr.write(`${error.message}\n`)
      process.exit(1)
    }
    throw error
  }

  // Signs in on the first tool call, so a wiki that is briefly down does not stop the client.
  const server = createServer(createContext(config, { log: stderrLogger }))
  await server.connect(new StdioServerTransport())
  process.stderr.write(`burrow: ready on stdio (${config.baseUrl}${config.readOnly ? ', read-only' : ''})\n`)
}

main().catch((error: unknown) => {
  process.stderr.write(`burrow: fatal: ${error instanceof Error ? error.message : String(error)}\n`)
  process.exit(1)
})
