import { afterAll, beforeAll, describe, expect, test } from 'vitest'
import { toolsFor } from '../../src/tools/index.js'
import { connect, type TestClient } from './client.js'
import { EDITOR, VIEWER } from './leafwiki.js'

let ed: TestClient
const touched = new Set<string>()

beforeAll(async () => {
  ed = await connect(EDITOR)
})
afterAll(() => ed.close())

/** Every registered tool must be exercised somewhere in this file. */
const call = async (name: string, args: Record<string, unknown> = {}) => {
  touched.add(name)
  return ed.call(name, args)
}
const must = async (name: string, args: Record<string, unknown> = {}) => {
  touched.add(name)
  return ed.must(name, args)
}

describe('pages', () => {
  test('create, read, update with versions, and refuse a stale version', async () => {
    const created = (await must('write_create_page', { title: 'Runbooks', kind: 'section' })).json()
    expect(created).toMatchObject({ path: 'runbooks', kind: 'section' })

    const deploy = (
      await must('write_create_page', { parent: 'runbooks', title: 'Deploy Guide', content: '# Deploy\n\nStep one.\n', tags: ['ops'], properties: { owner: 'leszek' } })
    ).json()
    expect(deploy.path).toBe('runbooks/deploy-guide')

    const page = (await must('read_page', { page: 'runbooks/deploy-guide' })).json()
    expect(page).toMatchObject({ title: 'Deploy Guide', content: '# Deploy\n\nStep one.\n', tags: ['ops'], properties: { owner: 'leszek' }, updatedBy: 'editor' })
    expect((await must('read_page', { page: page.id, content: false })).json().content).toBeUndefined()

    const updated = (await must('write_update_page', { page: page.id, version: page.version, content: '# Deploy\n\nStep one.\nStep two.\n' })).json()
    expect(updated.version).not.toBe(page.version)
    const after = (await must('read_page', { page: page.id })).json()
    expect(after).toMatchObject({ tags: ['ops'], properties: { owner: 'leszek' }, title: 'Deploy Guide' })

    const stale = await call('write_update_page', { page: page.id, version: page.version, content: 'lost update' })
    expect(stale.isError).toBe(true)
    expect(stale.text).toMatch(/changed since your read/)
    expect((await must('read_page', { page: page.id })).json().content).toContain('Step two.')
  })

  test('edit one passage, append, tags and properties keep the rest', async () => {
    let p = (await must('read_page', { page: 'runbooks/deploy-guide' })).json()
    expect((await call('write_edit_text', { page: p.id, version: p.version, find: 'Step', replace: 'x' })).text).toMatch(/more than once/)
    expect((await call('write_edit_text', { page: p.id, version: p.version, find: 'nope', replace: 'x' })).text).toMatch(/not in/)
    await must('write_edit_text', { page: p.id, version: p.version, find: 'Step two.', replace: 'Step 2: roll out.' })
    await must('write_append', { page: p.id, text: '## Rollback\n\nRevert.' })
    p = (await must('read_page', { page: p.id })).json()
    expect(p.content).toBe('# Deploy\n\nStep one.\nStep 2: roll out.\n\n## Rollback\n\nRevert.')

    expect((await must('write_tags', { page: p.id, add: ['howto', 'ops'], remove: [] })).json().tags.sort()).toEqual(['howto', 'ops'])
    expect((await must('write_tags', { page: p.id, remove: ['ops'] })).json().tags).toEqual(['howto'])
    expect((await must('write_properties', { page: p.id, set: { team: 'core' }, remove: ['owner'] })).json().properties).toEqual({ team: 'core' })
    expect((await must('read_page', { page: p.id })).json().content).toContain('Revert.')
  })

  test('paths: lookup, ensure, find by title, slug, tree', async () => {
    expect((await must('read_lookup_path', { path: 'docs/team/onboarding' })).json()).toMatchObject({ exists: false })
    const ensured = (await must('write_ensure_path', { path: 'docs/team/onboarding', title: 'Onboarding' })).json()
    expect(ensured).toMatchObject({ path: 'docs/team/onboarding', title: 'Onboarding' })
    expect((await must('read_lookup_path', { path: '/docs/team/onboarding/' })).json()).toMatchObject({ exists: true })

    expect((await must('read_find_title', { title: 'deploy guide' })).json()).toEqual([expect.objectContaining({ title: 'Deploy Guide' })])
    expect((await must('read_slug', { title: 'Deploy Guide', parent: 'runbooks' })).json().slug).not.toBe('deploy-guide')

    const whole = (await must('read_tree')).text
    expect(whole).toMatch(/^- Runbooks \(runbooks\) \[section\] id=/m)
    expect(whole).toMatch(/^ {2}- Deploy Guide \(runbooks\/deploy-guide\) id=/m)
    expect((await must('read_tree', { under: 'docs', depth: 1 })).text).toMatch(/^- docs \(docs\) \[section\] id=\S+\n {2}- team \(docs\/team\) \[section\] id=\S+ \+1 below$/)
  })

  test('search, tags, properties, index status', async () => {
    for (let i = 0; i < 50; i++) {
      if (!(await must('read_search_status')).json().active) break
    }
    const hits = (await must('read_search', { query: 'rollback' })).json()
    expect(hits.pages[0]).toMatchObject({ path: 'runbooks/deploy-guide' })
    expect(hits.pages[0].title).toBe('Deploy Guide')
    expect(hits.pages[0].excerpt).not.toMatch(/</)

    expect((await must('read_tags')).json()).toEqual(expect.arrayContaining([{ tag: 'howto', count: 1 }]))
    expect((await must('read_pages_by_tags', { tags: ['howto'] })).json()).toEqual([expect.objectContaining({ path: 'runbooks/deploy-guide' })])
    expect((await must('read_properties')).json()).toEqual(expect.arrayContaining([{ key: 'team', count: 1 }]))
    expect((await must('read_pages_by_property', { key: 'team', value: 'core' })).json()).toEqual([expect.objectContaining({ properties: { team: 'core' } })])
  })

  test('links both ways, and broken links need an admin', async () => {
    const target = (await must('read_page', { page: 'docs/team/onboarding' })).json()
    await must('write_append', { page: target.id, text: 'See [deploy](/runbooks/deploy-guide) and [gone](/nowhere).' })
    const links = (await must('read_links', { page: target.id })).json()
    expect(links.linksTo).toEqual([expect.objectContaining({ path: expect.stringContaining('deploy-guide') })])
    expect(links.brokenOut).toEqual(['/nowhere'])
    expect((await must('read_links', { page: 'runbooks/deploy-guide' })).json().linkedFrom).toEqual([expect.objectContaining({ id: target.id })])
    const broken = await call('read_broken_links')
    expect(broken.isError).toBe(true)
    expect(broken.text).toMatch(/status: 403/)
  })
})

describe('history', () => {
  test('list, read, compare and restore revisions', async () => {
    const p = (await must('read_page', { page: 'runbooks/deploy-guide' })).json()
    const list = (await must('read_revisions', { page: p.id })).json().revisions
    expect(list.length).toBeGreaterThanOrEqual(3)
    const first = list.at(-1)
    expect((await must('read_revision', { page: p.id, revision: first.id })).json().content).toBe('# Deploy Guide\n')

    const diff = (await must('read_compare_revisions', { page: p.id, base: first.id })).json()
    expect(diff.diff).toMatch(/^-# Deploy Guide$/m)
    expect(diff.diff).toMatch(/^\+## Rollback$/m)

    const restored = (await must('write_restore_revision', { page: p.id, revision: first.id })).json()
    expect(restored.version).not.toBe(p.version)
    expect((await must('read_page', { page: p.id })).json().content).toBe('# Deploy Guide\n')
    expect((await must('read_revisions', { page: p.id, limit: 1 })).json()).toMatchObject({ revisions: [expect.anything()], next: expect.any(String) })
  })
})

describe('structure', () => {
  test('move, sort, pin, convert, copy, favorites', async () => {
    const a = (await must('write_create_page', { parent: 'runbooks', title: 'Alpha' })).json()
    const b = (await must('write_create_page', { parent: 'runbooks', title: 'Beta' })).json()

    expect((await must('write_move', { page: a.id, parent: 'docs' })).json().path).toBe('docs/alpha')
    expect((await must('write_move', { page: a.id, parent: '/' })).json().path).toBe('alpha')
    await must('write_move', { page: a.id, parent: 'runbooks', position: 0 })

    await must('write_sort', { parent: 'runbooks', order: [b.id, 'runbooks/deploy-guide', a.id] })
    expect((await must('read_tree', { under: 'runbooks' })).text.split('\n').slice(1).map((l) => l.trim().split(' (')[0])).toEqual(['- Beta', '- Deploy Guide', '- Alpha'])

    expect((await must('write_pin', { page: a.id, pinned: true })).json().pinned).toBe(true)
    expect((await must('write_pin', { page: a.id, pinned: false })).json().pinned).toBeUndefined()

    expect((await must('write_convert', { page: b.id, kind: 'section' })).json().kind).toBe('section')
    expect((await call('write_convert', { page: b.id, kind: 'section' })).text).toMatch(/already a section/)

    const copy = (await must('write_copy', { page: 'runbooks/deploy-guide', parent: 'docs', title: 'Deploy Copy' })).json()
    expect(copy.path).toBe('docs/deploy-copy')

    await must('write_favorite', { page: a.id, favorite: true })
    expect((await must('read_favorites')).json()).toEqual([expect.objectContaining({ id: a.id })])
    await must('write_favorite', { page: a.id, favorite: false })
    expect((await must('read_favorites')).json()).toEqual([])
  })

  test('refactor: preview, then rename rewriting links', async () => {
    const onboarding = (await must('read_page', { page: 'docs/team/onboarding' })).json()
    const p = (await must('read_page', { page: 'runbooks/deploy-guide' })).json()
    const preview = (await must('read_refactor_preview', { page: p.id, slug: 'deploying' })).json()
    expect(preview).toMatchObject({ to: '/runbooks/deploying', pages: [expect.objectContaining({ id: onboarding.id })] })

    expect((await must('write_refactor', { page: p.id, version: p.version, slug: 'deploying' })).json().path).toBe('runbooks/deploying')
    expect((await must('read_page', { page: onboarding.id })).json().content).toContain('/runbooks/deploying')
  })
})

describe('assets', () => {
  test('upload, list, rename, delete', async () => {
    const page = 'runbooks/deploying'
    const up = (await must('write_upload_asset', { page, filename: 'diagram.png', base64: Buffer.from('not really a png').toString('base64') })).json()
    expect(up.markdown).toMatch(/^!\[diagram\.png\]\(\/assets\/.+\/diagram\.png\)$/)
    expect((await must('read_assets', { page })).json()).toEqual([expect.objectContaining({ name: 'diagram.png', url: expect.stringMatching(/^http:\/\/127\.0\.0\.1:\d+\/assets\//) })])
    expect((await must('write_rename_asset', { page, from: 'diagram.png', to: 'flow.png' })).json().file).toMatch(/flow\.png$/)
    await must('delete_asset', { page, filename: 'flow.png' })
    expect((await must('read_assets', { page })).json()).toEqual([])
  })
})

describe('delete', () => {
  test('refuses a stale version and children unless recursive', async () => {
    const docs = (await must('read_page', { page: 'docs' })).json()
    expect((await call('delete_page', { page: 'docs', version: 'old' })).text).toMatch(/changed since your read/)
    const refused = await call('delete_page', { page: 'docs', version: docs.version })
    expect(refused.isError).toBe(true)
    expect(refused.text).toMatch(/page_has_children/)
    expect(refused.text).toMatch(/only if the user asked/)
    expect((await must('delete_page', { page: 'docs', version: docs.version, recursive: true })).json()).toMatchObject({ deleted: 'docs', recursive: true })
    expect((await call('read_page', { page: 'docs/team/onboarding' })).isError).toBe(true)
  })
})

describe('accounts and modes', () => {
  test('read_wiki reports the account and features', async () => {
    expect((await must('read_wiki')).json()).toMatchObject({ user: 'editor', role: 'editor', revisions: true, linkRefactor: true })
  })

  test('a viewer can read but not write', async () => {
    const viewer = await connect(VIEWER)
    try {
      await viewer.must('read_page', { page: 'runbooks/deploying' })
      const denied = await viewer.call('write_append', { page: 'runbooks/deploying', text: 'x' })
      expect(denied.isError).toBe(true)
      expect(denied.text).toMatch(/status: 403/)
    } finally {
      await viewer.close()
    }
  })

  test('read-only registers only read tools', async () => {
    const ro = await connect(EDITOR, { env: { BURROW_READ_ONLY: 'true' } })
    try {
      const names = (await ro.client.listTools()).tools.map((t) => t.name)
      expect(names.length).toBe(toolsFor(true).length)
      expect(names.every((n) => n.startsWith('read_'))).toBe(true)
    } finally {
      await ro.close()
    }
  })

  test('wrong password and missing credentials say what to fix', async () => {
    const bad = await connect({ username: 'editor', password: 'wrong' })
    try {
      expect((await bad.call('read_tree')).text).toMatch(/Sign-in as "editor" failed.*LEAFWIKI_PASSWORD/)
    } finally {
      await bad.close()
    }
    const none = await connect()
    try {
      expect((await none.call('read_tree')).text).toMatch(/set LEAFWIKI_USERNAME and LEAFWIKI_PASSWORD/)
    } finally {
      await none.close()
    }
  })

  test('every tool was exercised against the real wiki', () => {
    const missing = toolsFor(false)
      .map((t) => t.name)
      .filter((n) => !touched.has(n))
    expect(missing).toEqual([])
  })
})
