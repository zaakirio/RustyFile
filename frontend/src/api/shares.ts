import { api, ApiClientError } from './client'
import type {
  ApiError,
  CreateShareRequest,
  PublicShareMeta,
  Share,
} from '../lib/types'

// ── Authenticated management API ───────────────────────────────────────────

export function createShare(req: CreateShareRequest): Promise<Share> {
  return api.post<Share>('/api/shares', req)
}

export async function listShares(): Promise<Share[]> {
  const res = await api.get<{ shares: Share[] }>('/api/shares')
  return res.shares
}

export function deleteShare(token: string): Promise<unknown> {
  return api.delete(`/api/shares/${encodeURIComponent(token)}`)
}

/** Full URL recipients open in their browser. */
export function shareUrl(token: string): string {
  return `${window.location.origin}/share/${token}`
}

// ── Public (anonymous) API ─────────────────────────────────────────────────

async function publicRequest<T>(
  method: string,
  path: string,
  opts: { password?: string; body?: unknown } = {},
): Promise<T> {
  const headers: Record<string, string> = {}
  if (opts.password) headers['X-Share-Password'] = opts.password
  if (opts.body !== undefined) headers['Content-Type'] = 'application/json'

  const res = await fetch(path, {
    method,
    headers,
    body: opts.body !== undefined ? JSON.stringify(opts.body) : undefined,
  })

  if (!res.ok) {
    const err: ApiError = await res.json().catch(() => ({ error: res.statusText }))
    throw new ApiClientError(res.status, err.code ?? 'UNKNOWN', err.error)
  }

  return res.json()
}

/**
 * Fetches share metadata; pass the password (if known) to unlock the full
 * metadata of a protected share.
 */
export function getPublicShareMeta(
  token: string,
  password?: string,
): Promise<PublicShareMeta> {
  return publicRequest<PublicShareMeta>(
    'GET',
    `/api/public/shares/${encodeURIComponent(token)}`,
    { password },
  )
}

/**
 * Exchanges the share password for a short-lived signed download token.
 * Browsers cannot attach headers to a download navigation, so the token is
 * appended to the download URL as `?t=` instead of the password itself.
 */
export async function verifySharePassword(
  token: string,
  password: string,
): Promise<string> {
  const res = await publicRequest<{ download_token: string }>(
    'POST',
    `/api/public/shares/${encodeURIComponent(token)}/verify`,
    { body: { password } },
  )
  return res.download_token
}

export function publicShareDownloadUrl(token: string, downloadToken?: string): string {
  const base = `/api/public/shares/${encodeURIComponent(token)}/download`
  return downloadToken ? `${base}?t=${encodeURIComponent(downloadToken)}` : base
}

export interface DroppedFile {
  name: string
  size: number
}

/**
 * Uploads one file into a drop share via XHR (fetch has no upload progress).
 */
export function uploadToShare(
  token: string,
  file: File,
  opts: { password?: string; onProgress?: (percent: number) => void } = {},
): Promise<DroppedFile[]> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest()
    xhr.open('POST', `/api/public/shares/${encodeURIComponent(token)}/upload`)
    if (opts.password) xhr.setRequestHeader('X-Share-Password', opts.password)

    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable && opts.onProgress) {
        opts.onProgress(Math.round((e.loaded / e.total) * 100))
      }
    }

    xhr.onload = () => {
      if (xhr.status >= 200 && xhr.status < 300) {
        try {
          const body = JSON.parse(xhr.responseText) as { files: DroppedFile[] }
          resolve(body.files)
        } catch {
          resolve([])
        }
      } else {
        let message = `Upload failed (${xhr.status})`
        try {
          const err = JSON.parse(xhr.responseText) as ApiError
          if (err.error) message = err.error
        } catch {
          // keep default message
        }
        reject(new ApiClientError(xhr.status, 'UPLOAD_FAILED', message))
      }
    }
    xhr.onerror = () => reject(new ApiClientError(0, 'NETWORK', 'Network error during upload'))

    const form = new FormData()
    form.append('file', file, file.name)
    xhr.send(form)
  })
}
