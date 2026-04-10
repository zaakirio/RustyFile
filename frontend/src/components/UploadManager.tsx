import type { UploadItem } from '../hooks/useTusUpload'
import { formatSize } from '../lib/format'

interface UploadManagerProps {
  items: UploadItem[]
  onPause: (id: string) => void
  onResume: (id: string) => void
  onClear: () => void
}

function formatSpeed(bytesPerSec: number): string {
  if (bytesPerSec <= 0) return ''
  if (bytesPerSec < 1024 * 1024) return `${(bytesPerSec / 1024).toFixed(0)} KB/s`
  return `${(bytesPerSec / (1024 * 1024)).toFixed(1)} MB/s`
}

function StatusBadge({ status }: { status: UploadItem['status'] }) {
  const map = {
    complete: { label: '[OK]', color: 'text-green-400' },
    uploading: { label: '[..]', color: 'text-primary' },
    queued: { label: '[--]', color: 'text-muted' },
    error: { label: '[!!]', color: 'text-red-400' },
    paused: { label: '[||]', color: 'text-yellow-400' },
  } as const

  const { label, color } = map[status]
  return <span className={`${color} shrink-0`}>{label}</span>
}

export default function UploadManager({
  items,
  onPause,
  onResume,
  onClear,
}: UploadManagerProps) {
  if (items.length === 0) return null

  const completed = items.filter((i) => i.status === 'complete').length
  const allDone = items.every(
    (i) => i.status === 'complete' || i.status === 'error',
  )

  return (
    <div className="bg-surface border-b border-borders px-4 py-3">
      {/* Header */}
      <div className="flex items-center justify-between mb-2">
        <p className="font-mono text-[12px] text-primary uppercase tracking-widest font-bold">
          [ UPLOADS {completed}/{items.length} ]
        </p>
        {allDone && (
          <button
            onClick={onClear}
            className="font-mono text-[11px] text-muted hover:text-primary uppercase tracking-widest transition-colors"
          >
            CLEAR
          </button>
        )}
      </div>

      {/* File rows */}
      <div className="flex flex-col gap-1.5">
        {items.map((item) => (
          <div key={item.id} className="flex items-center gap-2 font-mono text-[11px] tracking-wider">
            <StatusBadge status={item.status} />

            {/* Filename — truncated */}
            <span
              className="text-text-main truncate min-w-0 flex-1"
              title={item.name}
            >
              {item.name}
            </span>

            {/* Size */}
            <span className="text-muted shrink-0">{formatSize(item.size)}</span>

            {/* Speed */}
            {item.status === 'uploading' && item.speed > 0 && (
              <span className="text-muted shrink-0 w-20 text-right">
                {formatSpeed(item.speed)}
              </span>
            )}

            {/* Progress bar */}
            {(item.status === 'uploading' || item.status === 'paused') && (
              <div className="w-24 h-1.5 bg-background border border-borders shrink-0">
                <div
                  className="h-full bg-primary transition-[width] duration-200"
                  style={{ width: `${item.progress}%` }}
                />
              </div>
            )}

            {/* Progress percent */}
            {(item.status === 'uploading' || item.status === 'paused') && (
              <span className="text-muted shrink-0 w-8 text-right">
                {item.progress}%
              </span>
            )}

            {/* Pause/Resume button */}
            {item.status === 'uploading' && (
              <button
                onClick={() => onPause(item.id)}
                className="text-muted hover:text-yellow-400 transition-colors shrink-0"
                title="Pause"
              >
                ||
              </button>
            )}
            {item.status === 'paused' && (
              <button
                onClick={() => onResume(item.id)}
                className="text-muted hover:text-primary transition-colors shrink-0"
                title="Resume"
              >
                {'>'}
              </button>
            )}

            {/* Error message */}
            {item.status === 'error' && item.error && (
              <span className="text-red-400 truncate shrink-0 max-w-[140px]" title={item.error}>
                {item.error}
              </span>
            )}
          </div>
        ))}
      </div>
    </div>
  )
}
