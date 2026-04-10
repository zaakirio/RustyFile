import type { ApiError, SearchParams, SearchResponse } from '../lib/types'

export class ApiClientError extends Error {
  status: number
  code: string

  constructor(status: number, code: string, message: string) {
    super(message)
    this.name = 'ApiClientError'
    this.status = status
    this.code = code
  }
}

async function request<T>(
  method: string,
  path: string,
  body?: unknown,
  raw = false,
  signal?: AbortSignal,
): Promise<T> {
  const headers: Record<string, string> = {}
  if (body && !raw) headers['Content-Type'] = 'application/json'
  if (body && raw) headers['Content-Type'] = 'text/plain'

  const res = await fetch(path, {
    method,
    headers,
    signal,
    // Auth is handled entirely via HttpOnly cookie (sent automatically for same-origin).
    body: body ? (raw ? (body as string) : JSON.stringify(body)) : undefined,
  })

  if (!res.ok) {
    if (res.status === 401 && !path.includes('/auth/')) {
      window.dispatchEvent(new Event('rustyfile:auth-expired'))
    }
    const err: ApiError = await res.json().catch(() => ({
      error: res.statusText,
    }))
    throw new ApiClientError(res.status, err.code ?? 'UNKNOWN', err.error)
  }

  if (res.status === 204) return {} as T
  const text = await res.text()
  if (!text) return {} as T
  return JSON.parse(text)
}

export const api = {
  get: <T>(path: string, signal?: AbortSignal) =>
    request<T>('GET', path, undefined, false, signal),
  post: <T>(path: string, body?: unknown, signal?: AbortSignal) =>
    request<T>('POST', path, body, false, signal),
  put: <T>(path: string, body?: unknown, raw = false, signal?: AbortSignal) =>
    request<T>('PUT', path, body, raw, signal),
  patch: <T>(path: string, body?: unknown, signal?: AbortSignal) =>
    request<T>('PATCH', path, body, false, signal),
  delete: <T>(path: string, signal?: AbortSignal) =>
    request<T>('DELETE', path, undefined, false, signal),
  search: (params: SearchParams, signal?: AbortSignal) => {
    const qs = new URLSearchParams()
    qs.set('q', params.q)
    if (params.type) qs.set('type', params.type)
    if (params.min_size !== undefined) qs.set('min_size', String(params.min_size))
    if (params.max_size !== undefined) qs.set('max_size', String(params.max_size))
    if (params.after) qs.set('after', params.after)
    if (params.before) qs.set('before', params.before)
    if (params.path) qs.set('path', params.path)
    if (params.limit !== undefined) qs.set('limit', String(params.limit))
    if (params.offset !== undefined) qs.set('offset', String(params.offset))
    return request<SearchResponse>('GET', `/api/fs/search?${qs.toString()}`, undefined, false, signal)
  },
}
