import { deleteTools } from './delete.js'
import { readTools } from './read.js'
import type { ToolDef } from './types.js'
import { writeTools } from './write.js'

export type AnyToolDef = ToolDef<any>

/** The tools to register: only `read_` ones when `readOnly`. */
export function toolsFor(readOnly: boolean): AnyToolDef[] {
  return readOnly ? [...readTools] : [...readTools, ...writeTools, ...deleteTools]
}
