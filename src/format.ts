import type { Page, Revision, TreeNode } from './wiki/types.js'

/** `2026-11-19T14:54:39Z` → `2026-11-19 14:54`. */
export const day = (iso: string | undefined): string | undefined => (iso ? iso.replace('T', ' ').slice(0, 16) : undefined)

/** Search answers mark hits with `<b>`; the AI gets plain text. */
export function plain(html: string): string {
  return html
    .replace(/<[^>]+>/g, '')
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&amp;/g, '&')
}

/** Drops `undefined`, `null`, empty arrays and empty objects, so answers stay short. */
export function compact<T extends Record<string, unknown>>(value: T): Partial<T> {
  const out: Record<string, unknown> = {}
  for (const [k, v] of Object.entries(value)) {
    if (v === undefined || v === null || v === false) continue
    if (Array.isArray(v) && v.length === 0) continue
    if (typeof v === 'object' && !Array.isArray(v) && Object.keys(v).length === 0) continue
    out[k] = v
  }
  return out as Partial<T>
}

/** A page as the AI sees it: what it needs to read, link to and change it. */
export function pageOut(page: Page, withContent = true) {
  const m = page.metadata ?? {}
  return compact({
    id: page.id,
    path: page.path,
    title: page.title,
    kind: page.kind,
    version: page.version,
    pinned: page.pinned,
    tags: page.tags,
    properties: page.properties,
    updated: day(m.updatedAt),
    updatedBy: m.lastAuthor?.username,
    created: day(m.createdAt),
    createdBy: m.creator?.username,
    content: withContent ? page.content : undefined,
  })
}

/** A page after a change: enough to keep going without reading it again. */
export const changedOut = (page: Page) => compact({ id: page.id, path: page.path, title: page.title, kind: page.kind, version: page.version, pinned: page.pinned })

/**
 * The tree as indented lines, one node each: `- Title (path) [section, pinned] id=…`.
 * `maxDepth` counts levels below `node`; deeper children are summarized.
 */
export function treeLines(node: TreeNode, maxDepth = Infinity): string {
  const lines: string[] = []
  const walk = (n: TreeNode, depth: number) => {
    const flags = [n.kind === 'section' ? 'section' : '', n.pinned ? 'pinned' : ''].filter(Boolean)
    const kids = n.children ?? []
    let line = `${'  '.repeat(depth)}- ${n.title} (${n.path}) ${flags.length ? `[${flags.join(', ')}] ` : ''}id=${n.id}`
    if (depth + 1 > maxDepth && kids.length) line += ` +${kids.length} below`
    lines.push(line)
    if (depth + 1 <= maxDepth) for (const c of kids) walk(c, depth + 1)
  }
  if (node.id === 'root') for (const c of node.children ?? []) walk(c, 0)
  else walk(node, 0)
  return lines.length ? lines.join('\n') : '(empty)'
}

/** Finds a node by id in a tree. */
export function findNode(node: TreeNode, id: string): TreeNode | undefined {
  if (node.id === id) return node
  for (const c of node.children ?? []) {
    const hit = findNode(c, id)
    if (hit) return hit
  }
  return undefined
}

export const revisionOut = (r: Revision) =>
  compact({ id: r.id, at: day(r.createdAt), by: r.author?.username ?? r.authorId, type: r.type, title: r.title, path: r.path, summary: r.summary })

const MAX_DIFF_CELLS = 4_000_000

/**
 * Unified line diff of `before` → `after` with `context` unchanged lines
 * around each change. Null when the texts are too large to compare here.
 */
export function lineDiff(before: string, after: string, context = 2): string | null {
  const a = before.split('\n')
  const b = after.split('\n')
  if (a.length * b.length > MAX_DIFF_CELLS) return null
  // LCS table, from the end, so the walk below goes forward.
  const w = b.length + 1
  const lcs = new Uint32Array((a.length + 1) * w)
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      lcs[i * w + j] = a[i] === b[j] ? lcs[(i + 1) * w + j + 1]! + 1 : Math.max(lcs[(i + 1) * w + j]!, lcs[i * w + j + 1]!)
    }
  }
  const ops: { t: ' ' | '-' | '+'; s: string }[] = []
  let i = 0
  let j = 0
  while (i < a.length || j < b.length) {
    if (i < a.length && j < b.length && a[i] === b[j]) {
      ops.push({ t: ' ', s: a[i]! })
      i++
      j++
    } else if (i < a.length && (j === b.length || lcs[(i + 1) * w + j]! >= lcs[i * w + j + 1]!)) {
      ops.push({ t: '-', s: a[i++]! })
    } else {
      ops.push({ t: '+', s: b[j++]! })
    }
  }
  const keep = ops.map((op, k) => op.t !== ' ' || ops.slice(Math.max(0, k - context), k + context + 1).some((o) => o.t !== ' '))
  const out: string[] = []
  let gap = false
  ops.forEach((op, k) => {
    if (!keep[k]) {
      gap = true
      return
    }
    if (gap && out.length) out.push('…')
    gap = false
    out.push(`${op.t}${op.s}`)
  })
  return out.length ? out.join('\n') : '(no changes)'
}
