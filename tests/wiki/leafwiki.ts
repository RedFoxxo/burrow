import { spawn, type ChildProcess } from 'node:child_process'
import { createHash } from 'node:crypto'
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { setTimeout as sleep } from 'node:timers/promises'

/** The LeafWiki release burrow is tested against. Moving it is a deliberate change. */
export const LEAFWIKI_VERSION = 'v0.14.2'

const ADMIN = { username: 'admin', password: 'admin-pass-123' }
export const EDITOR = { username: 'editor', password: 'editor-pass-123' }
export const VIEWER = { username: 'viewer', password: 'viewer-pass-123' }

function asset(): string {
  const os = { linux: 'linux', darwin: 'darwin', win32: 'windows' }[process.platform as string]
  const arch = { x64: 'amd64', arm64: 'arm64' }[process.arch as string]
  if (!os || !arch) throw new Error(`No LeafWiki ${LEAFWIKI_VERSION} build for ${process.platform}/${process.arch}`)
  return `leafwiki-${LEAFWIKI_VERSION}-${os}-${arch}${os === 'windows' ? '.exe' : ''}`
}

/** The pinned binary in `.cache/`, downloaded once and checked against the release's sha256. */
export async function leafwikiBinary(): Promise<string> {
  const name = asset()
  const dir = join(process.cwd(), '.cache', 'leafwiki')
  const file = join(dir, name)
  if (existsSync(file)) return file
  mkdirSync(dir, { recursive: true })
  const base = `https://github.com/perber/leafwiki/releases/download/${LEAFWIKI_VERSION}/${name}`
  const [bin, sum] = await Promise.all([fetch(base), fetch(`${base}.sha256`)])
  if (!bin.ok || !sum.ok) throw new Error(`Download of ${base} failed: ${bin.status}/${sum.status}`)
  const bytes = Buffer.from(await bin.arrayBuffer())
  const expected = (await sum.text()).trim().split(/\s+/)[0]
  const actual = createHash('sha256').update(bytes).digest('hex')
  if (actual !== expected) throw new Error(`${name}: sha256 ${actual}, release says ${expected}`)
  writeFileSync(file, bytes)
  chmodSync(file, 0o755)
  return file
}

function freePort(): Promise<number> {
  const { promise, resolve, reject } = Promise.withResolvers<number>()
  const srv = createServer()
  srv.listen(0, '127.0.0.1', () => {
    const address = srv.address()
    srv.close(() => (typeof address === 'object' && address ? resolve(address.port) : reject(new Error('no port'))))
  })
  return promise
}

export interface RunningWiki {
  url: string
  stop: () => void
}

/** Starts LeafWiki with throwaway data, revisions and link refactoring on, and adds an editor and a viewer. `extraArgs` go to LeafWiki. */
export async function startLeafwiki(extraArgs: string[] = []): Promise<RunningWiki> {
  const bin = await leafwikiBinary()
  const port = await freePort()
  const data = mkdtempSync(join(tmpdir(), 'burrow-leafwiki-'))
  const child: ChildProcess = spawn(
    bin,
    [
      `--port=${port}`,
      '--host=127.0.0.1',
      `--data-dir=${data}`,
      '--jwt-secret=burrow-test-secret-burrow-test-secret',
      `--admin-password=${ADMIN.password}`,
      '--allow-insecure=true',
      '--enable-revision',
      '--enable-link-refactor',
      '--revision-coalesce-window=0',
      ...extraArgs,
    ],
    { stdio: ['ignore', 'ignore', 'pipe'] },
  )
  let log = ''
  child.stderr?.on('data', (d) => (log = (log + d).slice(-4000)))
  const url = `http://127.0.0.1:${port}`
  const stop = () => {
    child.kill()
    rmSync(data, { recursive: true, force: true })
  }

  for (let i = 0; ; i++) {
    try {
      if ((await fetch(`${url}/api/health`)).ok) break
    } catch {
      // not up yet
    }
    if (i > 100 || child.exitCode !== null) {
      stop()
      throw new Error(`LeafWiki did not start:\n${log}`)
    }
    await sleep(100)
  }

  await addUsers(url)
  return { url, stop }
}

/** Signs in as the admin the way a browser does and creates the test accounts. */
async function addUsers(url: string): Promise<void> {
  const login = await fetch(`${url}/api/auth/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ identifier: ADMIN.username, password: ADMIN.password }),
  })
  if (!login.ok) throw new Error(`admin sign-in failed: ${login.status} ${await login.text()}`)
  const cookie = login.headers
    .getSetCookie()
    .map((c) => c.split(';')[0])
    .join('; ')
  const csrf = login.headers.get('x-csrf-token') ?? ''
  for (const [user, role] of [
    [EDITOR, 'editor'],
    [VIEWER, 'viewer'],
  ] as const) {
    const r = await fetch(`${url}/api/users`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', cookie, 'x-csrf-token': csrf },
      body: JSON.stringify({ username: user.username, email: `${user.username}@example.com`, password: user.password, role }),
    })
    if (!r.ok) throw new Error(`creating ${user.username} failed: ${r.status} ${await r.text()}`)
  }
}

export const readText = (file: string) => readFileSync(file, 'utf8')
