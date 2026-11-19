import type { Config } from '../config.js'
import { Session, type FetchLike } from '../http/session.js'
import { silentLogger, type Logger } from '../log.js'
import { WikiClient } from '../wiki/client.js'

/** Everything a tool handler may use. Built once per server. */
export interface ToolContext {
  config: Config
  wiki: WikiClient
  log: Logger
}

export interface ContextOptions {
  fetch?: FetchLike
  log?: Logger
}

export function createContext(config: Config, options: ContextOptions = {}): ToolContext {
  const log = options.log ?? silentLogger
  const session = new Session({
    baseUrl: config.baseUrl,
    username: config.username,
    password: config.password,
    log,
    ...(options.fetch ? { fetch: options.fetch } : {}),
  })
  return { config, wiki: new WikiClient(session), log }
}
