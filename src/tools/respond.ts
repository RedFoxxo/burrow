import type { CallToolResult } from '@modelcontextprotocol/sdk/types.js'
import type { Err } from '../http/result.js'

/** Text answers stay as they are; anything else is compact JSON. */
export function success(data: unknown): CallToolResult {
  return { content: [{ type: 'text', text: typeof data === 'string' ? data : JSON.stringify(data) }] }
}

/** What the AI should do next, for the LeafWiki errors that have one answer. */
const NEXT: Record<string, string> = {
  page_version_conflict: 'Someone changed the page after you read it: read it again, check their change, and only then retry with the new version.',
  page_has_children: 'The page has child pages: delete them first, or pass recursive: true only if the user asked to delete everything below it.',
}

/** A failed call: one summary line, LeafWiki's message and status, and the next step when there is one. */
export function failure(summary: string, error?: Err): CallToolResult {
  const lines = [summary]
  if (error) {
    if (error.message && !summary.includes(error.message)) lines.push(`LeafWiki: ${error.message}`)
    if (error.status) lines.push(`status: ${error.status}${error.code ? ` (${error.code})` : ''}`)
    const next = error.code ? NEXT[error.code] : error.status === 403 ? 'The signed-in account may not do this (editors and admins can change pages; some reads are admin-only).' : undefined
    if (next) lines.push(next)
    if (error.request) lines.push(`request: ${error.request}`)
  }
  return { content: [{ type: 'text', text: lines.join('\n') }], isError: true }
}

/** A call rejected before anything was sent. */
export const invalid = (message: string): CallToolResult => failure(`Not sent: ${message}`)
