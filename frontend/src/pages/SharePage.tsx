import { useCallback, useEffect, useRef, useState } from 'react'
import { useParams } from 'react-router'
import {
  Check,
  Download,
  Folder,
  Lock,
  Page,
  Upload,
  WarningTriangle,
} from 'iconoir-react'
import type { DragEvent, FormEvent } from 'react'
import {
  getPublicShareMeta,
  publicShareDownloadUrl,
  uploadToShare,
  verifySharePassword,
} from '../api/shares'
import { ApiClientError } from '../api/client'
import { formatSize } from '../lib/format'
import type { PublicShareMeta } from '../lib/types'

interface DropItem {
  id: number
  name: string
  progress: number
  status: 'uploading' | 'done' | 'error'
  /** Final (possibly deduped) name on the server, or error message. */
  detail?: string
}

/**
 * Public landing page for a share link. Works logged-out: it only talks to
 * the anonymous /api/public/shares endpoints.
 */
export default function SharePage() {
  const { token = '' } = useParams()

  const [meta, setMeta] = useState<PublicShareMeta | null>(null)
  const [notFound, setNotFound] = useState(false)
  const [loading, setLoading] = useState(true)

  // Password state: kept in memory for the session so uploads and download
  // token refreshes can re-use it.
  const [password, setPassword] = useState('')
  const [unlockedPassword, setUnlockedPassword] = useState<string | null>(null)
  const [passwordError, setPasswordError] = useState<string | null>(null)
  const [unlocking, setUnlocking] = useState(false)

  const [downloadError, setDownloadError] = useState<string | null>(null)

  // Drop upload state
  const [items, setItems] = useState<DropItem[]>([])
  const [dragOver, setDragOver] = useState(false)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const nextId = useRef(0)

  const fetchMeta = useCallback(
    async (pw?: string) => {
      try {
        const res = await getPublicShareMeta(token, pw)
        setMeta(res)
        setNotFound(false)
        return res
      } catch (err) {
        if (err instanceof ApiClientError && err.status === 404) {
          setNotFound(true)
        } else if (err instanceof ApiClientError && err.status === 401) {
          throw err
        } else {
          setNotFound(true)
        }
        return null
      }
    },
    [token],
  )

  useEffect(() => {
    setLoading(true)
    fetchMeta()
      .catch(() => setNotFound(true))
      .finally(() => setLoading(false))
  }, [fetchMeta])

  const handleUnlock = useCallback(
    async (e: FormEvent) => {
      e.preventDefault()
      if (!password) return
      setPasswordError(null)
      setUnlocking(true)
      try {
        // Verify first (clean 401 on a wrong password), then re-fetch the
        // full metadata with the now-known-good password.
        await verifySharePassword(token, password)
        await fetchMeta(password)
        setUnlockedPassword(password)
      } catch (err) {
        setPasswordError(
          err instanceof ApiClientError && err.status === 401
            ? 'WRONG PASSWORD'
            : 'UNLOCK FAILED',
        )
      } finally {
        setUnlocking(false)
      }
    },
    [token, password, fetchMeta],
  )

  const handleDownload = useCallback(async () => {
    setDownloadError(null)
    try {
      let url = publicShareDownloadUrl(token)
      if (unlockedPassword) {
        // Browsers cannot attach headers to a download navigation, so
        // exchange the password for a fresh short-lived token each time.
        const downloadToken = await verifySharePassword(token, unlockedPassword)
        url = publicShareDownloadUrl(token, downloadToken)
      }
      window.location.assign(url)
    } catch {
      setDownloadError('DOWNLOAD FAILED — TRY AGAIN')
    }
  }, [token, unlockedPassword])

  const uploadFiles = useCallback(
    (files: FileList | File[]) => {
      for (const file of Array.from(files)) {
        const id = nextId.current++
        setItems((prev) => [
          ...prev,
          { id, name: file.name, progress: 0, status: 'uploading' },
        ])

        uploadToShare(token, file, {
          password: unlockedPassword ?? undefined,
          onProgress: (percent) =>
            setItems((prev) =>
              prev.map((it) => (it.id === id ? { ...it, progress: percent } : it)),
            ),
        })
          .then((saved) =>
            setItems((prev) =>
              prev.map((it) =>
                it.id === id
                  ? { ...it, progress: 100, status: 'done', detail: saved[0]?.name }
                  : it,
              ),
            ),
          )
          .catch((err: unknown) =>
            setItems((prev) =>
              prev.map((it) =>
                it.id === id
                  ? {
                      ...it,
                      status: 'error',
                      detail: err instanceof Error ? err.message : 'Upload failed',
                    }
                  : it,
              ),
            ),
          )
      }
    },
    [token, unlockedPassword],
  )

  const handleDrop = useCallback(
    (e: DragEvent) => {
      e.preventDefault()
      setDragOver(false)
      if (e.dataTransfer.files.length > 0) uploadFiles(e.dataTransfer.files)
    },
    [uploadFiles],
  )

  const shell = (content: React.ReactNode) => (
    <div className="min-h-screen flex items-center justify-center px-4">
      <div className="grain-overlay" />
      <div className="w-full max-w-[480px]">
        <div className="text-center mb-6">
          <h1 className="font-mono font-bold text-[28px] text-primary tracking-widest uppercase">
            RUSTYFILE
          </h1>
          <p className="font-mono text-[12px] text-muted uppercase tracking-widest mt-1">
            SHARED LINK
          </p>
        </div>
        <div className="bg-surface border border-borders p-8">{content}</div>
      </div>
    </div>
  )

  if (loading) {
    return shell(
      <p className="font-mono text-[13px] text-muted uppercase tracking-widest text-center">
        [ LOADING... ]
      </p>,
    )
  }

  if (notFound || !meta) {
    return shell(
      <div className="text-center space-y-3">
        <WarningTriangle width={32} height={32} strokeWidth={1.5} className="text-primary mx-auto" />
        <p className="font-mono text-[14px] text-text-main uppercase tracking-widest font-bold">
          LINK NOT FOUND
        </p>
        <p className="font-mono text-[12px] text-muted uppercase tracking-wider">
          THIS SHARE DOES NOT EXIST OR HAS EXPIRED
        </p>
      </div>,
    )
  }

  // Locked: password gate before anything else.
  const locked = meta.has_password && unlockedPassword === null

  if (locked) {
    return shell(
      <form onSubmit={handleUnlock} className="space-y-5">
        <div className="text-center space-y-2">
          <Lock width={28} height={28} strokeWidth={1.5} className="text-primary mx-auto" />
          <p className="font-mono text-[14px] text-text-main uppercase tracking-widest font-bold truncate">
            {meta.name}
          </p>
          <p className="font-mono text-[12px] text-muted uppercase tracking-wider">
            THIS LINK IS PASSWORD PROTECTED
          </p>
        </div>
        <input
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          className="w-full h-12 bg-background border border-borders text-text-main font-mono px-4 rounded-none focus:border-primary focus:outline-none transition-colors"
          placeholder="password"
          autoComplete="off"
          autoFocus
        />
        {passwordError && (
          <p className="font-mono text-[12px] text-primary uppercase tracking-wider text-center">
            [ {passwordError} ]
          </p>
        )}
        <button
          type="submit"
          disabled={unlocking || !password}
          className="w-full h-12 bg-primary text-background font-mono font-bold text-[14px] uppercase tracking-widest transition-all hover:-translate-x-0.5 hover:-translate-y-0.5 hover:shadow-[4px_4px_0px_#F2F2F2] disabled:opacity-50 disabled:hover:translate-x-0 disabled:hover:translate-y-0 disabled:hover:shadow-none"
        >
          {unlocking ? 'CHECKING...' : 'UNLOCK'}
        </button>
      </form>,
    )
  }

  if (meta.kind === 'drop') {
    return shell(
      <div className="space-y-5">
        <div className="text-center space-y-1">
          <Folder width={28} height={28} strokeWidth={1.5} className="text-primary mx-auto" />
          <p className="font-mono text-[14px] text-text-main uppercase tracking-widest font-bold truncate">
            {meta.name}
          </p>
          <p className="font-mono text-[12px] text-muted uppercase tracking-wider">
            DROP FILES HERE TO SEND THEM
          </p>
        </div>

        <div
          onDragOver={(e) => {
            e.preventDefault()
            setDragOver(true)
          }}
          onDragLeave={() => setDragOver(false)}
          onDrop={handleDrop}
          onClick={() => fileInputRef.current?.click()}
          className={`border-2 border-dashed p-8 text-center cursor-pointer transition-colors ${
            dragOver ? 'border-primary bg-primary/10' : 'border-borders hover:border-text-main'
          }`}
        >
          <Upload width={24} height={24} strokeWidth={1.8} className="text-muted mx-auto mb-2" />
          <p className="font-mono text-[12px] text-muted uppercase tracking-widest">
            DRAG &amp; DROP OR CLICK TO SELECT
          </p>
          <input
            ref={fileInputRef}
            type="file"
            multiple
            className="hidden"
            onChange={(e) => {
              if (e.target.files) uploadFiles(e.target.files)
              e.target.value = ''
            }}
          />
        </div>

        {items.length > 0 && (
          <div className="space-y-2">
            {items.map((item) => (
              <div key={item.id} className="border border-borders px-3 py-2">
                <div className="flex items-center gap-2">
                  <span className="flex-1 truncate font-mono text-[12px] text-text-main">
                    {item.name}
                  </span>
                  {item.status === 'done' && (
                    <Check width={14} height={14} strokeWidth={2} className="text-primary shrink-0" />
                  )}
                  <span className="font-mono text-[11px] text-muted uppercase tracking-wider shrink-0">
                    {item.status === 'uploading'
                      ? `${item.progress}%`
                      : item.status === 'done'
                        ? item.detail && item.detail !== item.name
                          ? `SAVED AS ${item.detail}`
                          : 'SENT'
                        : 'FAILED'}
                  </span>
                </div>
                {item.status === 'uploading' && (
                  <div className="h-[2px] w-full bg-borders mt-2">
                    <div
                      className="h-full bg-primary transition-all"
                      style={{ width: `${item.progress}%` }}
                    />
                  </div>
                )}
                {item.status === 'error' && item.detail && (
                  <p className="font-mono text-[11px] text-primary uppercase tracking-wider mt-1">
                    [ {item.detail} ]
                  </p>
                )}
              </div>
            ))}
          </div>
        )}
      </div>,
    )
  }

  // Download share.
  return shell(
    <div className="space-y-5 text-center">
      {meta.is_dir ? (
        <Folder width={32} height={32} strokeWidth={1.5} className="text-primary mx-auto" />
      ) : (
        <Page width={32} height={32} strokeWidth={1.5} className="text-primary mx-auto" />
      )}
      <div>
        <p className="font-mono text-[15px] text-text-main uppercase tracking-widest font-bold break-all">
          {meta.name}
        </p>
        <p className="font-mono text-[12px] text-muted uppercase tracking-wider mt-1">
          {meta.is_dir ? 'FOLDER (ZIP)' : meta.size !== undefined ? formatSize(meta.size) : ''}
        </p>
      </div>
      {downloadError && (
        <p className="font-mono text-[12px] text-primary uppercase tracking-wider">
          [ {downloadError} ]
        </p>
      )}
      <button
        onClick={() => void handleDownload()}
        className="w-full h-12 bg-primary text-background font-mono font-bold text-[14px] uppercase tracking-widest flex items-center justify-center gap-2 transition-all hover:-translate-x-0.5 hover:-translate-y-0.5 hover:shadow-[4px_4px_0px_#F2F2F2]"
      >
        <Download width={18} height={18} strokeWidth={2} />
        DOWNLOAD
      </button>
    </div>,
  )
}
