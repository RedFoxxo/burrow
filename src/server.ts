import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { ListToolsRequestSchema, type Tool } from '@modelcontextprotocol/sdk/types.js'
import { z } from 'zod'
import type { ToolContext } from './tools/context.js'
import { toolsFor, type AnyToolDef } from './tools/index.js'
import { tierOf, type Tier } from './tools/types.js'
import { VERSION } from './version.js'

/** Only the hints that differ from the MCP defaults (not read-only, destructive, not idempotent, open world). */
const ANNOTATIONS: Record<Tier, Tool['annotations']> = {
  read: { readOnlyHint: true, openWorldHint: false },
  write: { destructiveHint: false, openWorldHint: false },
  delete: { destructiveHint: true, openWorldHint: false },
}

/**
 * The JSON schema the model reads, without what costs tokens and says nothing:
 * `$schema`, the safe-integer `maximum` zod adds to every `.int()`, and the
 * `propertyNames: {type: string}` of a record (JSON keys are strings anyway).
 */
function inputSchema(shape: z.ZodRawShape): Tool['inputSchema'] {
  const { $schema: _, ...schema } = z.toJSONSchema(z.object(shape), {
    target: 'draft-07',
    io: 'input',
    override: ({ jsonSchema }) => {
      if (jsonSchema.maximum === Number.MAX_SAFE_INTEGER) delete jsonSchema.maximum
      const names = jsonSchema.propertyNames
      if (typeof names === 'object' && names.type === 'string' && Object.keys(names).length === 1) delete jsonSchema.propertyNames
    },
  })
  return schema as Tool['inputSchema']
}

export function createServer(ctx: ToolContext, tools: AnyToolDef[] = toolsFor(ctx.config.readOnly)): McpServer {
  const server = new McpServer({ name: 'burrow', version: VERSION })
  const listed: Tool[] = []

  for (const tool of tools) {
    const tier = tierOf(tool.name)
    if (!tier) throw new Error(`Tool "${tool.name}" has no tier prefix (read_/write_/delete_)`)
    if (listed.some((t) => t.name === tool.name)) throw new Error(`Tool "${tool.name}" is registered twice`)

    // The SDK validates arguments against `input` and calls the handler.
    server.registerTool(tool.name, { description: tool.description, inputSchema: tool.input }, async (args: Record<string, unknown>) => tool.handler(args, ctx))
    listed.push({ name: tool.name, description: tool.description, inputSchema: inputSchema(tool.input), annotations: ANNOTATIONS[tier] })
  }

  // The SDK's own list adds `$schema`, `execution` and default-valued hints to every tool;
  // this one is built once and says the same in fewer tokens.
  server.server.setRequestHandler(ListToolsRequestSchema, () => ({ tools: listed }))
  return server
}
