import type { Result } from '../http/result.js'
import type { Session } from '../http/session.js'
import type {
  Comparison,
  CountedKey,
  IndexingStatus,
  Kind,
  LinkStatus,
  Me,
  Page,
  PathLookup,
  PropertyPage,
  RefactorPreview,
  Revision,
  RevisionList,
  SearchResult,
  Snapshot,
  TaggedPage,
  TitleMatches,
  TreeNode,
  WikiConfig,
} from './types.js'

const enc = encodeURIComponent

/** Page paths are stored without leading or trailing slashes. */
export const normalizePath = (path: string): string => path.trim().replace(/^\/+|\/+$/g, '')

export interface PageUpdate {
  version: string
  title: string
  slug: string
  content?: string
  tags?: string[]
  properties?: Record<string, string>
}

export interface RefactorInput {
  kind: 'rename' | 'move'
  title?: string
  slug?: string
  content?: string
  parentId?: string
}

/** One method per LeafWiki route burrow uses (v0.14). */
export class WikiClient {
  constructor(readonly session: Session) {}

  config = () => this.session.request<WikiConfig>({ method: 'GET', path: '/config' })
  me = () => this.session.request<Me>({ method: 'GET', path: '/auth/me' })

  tree = (depth?: number) => this.session.request<TreeNode>({ method: 'GET', path: '/tree', query: { depth } })
  page = (id: string) => this.session.request<Page>({ method: 'GET', path: `/pages/${enc(id)}` })
  pageByPath = (path: string) => this.session.request<Page>({ method: 'GET', path: '/pages/by-path', query: { path: normalizePath(path) } })
  lookup = (path: string) => this.session.request<PathLookup>({ method: 'GET', path: '/pages/lookup', query: { path: normalizePath(path) } })
  byTitle = (title: string) => this.session.request<TitleMatches>({ method: 'GET', path: '/pages/by-title', query: { title } })
  slug = (title: string, parentId?: string, currentId?: string) =>
    this.session.request<{ slug: string }>({ method: 'GET', path: '/pages/slug-suggestion', query: { title, parentId, currentId } })

  /**
   * A page by id or by path. A value with a `/` is a path; otherwise the id is
   * tried first and the path second, since top-level slugs look like ids.
   */
  async resolve(ref: string): Promise<Result<Page>> {
    const value = ref.trim()
    if (value.includes('/')) return this.pageByPath(value)
    const byId = await this.page(value)
    if (byId.ok || byId.status !== 404) return byId
    const byPath = await this.pageByPath(value)
    return byPath.ok ? byPath : { ...byPath, message: `No page has the id or path "${value}"` }
  }

  search = (q: string, offset?: number, limit?: number) =>
    this.session.request<SearchResult>({ method: 'GET', path: '/search', query: { q, offset, limit } })
  indexing = () => this.session.request<IndexingStatus>({ method: 'GET', path: '/search/status' })
  tags = (q?: string, limit?: number) => this.session.request<(CountedKey & { tag: string })[]>({ method: 'GET', path: '/tags', query: { q, limit } })
  pagesByTags = (tags: string[]) => this.session.request<TaggedPage[] | null>({ method: 'GET', path: '/tags/pages', query: { tags } })
  propertyKeys = (q?: string, limit?: number) =>
    this.session.request<(CountedKey & { key: string })[]>({ method: 'GET', path: '/properties', query: { q, limit } })
  pagesByProperty = (key: string, value?: string) =>
    this.session.request<PropertyPage[] | null>({ method: 'GET', path: '/properties/pages', query: { key, value } })
  links = (id: string) => this.session.request<LinkStatus>({ method: 'GET', path: `/pages/${enc(id)}/links` })
  brokenLinks = () =>
    this.session.request<{ links: { from_page_id: string; from_title: string; from_path: string; to_path: string }[] | null }>({
      method: 'GET',
      path: '/links/broken',
    })

  revisions = (id: string, limit?: number, cursor?: string) =>
    this.session.request<RevisionList>({ method: 'GET', path: `/pages/${enc(id)}/revisions`, query: { limit, cursor } })
  latestRevision = (id: string) => this.session.request<Revision>({ method: 'GET', path: `/pages/${enc(id)}/revisions/latest` })
  revision = (id: string, revisionId: string) =>
    this.session.request<Snapshot>({ method: 'GET', path: `/pages/${enc(id)}/revisions/${enc(revisionId)}` })
  compare = (id: string, base: string, target: string) =>
    this.session.request<Comparison>({ method: 'GET', path: `/pages/${enc(id)}/revisions/compare`, query: { base, target } })
  restore = (id: string, revisionId: string) =>
    this.session.request<Page>({ method: 'POST', path: `/pages/${enc(id)}/revisions/${enc(revisionId)}/restore` })

  favorites = () => this.session.request<{ pages: Page[] | null }>({ method: 'GET', path: '/favorites' })
  favorite = (id: string, on: boolean) =>
    this.session.request<{ message: string }>({ method: on ? 'PUT' : 'DELETE', path: `/pages/${enc(id)}/favorite` })

  create = (input: { parentId?: string; title: string; slug: string; kind: Kind }) =>
    this.session.request<Page>({ method: 'POST', path: '/pages', json: input })
  ensure = (path: string, title: string, kind: Kind) =>
    this.session.request<Page>({ method: 'POST', path: '/pages/ensure', json: { path: normalizePath(path), title, kind } })
  update = (id: string, input: PageUpdate) => this.session.request<Page>({ method: 'PUT', path: `/pages/${enc(id)}`, json: input })
  remove = (id: string, version: string, recursive: boolean) =>
    this.session.request<{ message: string }>({ method: 'DELETE', path: `/pages/${enc(id)}`, query: { version, recursive } })
  move = (id: string, version: string, parentId: string, position?: number) =>
    this.session.request<{ message: string }>({ method: 'PUT', path: `/pages/${enc(id)}/move`, json: { version, parentId, position } })
  sort = (parentId: string, orderedIds: string[]) =>
    this.session.request<{ message: string }>({ method: 'PUT', path: `/pages/${enc(parentId)}/sort`, json: { orderedIds } })
  pin = (id: string, version: string, pinned: boolean) =>
    this.session.request<Page>({ method: 'PUT', path: `/pages/${enc(id)}/pin`, json: { version, pinned } })
  convert = (id: string, version: string, kind: Kind) =>
    this.session.request<undefined>({ method: 'POST', path: `/pages/convert/${enc(id)}`, json: { targetKind: kind, version } })
  copy = (id: string, targetParentId: string | undefined, title: string, slug: string) =>
    this.session.request<Page>({ method: 'POST', path: `/pages/copy/${enc(id)}`, json: { targetParentId: targetParentId ?? null, title, slug } })
  refactorPreview = (id: string, input: RefactorInput) =>
    this.session.request<RefactorPreview>({ method: 'POST', path: `/pages/${enc(id)}/refactor/preview`, json: input })
  refactorApply = (id: string, input: RefactorInput & { version: string; rewriteLinks: boolean; position?: number }) =>
    this.session.request<Page>({ method: 'POST', path: `/pages/${enc(id)}/refactor/apply`, json: input })

  assets = (id: string) => this.session.request<{ files: string[] | null }>({ method: 'GET', path: `/pages/${enc(id)}/assets` })
  upload = (id: string, filename: string, bytes: Uint8Array) => {
    const form = new FormData()
    form.append('file', new Blob([bytes]), filename)
    return this.session.request<{ file: string }>({ method: 'POST', path: `/pages/${enc(id)}/assets`, form })
  }
  renameAsset = (id: string, from: string, to: string) =>
    this.session.request<{ url: string }>({ method: 'PUT', path: `/pages/${enc(id)}/assets/rename`, json: { old_filename: from, new_filename: to } })
  deleteAsset = (id: string, filename: string) =>
    this.session.request<{ message: string }>({ method: 'DELETE', path: `/pages/${enc(id)}/assets/${enc(filename)}` })
}
