/**
 * Outcome of every request. HTTP failures are values, never exceptions: narrow
 * on `ok`.
 */
export type Result<T> = Ok<T> | Err

export interface Ok<T> {
  ok: true
  status: number
  data: T
}

export interface Err {
  ok: false
  /** HTTP status, or 0 when no response was received (network error, timeout, not signed in). */
  status: number
  /** One line: LeafWiki's own error message when it sent one. */
  message: string
  /** LeafWiki's error code (`page_version_conflict`, `page_has_children`, …), when it sent one. */
  code?: string
  /** Raw response body, or the reason no response was received. */
  body: string
  /** `METHOD path`, for diagnostics. */
  request?: string
}

export function ok<T>(data: T, status = 200): Ok<T> {
  return { ok: true, status, data }
}

export function err(status: number, message: string, body = '', extra: { code?: string; request?: string } = {}): Err {
  const e: Err = { ok: false, status, message, body }
  if (extra.code) e.code = extra.code
  if (extra.request) e.request = extra.request
  return e
}
