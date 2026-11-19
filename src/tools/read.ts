import { z } from 'zod'
import { changedOut, compact, day, findNode, lineDiff, pageOut, plain, revisionOut, treeLines } from '../format.js'
import { failure, success } from './respond.js'
import { pageRef, parentId, parentRef, withPage } from './shared.js'
import { defineTool } from './types.js'

const limit = (max: number, dflt: number) => z.number().int().min(1).max(max).optional().describe(`Default ${dflt}, at most ${max}`)

export const readTools = [
  defineTool({
    name: 'read_wiki',
    description: 'Who is signed in (username, role) and which wiki features are on (revisions, link refactoring, upload size).',
    input: {},
    handler: async (_args, ctx) => {
      const [config, me] = await Promise.all([ctx.wiki.config(), ctx.wiki.me()])
      if (!config.ok) return failure('Could not read the wiki settings', config)
      const c = config.data
      return success(
        compact({
          url: ctx.wiki.session.baseUrl,
          user: me.ok && me.data ? me.data.username : c.authDisabled ? '(no sign-in on this wiki)' : undefined,
          role: me.ok && me.data ? me.data.role : undefined,
          revisions: Boolean(c.enableRevision),
          linkRefactor: Boolean(c.enableLinkRefactor),
          publicAccess: c.publicAccess,
          maxUploadMB: c.maxAssetUploadSizeBytes ? Math.round(c.maxAssetUploadSizeBytes / 1048576) : undefined,
          readOnly: ctx.config.readOnly,
        }),
      )
    },
  }),

  defineTool({
    name: 'read_tree',
    description: 'Page hierarchy as indented lines with path and id, for the whole wiki or below one page.',
    input: {
      under: parentRef.describe('Page id or path to list below; omit for the whole wiki'),
      depth: z.number().int().min(1).max(20).optional().describe('Levels to show (default 3); deeper ones are counted, not listed'),
    },
    handler: async ({ under, depth }, ctx) => {
      const tree = await ctx.wiki.tree()
      if (!tree.ok) return failure('Could not read the page tree', tree)
      let node = tree.data
      if (under) {
        const parent = await parentId(ctx, under)
        if (parent.error) return parent.error
        const hit = parent.id ? findNode(tree.data, parent.id) : undefined
        if (!hit) return failure(`"${under}" is not in the tree`)
        node = hit
      }
      return success(treeLines(node, depth ?? 3))
    },
  }),

  defineTool({
    name: 'read_page',
    description: 'One page: Markdown content, tags, properties, authors and the version needed to change it.',
    input: {
      page: pageRef,
      content: z.boolean().optional().describe('false leaves out the content (default true)'),
    },
    handler: async ({ page, content }, ctx) => withPage(ctx, page, async (p) => success(pageOut(p, content !== false))),
  }),

  defineTool({
    name: 'read_lookup_path',
    description: 'Whether a path exists, segment by segment, and whether it can be created.',
    input: { path: z.string().trim().min(1).describe('Path like docs/runbooks/deploy') },
    handler: async ({ path }, ctx) => {
      const r = await ctx.wiki.lookup(path)
      return r.ok ? success(r.data) : failure(`Could not look up "${path}"`, r)
    },
  }),

  defineTool({
    name: 'read_find_title',
    description: 'Pages whose title matches exactly (case-insensitive), with path and id.',
    input: { title: z.string().trim().min(1) },
    handler: async ({ title }, ctx) => {
      const r = await ctx.wiki.byTitle(title)
      return r.ok ? success(r.data.matches ?? []) : failure(`Could not search titles for "${title}"`, r)
    },
  }),

  defineTool({
    name: 'read_search',
    description: 'Full-text search over page titles, content and tags, best match first, with excerpts.',
    input: {
      query: z.string().trim().min(1),
      offset: z.number().int().min(0).optional(),
      limit: limit(100, 20),
    },
    handler: async ({ query, offset, limit }, ctx) => {
      const r = await ctx.wiki.search(query, offset, limit)
      if (!r.ok) return failure(`Search for "${query}" failed`, r)
      const d = r.data
      return success(
        compact({
          total: d.count,
          offset: d.offset,
          more: d.offset + (d.items?.length ?? 0) < d.count ? true : undefined,
          pages: (d.items ?? []).map((i) => compact({ id: i.page_id, path: i.path, title: plain(i.title), kind: i.kind, tags: i.tags, excerpt: plain(i.excerpt) })),
          tags: d.tag_facets,
        }),
      )
    },
  }),

  defineTool({
    name: 'read_search_status',
    description: 'Whether the search index is still building, and how many pages it holds.',
    input: {},
    handler: async (_args, ctx) => {
      const r = await ctx.wiki.indexing()
      return r.ok ? success(r.data) : failure('Could not read the search index status', r)
    },
  }),

  defineTool({
    name: 'read_tags',
    description: 'Tags in use with how many pages carry each.',
    input: { filter: z.string().optional().describe('Only tags containing this text'), limit: limit(500, 50) },
    handler: async ({ filter, limit }, ctx) => {
      const r = await ctx.wiki.tags(filter, limit)
      return r.ok ? success(r.data ?? []) : failure('Could not list tags', r)
    },
  }),

  defineTool({
    name: 'read_pages_by_tags',
    description: 'Pages carrying all of the given tags.',
    input: { tags: z.array(z.string().trim().min(1)).min(1) },
    handler: async ({ tags }, ctx) => {
      const r = await ctx.wiki.pagesByTags(tags)
      if (!r.ok) return failure('Could not list pages by tag', r)
      return success((r.data ?? []).map((p) => compact({ id: p.id, path: p.path, title: p.title, tags: p.tags, updated: day(p.updatedAt), updatedBy: p.lastAuthor?.username })))
    },
  }),

  defineTool({
    name: 'read_properties',
    description: 'Property keys in use (structured page metadata) with how many pages set each.',
    input: { filter: z.string().optional().describe('Only keys containing this text'), limit: limit(500, 50) },
    handler: async ({ filter, limit }, ctx) => {
      const r = await ctx.wiki.propertyKeys(filter, limit)
      return r.ok ? success(r.data ?? []) : failure('Could not list property keys', r)
    },
  }),

  defineTool({
    name: 'read_pages_by_property',
    description: 'Pages that set a property, optionally to one value.',
    input: { key: z.string().trim().min(1), value: z.string().optional() },
    handler: async ({ key, value }, ctx) => {
      const r = await ctx.wiki.pagesByProperty(key, value)
      if (!r.ok) return failure(`Could not list pages with property "${key}"`, r)
      return success(
        (r.data ?? []).map((p) =>
          compact({
            id: p.id,
            path: p.path,
            title: p.title,
            properties: Object.fromEntries(Object.entries(p.properties ?? {}).map(([k, v]) => [k, v.value])),
            updated: day(p.updatedAt),
          }),
        ),
      )
    },
  }),

  defineTool({
    name: 'read_links',
    description: "A page's links: pages linking to it, pages it links to, and broken links both ways.",
    input: { page: pageRef },
    handler: async ({ page }, ctx) =>
      withPage(ctx, page, async (p) => {
        const r = await ctx.wiki.links(p.id)
        if (!r.ok) return failure(`Could not read the links of "${p.path}"`, r)
        const to = (l: { to_page_id: string; to_path: string; to_page_title: string }) => compact({ id: l.to_page_id, path: l.to_path, title: l.to_page_title })
        const from = (l: { from_page_id: string; from_path?: string; from_page_title?: string }) => compact({ id: l.from_page_id, path: l.from_path, title: l.from_page_title })
        return success(
          compact({
            linkedFrom: (r.data.backlinks ?? []).map(from),
            linksTo: (r.data.outgoings ?? []).map(to),
            brokenOut: (r.data.broken_outgoings ?? []).map((l) => l.to_path),
            brokenIn: (r.data.broken_incoming ?? []).map(from),
          }),
        )
      }),
  }),

  defineTool({
    name: 'read_broken_links',
    description: 'Every broken link in the wiki (admins only).',
    input: {},
    handler: async (_args, ctx) => {
      const r = await ctx.wiki.brokenLinks()
      if (!r.ok) return failure('Could not list broken links', r)
      return success((r.data.links ?? []).map((l) => ({ from: l.from_path, fromId: l.from_page_id, to: l.to_path })))
    },
  }),

  defineTool({
    name: 'read_revisions',
    description: "A page's revision history, newest first (needs revisions on).",
    input: { page: pageRef, limit: limit(100, 20), cursor: z.string().optional().describe('next from the previous answer') },
    handler: async ({ page, limit, cursor }, ctx) =>
      withPage(ctx, page, async (p) => {
        const r = await ctx.wiki.revisions(p.id, limit ?? 20, cursor)
        if (!r.ok) return failure(`Could not read the history of "${p.path}" (are revisions on? see read_wiki)`, r)
        return success(compact({ revisions: (r.data.revisions ?? []).map(revisionOut), next: r.data.nextCursor || undefined }))
      }),
  }),

  defineTool({
    name: 'read_revision',
    description: 'One past revision of a page with its full content.',
    input: { page: pageRef, revision: z.string().trim().min(1).describe('Revision id from read_revisions') },
    handler: async ({ page, revision }, ctx) =>
      withPage(ctx, page, async (p) => {
        const r = await ctx.wiki.revision(p.id, revision)
        if (!r.ok) return failure(`Could not read revision ${revision} of "${p.path}"`, r)
        return success({ ...revisionOut(r.data.revision), content: r.data.content })
      }),
  }),

  defineTool({
    name: 'read_compare_revisions',
    description: 'Line diff between two revisions of a page, or between one revision and the latest.',
    input: {
      page: pageRef,
      base: z.string().trim().min(1).describe('Older revision id'),
      target: z.string().trim().optional().describe('Newer revision id; omit for the latest'),
    },
    handler: async ({ page, base, target }, ctx) =>
      withPage(ctx, page, async (p) => {
        let to = target
        if (!to) {
          const latest = await ctx.wiki.latestRevision(p.id)
          if (!latest.ok) return failure(`Could not find the latest revision of "${p.path}"`, latest)
          to = latest.data.id
        }
        const r = await ctx.wiki.compare(p.id, base, to)
        if (!r.ok) return failure(`Could not compare revisions of "${p.path}"`, r)
        const { base: b, target: t } = r.data
        const diff = r.data.contentChanged ? lineDiff(b.content, t.content) : '(content unchanged)'
        return success(
          compact({
            base: revisionOut(b.revision),
            target: revisionOut(t.revision),
            title: b.revision.title !== t.revision.title ? `${b.revision.title} → ${t.revision.title}` : undefined,
            path: b.revision.path !== t.revision.path ? `${b.revision.path} → ${t.revision.path}` : undefined,
            assetChanges: r.data.assetChanges,
            diff: diff ?? 'Too large to diff here: read both revisions with read_revision.',
          }),
        )
      }),
  }),

  defineTool({
    name: 'read_favorites',
    description: "The signed-in user's favorite pages.",
    input: {},
    handler: async (_args, ctx) => {
      const r = await ctx.wiki.favorites()
      return r.ok ? success((r.data.pages ?? []).map(changedOut)) : failure('Could not list favorites', r)
    },
  }),

  defineTool({
    name: 'read_assets',
    description: 'Files attached to a page, with the URL to embed or link them.',
    input: { page: pageRef },
    handler: async ({ page }, ctx) =>
      withPage(ctx, page, async (p) => {
        const r = await ctx.wiki.assets(p.id)
        if (!r.ok) return failure(`Could not list the files of "${p.path}"`, r)
        return success((r.data.files ?? []).map((f) => ({ name: f.split('/').pop(), markdown: f, url: ctx.wiki.session.absolute(f) })))
      }),
  }),

  defineTool({
    name: 'read_slug',
    description: 'The slug LeafWiki would give a title, unique under its parent (editors only).',
    input: { title: z.string().trim().min(1), parent: parentRef },
    handler: async ({ title, parent }, ctx) => {
      const p = await parentId(ctx, parent)
      if (p.error) return p.error
      const r = await ctx.wiki.slug(title, p.id)
      return r.ok ? success(r.data) : failure(`Could not suggest a slug for "${title}"`, r)
    },
  }),

  defineTool({
    name: 'read_refactor_preview',
    description: 'Which pages link to a page and would be rewritten if it were renamed or moved (needs link refactoring on); changes nothing.',
    input: {
      page: pageRef,
      title: z.string().trim().optional().describe('New title (rename)'),
      slug: z.string().trim().optional().describe('New slug (rename)'),
      parent: parentRef.describe('New parent id or path (move); "/" for the top level'),
    },
    handler: async ({ page, title, slug, parent }, ctx) =>
      withPage(ctx, page, async (p) => {
        const target = await parentId(ctx, parent)
        if (target.error) return target.error
        const moving = parent !== undefined
        const r = await ctx.wiki.refactorPreview(p.id, {
          kind: moving ? 'move' : 'rename',
          title: title ?? p.title,
          slug: slug ?? p.slug,
          ...(moving ? { parentId: target.id ?? 'root' } : {}),
        })
        if (!r.ok) return failure(`Could not preview the change of "${p.path}" (is link refactoring on? see read_wiki)`, r)
        const d = r.data
        return success(compact({ from: d.oldPath, to: d.newPath, links: d.counts.matchedLinks, pages: d.affectedPages.map((a) => compact({ id: a.fromPageId, path: a.fromPath, links: a.matchedPaths, warnings: a.warnings })), warnings: d.warnings }))
      }),
  }),
]
