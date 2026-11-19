import { z } from 'zod'
import { failure, success } from './respond.js'
import { pageRef, stale, version, withPage } from './shared.js'
import { defineTool } from './types.js'

export const deleteTools = [
  defineTool({
    name: 'delete_page',
    description: 'Delete a page; a page with children is refused unless recursive is true.',
    input: {
      page: pageRef,
      version,
      recursive: z.boolean().optional().describe('Also delete every page below it; only when the user asked for that'),
    },
    handler: async ({ page, version, recursive }, ctx) =>
      withPage(ctx, page, async (p) => {
        const old = stale(p, version)
        if (old) return old
        const r = await ctx.wiki.remove(p.id, p.version, recursive === true)
        return r.ok ? success({ deleted: p.path, id: p.id, recursive: recursive === true }) : failure(`Could not delete "${p.path}"`, r)
      }),
  }),

  defineTool({
    name: 'delete_asset',
    description: 'Delete a file attached to a page.',
    input: { page: pageRef, filename: z.string().trim().min(1) },
    handler: async ({ page, filename }, ctx) =>
      withPage(ctx, page, async (p) => {
        const r = await ctx.wiki.deleteAsset(p.id, filename)
        return r.ok ? success({ deleted: filename, page: p.path }) : failure(`Could not delete "${filename}" from "${p.path}"`, r)
      }),
  }),
]
