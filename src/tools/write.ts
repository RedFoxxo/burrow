import { z } from 'zod'
import { changedOut } from '../format.js'
import type { ToolContext } from './context.js'
import type { Page } from '../wiki/types.js'
import { failure, invalid, success } from './respond.js'
import { kind, pageRef, parentId, parentRef, stale, version, withPage } from './shared.js'
import { defineTool } from './types.js'

const slug = z
  .string()
  .trim()
  .regex(/^[a-z0-9][a-z0-9-]*$/, 'lowercase letters, digits and dashes')
  .optional()
  .describe('URL segment; omit to derive it from the title')
const tags = z.array(z.string().trim().min(1)).describe('The full tag list (replaces the current one)')
const properties = z.record(z.string(), z.string()).describe('The full property map (replaces the current one)')

/** Saves `page` with the given changes, keeping everything else as it is. */
async function save(ctx: ToolContext, page: Page, changes: { title?: string; slug?: string; content?: string; tags?: string[]; properties?: Record<string, string> }) {
  return ctx.wiki.update(page.id, {
    version: page.version,
    title: changes.title ?? page.title,
    slug: changes.slug ?? page.slug,
    content: changes.content ?? page.content,
    tags: changes.tags ?? page.tags ?? [],
    properties: changes.properties ?? page.properties ?? {},
  })
}

async function suggestSlug(ctx: ToolContext, title: string, parent?: string, current?: string) {
  const r = await ctx.wiki.slug(title, parent, current)
  return r.ok ? { slug: r.data.slug } : { error: failure(`Could not derive a slug from "${title}"`, r) }
}

export const writeTools = [
  defineTool({
    name: 'write_create_page',
    description: 'Create a page (or section) under a parent, optionally with content, tags and properties.',
    input: {
      parent: parentRef,
      title: z.string().trim().min(1),
      slug,
      kind: kind.optional().describe('page (default) or section'),
      content: z.string().optional().describe('Markdown; LeafWiki starts a page with "# Title" when omitted'),
      tags: tags.optional(),
      properties: properties.optional(),
    },
    handler: async (args, ctx) => {
      const parent = await parentId(ctx, args.parent)
      if (parent.error) return parent.error
      let pageSlug = args.slug
      if (!pageSlug) {
        const s = await suggestSlug(ctx, args.title, parent.id)
        if (s.error) return s.error
        pageSlug = s.slug
      }
      const created = await ctx.wiki.create({ ...(parent.id ? { parentId: parent.id } : {}), title: args.title, slug: pageSlug, kind: args.kind ?? 'page' })
      if (!created.ok) return failure(`Could not create "${args.title}"`, created)
      if (args.content === undefined && !args.tags && !args.properties) return success(changedOut(created.data))
      // LeafWiki creates empty pages; content, tags and properties are a second, versioned save.
      const saved = await save(ctx, created.data, { ...(args.content !== undefined ? { content: args.content } : {}), ...(args.tags ? { tags: args.tags } : {}), ...(args.properties ? { properties: args.properties } : {}) })
      if (!saved.ok) return failure(`Created "${created.data.path}" (id ${created.data.id}), but saving its content, tags or properties failed: retry with write_update_page`, saved)
      return success(changedOut(saved.data))
    },
  }),

  defineTool({
    name: 'write_ensure_path',
    description: 'Make sure a path exists, creating every missing segment; the last one gets the title.',
    input: { path: z.string().trim().min(1).describe('Path like docs/runbooks/deploy'), title: z.string().trim().min(1), kind: kind.optional().describe('Kind of the last segment: page (default) or section') },
    handler: async ({ path, title, kind }, ctx) => {
      const r = await ctx.wiki.ensure(path, title, kind ?? 'page')
      return r.ok ? success(changedOut(r.data)) : failure(`Could not create the path "${path}"`, r)
    },
  }),

  defineTool({
    name: 'write_update_page',
    description: "Change a page's title, slug, content, tags or properties; fields you leave out stay as they are.",
    input: {
      page: pageRef,
      version,
      title: z.string().trim().min(1).optional(),
      slug: slug.describe('New URL segment; changes the path and can break links to the page'),
      content: z.string().optional().describe('The full new Markdown content'),
      tags: tags.optional(),
      properties: properties.optional(),
    },
    handler: async ({ page, version, ...changes }, ctx) =>
      withPage(ctx, page, async (p) => {
        const old = stale(p, version)
        if (old) return old
        if (Object.values(changes).every((v) => v === undefined)) return invalid('nothing to change')
        const r = await save(ctx, p, changes)
        return r.ok ? success(changedOut(r.data)) : failure(`Could not update "${p.path}"`, r)
      }),
  }),

  defineTool({
    name: 'write_edit_text',
    description: "Replace one exact passage of a page's content, without sending the whole page.",
    input: {
      page: pageRef,
      version,
      find: z.string().min(1).describe('Exact text to replace, long enough to occur only once'),
      replace: z.string().describe('The new text (empty removes the passage)'),
    },
    handler: async ({ page, version, find, replace }, ctx) =>
      withPage(ctx, page, async (p) => {
        const old = stale(p, version)
        if (old) return old
        const at = p.content.indexOf(find)
        if (at < 0) return invalid(`the text to replace is not in "${p.path}"; read the page again and copy it exactly`)
        if (p.content.indexOf(find, at + 1) >= 0) return invalid(`the text to replace occurs more than once in "${p.path}"; include more of the surrounding text`)
        const r = await save(ctx, p, { content: p.content.slice(0, at) + replace + p.content.slice(at + find.length) })
        return r.ok ? success(changedOut(r.data)) : failure(`Could not edit "${p.path}"`, r)
      }),
  }),

  defineTool({
    name: 'write_append',
    description: 'Add Markdown at the end of a page.',
    input: { page: pageRef, text: z.string().min(1) },
    handler: async ({ page, text }, ctx) =>
      withPage(ctx, page, async (p) => {
        const sep = !p.content || p.content.endsWith('\n\n') ? '' : p.content.endsWith('\n') ? '\n' : '\n\n'
        const r = await save(ctx, p, { content: p.content + sep + text })
        return r.ok ? success(changedOut(r.data)) : failure(`Could not append to "${p.path}"`, r)
      }),
  }),

  defineTool({
    name: 'write_tags',
    description: 'Add or remove tags on a page, keeping the others.',
    input: { page: pageRef, add: z.array(z.string().trim().min(1)).optional(), remove: z.array(z.string().trim().min(1)).optional() },
    handler: async ({ page, add, remove }, ctx) =>
      withPage(ctx, page, async (p) => {
        if (!add?.length && !remove?.length) return invalid('give tags to add or remove')
        const next = [...new Set([...(p.tags ?? []), ...(add ?? [])])].filter((t) => !remove?.includes(t))
        const r = await save(ctx, p, { tags: next })
        return r.ok ? success({ ...changedOut(r.data), tags: r.data.tags }) : failure(`Could not change the tags of "${p.path}"`, r)
      }),
  }),

  defineTool({
    name: 'write_properties',
    description: 'Set or remove properties (structured metadata) on a page, keeping the others.',
    input: { page: pageRef, set: z.record(z.string(), z.string()).optional(), remove: z.array(z.string()).optional() },
    handler: async ({ page, set, remove }, ctx) =>
      withPage(ctx, page, async (p) => {
        if (!set && !remove?.length) return invalid('give properties to set or remove')
        const next = Object.fromEntries(Object.entries({ ...(p.properties ?? {}), ...(set ?? {}) }).filter(([k]) => !remove?.includes(k)))
        const r = await save(ctx, p, { properties: next })
        return r.ok ? success({ ...changedOut(r.data), properties: r.data.properties }) : failure(`Could not change the properties of "${p.path}"`, r)
      }),
  }),

  defineTool({
    name: 'write_move',
    description: 'Move a page (with its children) under another parent; links to it are not rewritten (see write_refactor).',
    input: { page: pageRef, parent: z.string().trim().min(1).describe('New parent id or path; "/" for the top level'), position: z.number().int().min(0).optional().describe('Place among the new siblings, 0 first; default last') },
    handler: async ({ page, parent, position }, ctx) =>
      withPage(ctx, page, async (p) => {
        const target = await parentId(ctx, parent)
        if (target.error) return target.error
        const r = await ctx.wiki.move(p.id, p.version, target.id ?? 'root', position)
        if (!r.ok) return failure(`Could not move "${p.path}"`, r)
        const moved = await ctx.wiki.page(p.id)
        return moved.ok ? success(changedOut(moved.data)) : success({ id: p.id, moved: true })
      }),
  }),

  defineTool({
    name: 'write_sort',
    description: 'Set the order of the children of a page; list every child.',
    input: { parent: z.string().trim().min(1).describe('Parent id or path; "/" for the top level'), order: z.array(z.string().trim().min(1)).min(1).describe('Child ids or paths in the new order') },
    handler: async ({ parent, order }, ctx) => {
      const target = await parentId(ctx, parent)
      if (target.error) return target.error
      const ids: string[] = []
      for (const ref of order) {
        const child = await ctx.wiki.resolve(ref)
        if (!child.ok) return failure(`Child "${ref}" not found`, child)
        ids.push(child.data.id)
      }
      const r = await ctx.wiki.sort(target.id ?? 'root', ids)
      return r.ok ? success({ sorted: ids }) : failure(`Could not sort the children of "${parent}"`, r)
    },
  }),

  defineTool({
    name: 'write_pin',
    description: 'Pin a page to the top of its siblings, or unpin it.',
    input: { page: pageRef, pinned: z.boolean() },
    handler: async ({ page, pinned }, ctx) =>
      withPage(ctx, page, async (p) => {
        const r = await ctx.wiki.pin(p.id, p.version, pinned)
        return r.ok ? success(changedOut(r.data)) : failure(`Could not ${pinned ? 'pin' : 'unpin'} "${p.path}"`, r)
      }),
  }),

  defineTool({
    name: 'write_favorite',
    description: "Add a page to the signed-in user's favorites, or remove it.",
    input: { page: pageRef, favorite: z.boolean() },
    handler: async ({ page, favorite }, ctx) =>
      withPage(ctx, page, async (p) => {
        const r = await ctx.wiki.favorite(p.id, favorite)
        return r.ok ? success({ id: p.id, path: p.path, favorite }) : failure(`Could not change the favorite "${p.path}"`, r)
      }),
  }),

  defineTool({
    name: 'write_convert',
    description: 'Turn a page into a section or a section into a page.',
    input: { page: pageRef, kind },
    handler: async ({ page, kind }, ctx) =>
      withPage(ctx, page, async (p) => {
        if (p.kind === kind) return invalid(`"${p.path}" is already a ${kind}`)
        const r = await ctx.wiki.convert(p.id, p.version, kind)
        if (!r.ok) return failure(`Could not convert "${p.path}" to a ${kind}`, r)
        const after = await ctx.wiki.page(p.id)
        return after.ok ? success(changedOut(after.data)) : success({ id: p.id, kind })
      }),
  }),

  defineTool({
    name: 'write_copy',
    description: 'Copy a page (content, tags, properties, files) under a parent with a new title.',
    input: { page: pageRef, parent: parentRef, title: z.string().trim().min(1), slug },
    handler: async ({ page, parent, title, slug }, ctx) =>
      withPage(ctx, page, async (p) => {
        const target = await parentId(ctx, parent)
        if (target.error) return target.error
        let copySlug = slug
        if (!copySlug) {
          const s = await suggestSlug(ctx, title, target.id)
          if (s.error) return s.error
          copySlug = s.slug
        }
        const r = await ctx.wiki.copy(p.id, target.id, title, copySlug)
        return r.ok ? success(changedOut(r.data)) : failure(`Could not copy "${p.path}"`, r)
      }),
  }),

  defineTool({
    name: 'write_refactor',
    description: 'Rename or move a page and rewrite the links that point to it (needs link refactoring on; preview with read_refactor_preview).',
    input: {
      page: pageRef,
      version,
      title: z.string().trim().optional().describe('New title (rename)'),
      slug: slug.describe('New slug (rename)'),
      parent: parentRef.describe('New parent id or path (move); "/" for the top level'),
      position: z.number().int().min(0).optional(),
      rewriteLinks: z.boolean().optional().describe('Default true'),
    },
    handler: async ({ page, version, title, slug, parent, position, rewriteLinks }, ctx) =>
      withPage(ctx, page, async (p) => {
        const old = stale(p, version)
        if (old) return old
        const target = await parentId(ctx, parent)
        if (target.error) return target.error
        const moving = parent !== undefined
        if (!moving && title === undefined && slug === undefined) return invalid('give a new title, slug or parent')
        const r = await ctx.wiki.refactorApply(p.id, {
          version: p.version,
          kind: moving ? 'move' : 'rename',
          title: title ?? p.title,
          slug: slug ?? p.slug,
          ...(moving ? { parentId: target.id ?? 'root' } : {}),
          rewriteLinks: rewriteLinks !== false,
          ...(position !== undefined ? { position } : {}),
        })
        return r.ok ? success(changedOut(r.data)) : failure(`Could not ${moving ? 'move' : 'rename'} "${p.path}" (is link refactoring on? see read_wiki)`, r)
      }),
  }),

  defineTool({
    name: 'write_restore_revision',
    description: 'Bring a page back to a past revision (the current state stays in the history).',
    input: { page: pageRef, revision: z.string().trim().min(1).describe('Revision id from read_revisions') },
    handler: async ({ page, revision }, ctx) =>
      withPage(ctx, page, async (p) => {
        const r = await ctx.wiki.restore(p.id, revision)
        return r.ok ? success(changedOut(r.data)) : failure(`Could not restore revision ${revision} of "${p.path}"`, r)
      }),
  }),

  defineTool({
    name: 'write_upload_asset',
    description: 'Attach a file to a page from base64 data; answers with the Markdown link to embed it.',
    input: {
      page: pageRef,
      filename: z.string().trim().min(1).regex(/^[^/\\]+$/, 'a file name, not a path'),
      base64: z.string().min(1).describe('The file content, base64-encoded; burrow never reads local files'),
    },
    handler: async ({ page, filename, base64 }, ctx) =>
      withPage(ctx, page, async (p) => {
        const bytes = Buffer.from(base64, 'base64')
        if (!bytes.length) return invalid('the base64 data is empty or not base64')
        const r = await ctx.wiki.upload(p.id, filename, bytes)
        if (!r.ok) return failure(`Could not attach "${filename}" to "${p.path}"`, r)
        const embed = /\.(png|jpe?g|gif|webp|svg)$/i.test(filename) ? `![${filename}](${r.data.file})` : `[${filename}](${r.data.file})`
        return success({ file: r.data.file, url: ctx.wiki.session.absolute(r.data.file), markdown: embed })
      }),
  }),

  defineTool({
    name: 'write_rename_asset',
    description: 'Rename a file attached to a page.',
    input: { page: pageRef, from: z.string().trim().min(1), to: z.string().trim().min(1).regex(/^[^/\\]+$/, 'a file name, not a path') },
    handler: async ({ page, from, to }, ctx) =>
      withPage(ctx, page, async (p) => {
        const r = await ctx.wiki.renameAsset(p.id, from, to)
        return r.ok ? success({ file: r.data.url, url: ctx.wiki.session.absolute(r.data.url), note: 'Links to the old name are not updated.' }) : failure(`Could not rename "${from}" on "${p.path}"`, r)
      }),
  }),
]
