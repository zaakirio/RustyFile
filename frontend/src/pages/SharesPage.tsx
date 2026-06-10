import { useCallback, useEffect, useState } from 'react'
import { Check, Copy, Refresh, Trash, Lock, Xmark } from 'iconoir-react'
import { deleteShare, listShares, shareUrl } from '../api/shares'
import { formatDate } from '../lib/format'
import type { Share } from '../lib/types'

function formatUnix(secs: number): string {
  return formatDate(new Date(secs * 1000).toISOString())
}

function expiryLabel(share: Share): string {
  if (share.expires_at === null) return 'NEVER'
  if (share.expires_at * 1000 <= Date.now()) return 'EXPIRED'
  return formatUnix(share.expires_at)
}

export default function SharesPage() {
  const [shares, setShares] = useState<Share[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [copiedToken, setCopiedToken] = useState<string | null>(null)
  const [pendingDelete, setPendingDelete] = useState<Share | null>(null)

  const refresh = useCallback(async () => {
    setError(null)
    setLoading(true)
    try {
      setShares(await listShares())
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load shares')
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void refresh()
  }, [refresh])

  const handleCopy = useCallback(async (share: Share) => {
    try {
      await navigator.clipboard.writeText(shareUrl(share.token))
      setCopiedToken(share.token)
      setTimeout(() => setCopiedToken(null), 2000)
    } catch {
      // Clipboard unavailable; nothing else to do.
    }
  }, [])

  const confirmDelete = useCallback(async () => {
    if (!pendingDelete) return
    setError(null)
    try {
      await deleteShare(pendingDelete.token)
      setShares((prev) => prev.filter((s) => s.token !== pendingDelete.token))
      setPendingDelete(null)
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to delete share')
    }
  }, [pendingDelete])

  return (
    <div className="flex-1 flex flex-col overflow-hidden">
      {/* Header */}
      <header className="h-14 border-b border-borders flex items-center px-4 md:px-6 shrink-0 gap-4">
        <h1 className="font-mono text-[14px] font-bold text-primary uppercase tracking-widest">
          [ SHARE LINKS ]
        </h1>
        <button
          onClick={() => void refresh()}
          className="ml-auto p-2 text-muted hover:text-primary transition-colors"
          title="Refresh"
        >
          <Refresh width={18} height={18} strokeWidth={1.8} />
        </button>
      </header>

      {/* Error banner */}
      {error && (
        <div className="bg-surface border-b border-borders px-4 py-2.5 flex items-center gap-3">
          <span className="font-mono text-[12px] text-primary uppercase tracking-widest font-bold flex-1">
            [ ERROR: {error} ]
          </span>
          <button
            onClick={() => setError(null)}
            className="p-1 text-muted hover:text-primary transition-colors shrink-0"
            title="Dismiss"
          >
            <Xmark width={16} height={16} strokeWidth={2} />
          </button>
        </div>
      )}

      {/* Delete confirmation */}
      {pendingDelete && (
        <div className="bg-surface border-b border-borders px-4 py-3 flex items-center gap-3">
          <span className="font-mono text-[12px] text-primary uppercase tracking-widest font-bold truncate">
            REVOKE LINK FOR {pendingDelete.path || 'ROOT'}?
          </span>
          <button
            onClick={() => void confirmDelete()}
            className="font-mono text-[12px] font-bold uppercase tracking-widest px-3 py-1 bg-primary text-background hover:opacity-80 transition-opacity"
          >
            YES
          </button>
          <button
            onClick={() => setPendingDelete(null)}
            className="font-mono text-[12px] font-bold uppercase tracking-widest px-3 py-1 border border-borders text-text-main hover:border-text-main transition-colors"
          >
            NO
          </button>
        </div>
      )}

      {/* List */}
      <div className="flex-1 overflow-y-auto">
        {loading ? (
          <div className="flex items-center justify-center py-12">
            <span className="font-mono text-[14px] text-muted uppercase tracking-widest">
              [ LOADING... ]
            </span>
          </div>
        ) : shares.length === 0 ? (
          <div className="flex items-center justify-center py-12">
            <span className="font-mono text-[14px] text-muted uppercase tracking-widest">
              [ NO ACTIVE SHARES — USE THE SHARE BUTTON ON A FILE ]
            </span>
          </div>
        ) : (
          <>
            {/* Desktop column headers */}
            <div className="hidden md:grid grid-cols-[1fr_110px_90px_140px_80px_120px] items-center h-9 px-4 border-b border-borders">
              <span className="font-mono text-[11px] text-muted uppercase tracking-widest">PATH</span>
              <span className="font-mono text-[11px] text-muted uppercase tracking-widest">KIND</span>
              <span className="font-mono text-[11px] text-muted uppercase tracking-widest">COUNT</span>
              <span className="font-mono text-[11px] text-muted uppercase tracking-widest">EXPIRES</span>
              <span className="font-mono text-[11px] text-muted uppercase tracking-widest">LOCK</span>
              <span />
            </div>

            {shares.map((share) => (
              <div
                key={share.token}
                className="grid grid-cols-[1fr_auto] md:grid-cols-[1fr_110px_90px_140px_80px_120px] items-center min-h-11 px-4 py-2 border-b border-borders/40 hover:bg-surface transition-colors"
              >
                <div className="min-w-0">
                  <span
                    className={`block truncate text-[14px] ${
                      share.exists ? 'text-text-main' : 'text-muted line-through'
                    }`}
                    title={share.exists ? share.path : `${share.path} (missing on disk)`}
                  >
                    {share.path || 'ROOT'}
                  </span>
                  <span className="md:hidden font-mono text-[10px] text-muted uppercase tracking-wider">
                    {share.kind} / {share.download_count} DL / {expiryLabel(share)}
                    {share.has_password ? ' / LOCKED' : ''}
                  </span>
                </div>
                <span className="hidden md:block font-mono text-[12px] text-muted uppercase tracking-wider">
                  {share.kind}
                </span>
                <span className="hidden md:block font-mono text-[12px] text-muted uppercase tracking-wider">
                  {share.download_count}
                </span>
                <span className="hidden md:block font-mono text-[12px] text-muted uppercase tracking-wider">
                  {expiryLabel(share)}
                </span>
                <span className="hidden md:flex items-center text-muted">
                  {share.has_password && <Lock width={14} height={14} strokeWidth={2} />}
                </span>
                <div className="flex items-center justify-end gap-1">
                  <button
                    onClick={() => void handleCopy(share)}
                    className="p-1.5 text-muted hover:text-primary transition-colors"
                    title="Copy link"
                    aria-label={`Copy link for ${share.path}`}
                  >
                    {copiedToken === share.token ? (
                      <Check width={14} height={14} strokeWidth={2} />
                    ) : (
                      <Copy width={14} height={14} strokeWidth={2} />
                    )}
                  </button>
                  <button
                    onClick={() => setPendingDelete(share)}
                    className="p-1.5 text-muted hover:text-primary transition-colors"
                    title="Revoke"
                    aria-label={`Revoke share for ${share.path}`}
                  >
                    <Trash width={14} height={14} strokeWidth={2} />
                  </button>
                </div>
              </div>
            ))}

            <div className="hidden md:flex items-center h-9 px-4">
              <span className="font-mono text-[11px] text-muted uppercase tracking-widest">
                {shares.length} LINK{shares.length !== 1 ? 'S' : ''}
              </span>
            </div>
          </>
        )}
      </div>
    </div>
  )
}
