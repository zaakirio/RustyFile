import { useEffect, useState } from 'react'
import { NavLink } from 'react-router'
import { Folder, HardDrive } from 'iconoir-react'
import { api } from '../api/client'
import type { DirListing } from '../lib/types'

export default function Sidebar() {
  const [dirs, setDirs] = useState<{ name: string; path: string }[]>([])
  const [loadError, setLoadError] = useState<string | null>(null)

  useEffect(() => {
    const controller = new AbortController()
    api
      .get<DirListing>('/api/fs', controller.signal)
      .then((listing) => {
        setDirs(
          listing.items
            .filter((i) => i.is_dir)
            .map((i) => ({ name: i.name, path: i.path })),
        )
      })
      .catch((err) => {
        if (err instanceof DOMException && err.name === 'AbortError') return
        setLoadError(err instanceof Error ? err.message : 'Failed to load directories')
      })
    return () => controller.abort()
  }, [])

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="h-16 flex items-center gap-3 px-5 border-b border-borders shrink-0">
        <img src="/logo.png" alt="RustyFile" className="h-8 w-auto" />
        <h1 className="font-mono text-xl font-bold text-primary-container tracking-tighter uppercase">
          RUSTYFILE
        </h1>
      </div>

      {/* Navigation */}
      <nav className="flex-1 py-3 px-3 space-y-0.5 overflow-y-auto">
        {loadError && (
          <div className="px-3 py-2 font-mono text-[11px] text-primary uppercase tracking-wider">
            [ {loadError} ]
          </div>
        )}
        <NavLink
          to="/browse"
          end
          className={({ isActive }) =>
            `flex items-center gap-3 h-10 px-3 font-mono text-[13px] uppercase tracking-wider transition-colors ${
              isActive
                ? 'text-primary bg-surface border border-borders'
                : 'text-muted border border-transparent hover:bg-surface hover:border-borders'
            }`
          }
        >
          <Folder width={18} height={18} strokeWidth={1.8} />
          Root
        </NavLink>
        {dirs.map((d) => (
          <NavLink
            key={d.path}
            to={`/browse/${d.path}`}
            className={({ isActive }) =>
              `flex items-center gap-3 h-10 px-3 font-mono text-[13px] uppercase tracking-wider transition-colors ${
                isActive
                  ? 'text-primary bg-surface border border-borders'
                  : 'text-muted border border-transparent hover:bg-surface hover:border-borders'
              }`
            }
          >
            <Folder width={18} height={18} strokeWidth={1.8} />
            {d.name}
          </NavLink>
        ))}
      </nav>

      {/* TODO: wire to /api/system/info when available */}
      <div className="px-5 py-4 border-t border-borders">
        <div className="flex items-center justify-between mb-2">
          <div className="flex items-center gap-2 text-muted">
            <HardDrive width={14} height={14} strokeWidth={1.8} />
            <span className="font-mono text-[11px] uppercase tracking-widest">
              STORAGE
            </span>
          </div>
          <span className="font-mono text-[11px] text-muted uppercase tracking-wider">
            --%
          </span>
        </div>
        <div className="h-[1px] w-full bg-borders">
          <div className="h-full bg-primary" style={{ width: '0%' }} />
        </div>
        <p className="font-mono text-[10px] text-muted mt-1.5 tracking-wider">
          -- / --
        </p>
      </div>
    </div>
  )
}
