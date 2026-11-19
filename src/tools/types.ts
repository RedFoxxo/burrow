import type { z } from 'zod'
import type { CallToolResult } from '@modelcontextprotocol/sdk/types.js'
import type { ToolContext } from './context.js'

/** The name prefix is the permission contract: one approval rule per tier. */
export type Tier = 'read' | 'write' | 'delete'

export const TIERS: readonly Tier[] = ['read', 'write', 'delete']

export interface ToolDef<S extends z.ZodRawShape = z.ZodRawShape> {
  name: string
  description: string
  input: S
  handler: (args: z.output<z.ZodObject<S>>, ctx: ToolContext) => Promise<CallToolResult>
}

/** Keeps the handler's argument type tied to its schema. */
export function defineTool<S extends z.ZodRawShape>(def: ToolDef<S>): ToolDef<S> {
  return def
}

export function tierOf(name: string): Tier | undefined {
  const prefix = name.split('_', 1)[0]
  return TIERS.find((t) => t === prefix)
}
