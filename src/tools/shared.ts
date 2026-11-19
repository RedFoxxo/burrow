import type { CallToolResult } from '@modelcontextprotocol/sdk/types.js'
import { z } from 'zod'
import type { Page } from '../wiki/types.js'
import type { ToolContext } from './context.js'
import { failure } from './respond.js'

export const pageRef = z.string().trim().min(1).describe('Page id or path')
export const parentRef = z.string().trim().optional().describe('Parent id or path; omit for the top level')
export const kind = z.enum(['page', 'section']).describe('section: a folder-like page for children')
export const version = z.string().trim().min(1).describe('version from your last read of the page')

/** Resolves a page reference, or answers with why it could not. */
export async function withPage(ctx: ToolContext, ref: string, then: (page: Page) => Promise<CallToolResult>): Promise<CallToolResult> {
  const page = await ctx.wiki.resolve(ref)
  if (!page.ok) return failure(`Page "${ref}" not found or not readable`, page)
  return then(page.data)
}

/** The id of a parent reference; `undefined` for the top level. */
export async function parentId(ctx: ToolContext, ref: string | undefined): Promise<{ id?: string; error?: CallToolResult }> {
  if (!ref || ref === '/') return {}
  const page = await ctx.wiki.resolve(ref)
  if (!page.ok) return { error: failure(`Parent "${ref}" not found`, page) }
  return { id: page.data.id }
}

/** Checks the caller's version against the page's current one before changing anything. */
export function stale(page: Page, given: string): CallToolResult | undefined {
  if (page.version === given) return undefined
  return failure(
    `Not sent: "${page.path}" changed since your read (your version ${given}, now ${page.version}${page.metadata?.lastAuthor ? ` by ${page.metadata.lastAuthor.username}` : ''}). Read it again, check the change, then retry with the new version.`,
  )
}
