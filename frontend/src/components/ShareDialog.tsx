import { useCallback, useState } from 'react'
import { Check, Copy, Xmark } from 'iconoir-react'
import { createShare, shareUrl } from '../api/shares'
import type { FileEntry, Share, ShareKind } from '../lib/types'

interface ShareDialogProps {
  entry: FileEntry
  onClose: () => void
  /** Called after a share is created so lists can refresh. */
  onCreated?: (share: Share) => void
}

const EXPIRY_OPTIONS: { label: string; hours?: number }[] = [
  { label: '1 HOUR', hours: 1 },
  { label: '24 HOURS', hours: 24 },
  { label: '7 DAYS', hours: 7 * 24 },
  { label: '30 DAYS', hours: 30 * 24 },
  { label: 'NEVER', hours: undefined },
]

export default function ShareDialog({ entry, onClose, onCreated }: ShareDialogProps) {
  const [kind, setKind] = useState<ShareKind>('download')
  const [password, setPassword] = useState('')
  const [expiry, setExpiry] = useState<string>('168') // default 7 days
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [created, setCreated] = useState<Share | null>(null)
  const [copied, setCopied] = useState(false)

  const handleCreate = useCallback(async () => {
    setError(null)
    setSubmitting(true)
    try {
      const share = await createShare({
        path: entry.path,
        kind,
        password: password || undefined,
        expires_in_hours: expiry ? Number(expiry) : undefined,
      })
      setCreated(share)
      onCreated?.(share)
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to create share')
    } finally {
      setSubmitting(false)
    }
  }, [entry.path, kind, password, expiry, onCreated])

  const handleCopy = useCallback(async () => {
    if (!created) return
    try {
      await navigator.clipboard.writeText(shareUrl(created.token))
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch {
      // Clipboard unavailable (e.g. non-secure context); the URL stays
      // selectable in the input.
    }
  }, [created])

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-background/80 px-4"
      onClick={onClose}
    >
      <div
        className="w-full max-w-[440px] bg-surface border border-borders p-6"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between mb-5">
          <h2 className="font-mono text-[14px] font-bold text-primary uppercase tracking-widest">
            [ SHARE: {entry.name} ]
          </h2>
          <button
            onClick={onClose}
            className="p-1 text-muted hover:text-primary transition-colors"
            title="Close"
            aria-label="Close share dialog"
          >
            <Xmark width={18} height={18} strokeWidth={2} />
          </button>
        </div>

        {created ? (
          <div className="space-y-4">
            <p className="font-mono text-[12px] text-muted uppercase tracking-wider">
              LINK READY — ANYONE WITH THIS URL CAN {created.kind === 'drop' ? 'UPLOAD' : 'DOWNLOAD'}
              {created.has_password ? ' (PASSWORD REQUIRED)' : ''}
            </p>
            <div className="flex items-stretch gap-2">
              <input
                type="text"
                readOnly
                value={shareUrl(created.token)}
                onFocus={(e) => e.target.select()}
                className="flex-1 h-10 bg-background border border-borders text-text-main font-mono text-[12px] px-3 rounded-none focus:border-primary focus:outline-none min-w-0"
              />
              <button
                onClick={handleCopy}
                className="flex items-center gap-1.5 px-3 bg-primary text-background font-mono text-[12px] font-bold uppercase tracking-widest hover:opacity-80 transition-opacity shrink-0"
              >
                {copied ? (
                  <Check width={14} height={14} strokeWidth={2} />
                ) : (
                  <Copy width={14} height={14} strokeWidth={2} />
                )}
                {copied ? 'COPIED' : 'COPY'}
              </button>
            </div>
            <button
              onClick={onClose}
              className="w-full h-10 border border-borders text-text-main font-mono text-[12px] font-bold uppercase tracking-widest hover:border-text-main transition-colors"
            >
              DONE
            </button>
          </div>
        ) : (
          <div className="space-y-4">
            {/* Kind */}
            <div>
              <span className="block font-mono text-[12px] text-muted uppercase tracking-wider mb-2">
                TYPE
              </span>
              <div className="flex gap-2">
                <button
                  onClick={() => setKind('download')}
                  className={`flex-1 h-10 font-mono text-[12px] font-bold uppercase tracking-widest border transition-colors ${
                    kind === 'download'
                      ? 'bg-primary text-background border-primary'
                      : 'border-borders text-muted hover:border-text-main'
                  }`}
                >
                  DOWNLOAD
                </button>
                {entry.is_dir && (
                  <button
                    onClick={() => setKind('drop')}
                    className={`flex-1 h-10 font-mono text-[12px] font-bold uppercase tracking-widest border transition-colors ${
                      kind === 'drop'
                        ? 'bg-primary text-background border-primary'
                        : 'border-borders text-muted hover:border-text-main'
                    }`}
                  >
                    DROP (RECEIVE)
                  </button>
                )}
              </div>
              {kind === 'drop' && (
                <p className="font-mono text-[11px] text-muted tracking-wider mt-1.5">
                  RECIPIENTS CAN UPLOAD FILES INTO THIS FOLDER
                </p>
              )}
            </div>

            {/* Password */}
            <div>
              <label
                htmlFor="share-password"
                className="block font-mono text-[12px] text-muted uppercase tracking-wider mb-2"
              >
                PASSWORD (OPTIONAL)
              </label>
              <input
                id="share-password"
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                autoComplete="new-password"
                className="w-full h-10 bg-background border border-borders text-text-main font-mono text-[13px] px-3 rounded-none focus:border-primary focus:outline-none transition-colors"
                placeholder="leave empty for none"
              />
            </div>

            {/* Expiry */}
            <div>
              <label
                htmlFor="share-expiry"
                className="block font-mono text-[12px] text-muted uppercase tracking-wider mb-2"
              >
                EXPIRES
              </label>
              <select
                id="share-expiry"
                value={expiry}
                onChange={(e) => setExpiry(e.target.value)}
                className="w-full h-10 bg-background border border-borders text-text-main font-mono text-[12px] px-2 rounded-none focus:border-primary focus:outline-none uppercase tracking-widest"
              >
                {EXPIRY_OPTIONS.map((opt) => (
                  <option key={opt.label} value={opt.hours ?? ''}>
                    {opt.label}
                  </option>
                ))}
              </select>
            </div>

            {error && (
              <p className="font-mono text-[12px] text-primary uppercase tracking-wider">
                [ ERROR: {error} ]
              </p>
            )}

            <button
              onClick={handleCreate}
              disabled={submitting}
              className="w-full h-11 bg-primary text-background font-mono text-[13px] font-bold uppercase tracking-widest hover:-translate-x-0.5 hover:-translate-y-0.5 hover:shadow-[3px_3px_0px_#F2F2F2] transition-all disabled:opacity-50 disabled:hover:translate-x-0 disabled:hover:translate-y-0 disabled:hover:shadow-none"
            >
              {submitting ? 'CREATING...' : 'CREATE LINK'}
            </button>
          </div>
        )}
      </div>
    </div>
  )
}
