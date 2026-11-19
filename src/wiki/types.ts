/** Shapes LeafWiki v0.14 answers with, as observed on a live instance. Only fields burrow reads. */

export type Kind = 'page' | 'section'

export interface UserRef {
  id: string
  username: string
}

export interface NodeMetadata {
  createdAt?: string
  updatedAt?: string
  creatorId?: string
  lastAuthorId?: string
  creator?: UserRef
  lastAuthor?: UserRef
}

export interface TreeNode {
  id: string
  title: string
  slug: string
  path: string
  /** Optimistic-concurrency token: send it back with every change. */
  version: string
  position: number
  kind: Kind
  pinned?: boolean
  children: TreeNode[] | null
  metadata?: NodeMetadata
}

export interface Page extends TreeNode {
  content: string
  tags: string[] | null
  properties: Record<string, string> | null
}

export interface PathLookup {
  path: string
  segments: { slug: string; exists: boolean; id?: string }[]
  exists: boolean
  canCreate?: boolean
}

export interface TitleMatches {
  matches: { id: string; title: string; path: string; kind: Kind }[]
  count: number
}

export interface SearchResult {
  limit: number
  offset: number
  count: number
  items: { page_id: string; title: string; path: string; kind: Kind; rank: number; excerpt: string; tags: string[] | null }[]
  tag_facets: { tag: string; count: number }[] | null
}

export interface IndexingStatus {
  active: boolean
  indexed: number
  failed: number
  finished_at?: string
}

export interface CountedKey {
  count: number
}

export interface TaggedPage {
  id: string
  title: string
  path: string
  excerpt?: string
  tags?: string[] | null
  updatedAt?: string
  lastAuthor?: UserRef
}

export interface PropertyPage {
  id: string
  title: string
  path: string
  properties: Record<string, { value: string; type: string }> | null
  updatedAt?: string
  lastAuthor?: UserRef
}

export interface Link {
  to_page_id: string
  to_page_title: string
  to_path: string
  broken: boolean
  from_page_id: string
  from_page_title?: string
  from_path?: string
}

export interface LinkStatus {
  backlinks: Link[] | null
  broken_incoming: Link[] | null
  outgoings: Link[] | null
  broken_outgoings: Link[] | null
  counts: Record<string, number>
}

export interface Revision {
  id: string
  pageId: string
  type: string
  author?: UserRef
  authorId?: string
  createdAt: string
  title: string
  slug: string
  kind: Kind
  path: string
  summary?: string
}

export interface RevisionList {
  nextCursor: string
  revisions: Revision[]
}

export interface Snapshot {
  revision: Revision
  content: string
  assets: unknown[] | null
}

export interface Comparison {
  base: Snapshot
  target: Snapshot
  contentChanged: boolean
  assetChanges: unknown[] | null
}

export interface RefactorPreview {
  kind: string
  pageId: string
  oldPath: string
  newPath: string
  affectedPages: { fromPageId: string; fromTitle: string; fromPath: string; matchedPaths: string[]; warnings: string[] }[]
  counts: { affectedPages: number; matchedLinks: number }
  warnings: string[]
}

export interface WikiConfig {
  authDisabled?: boolean
  publicAccess?: boolean
  enableRevision?: boolean
  enableLinkRefactor?: boolean
  maxAssetUploadSizeBytes?: number
  basePath?: string
}

export interface Me {
  id: string
  username: string
  email?: string
  role: string
  totpEnabled?: boolean
}
