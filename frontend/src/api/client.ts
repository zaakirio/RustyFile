import type { ApiError } from '../lib/types'

// In-memory token for programmatic API calls (TUS uploads, etc.)
// NOT persisted to localStorage. Session survives via HttpOnly cookie.
let token: string | null = null

export function setToken(t: string | null) {
  token = t
  // No localStorage — the HttpOnly cookie handles persistence.
}

export function getToken() {
  return token
}

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
): Promise<T> {
  const headers: Record<string, string> = {}
  if (token) headers['Authorization'] = `Bearer ${token}`
  if (body && !raw) headers['Content-Type'] = 'application/json'
  if (body && raw) headers['Content-Type'] = 'text/plain'

  const res = await fetch(path, {
    method,
    headers,
    body: body ? (raw ? (body as string) : JSON.stringify(body)) : undefined,
  })

  if (!res.ok) {
    // On 401, clear token and redirect to login
    if (res.status === 401 && !path.includes('/auth/')) {
      setToken(null)
      window.location.href = '/login'
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
  get: <T>(path: string) => request<T>('GET', path),
  post: <T>(path: string, body?: unknown) => request<T>('POST', path, body),
  put: <T>(path: string, body?: unknown, raw = false) =>
    request<T>('PUT', path, body, raw),
  patch: <T>(path: string, body?: unknown) => request<T>('PATCH', path, body),
  delete: <T>(path: string) => request<T>('DELETE', path),
}
