import type { TestProject } from 'vitest/node'
import { startLeafwiki } from './leafwiki.js'

declare module 'vitest' {
  export interface ProvidedContext {
    wikiUrl: string
  }
}

export default async function setup(project: TestProject) {
  const wiki = await startLeafwiki()
  project.provide('wikiUrl', wiki.url)
  return () => wiki.stop()
}
