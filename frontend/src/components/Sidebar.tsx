import { NavLink } from 'react-router'
import { Folder, Home, Journal, Settings, HardDrive } from 'iconoir-react'

const NAV_ITEMS = [
  { to: '/browse', label: 'Root', icon: Folder },
  { to: '/browse/home', label: 'Home', icon: Home },
  { to: '/browse/var/log', label: 'Logs', icon: Journal },
  { to: '/browse/etc', label: 'Config', icon: Settings },
] as const

export default function Sidebar() {
  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="h-16 flex items-center px-5 border-b border-borders shrink-0">
        <h1 className="font-mono text-xl font-bold text-primary-container tracking-tighter uppercase">
          SYS_DIR
        </h1>
      </div>

      {/* Navigation */}
      <nav className="flex-1 py-3 px-3 space-y-0.5 overflow-y-auto">
        {NAV_ITEMS.map(({ to, label, icon: Icon }) => (
          <NavLink
            key={to}
            to={to}
            end={to === '/browse'}
            className={({ isActive }) =>
              `flex items-center gap-3 h-10 px-3 font-mono text-[13px] uppercase tracking-wider transition-colors ${
                isActive
                  ? 'text-primary bg-surface border border-borders'
                  : 'text-muted border border-transparent hover:bg-surface hover:border-borders'
              }`
            }
          >
            <Icon width={18} height={18} strokeWidth={1.8} />
            {label}
          </NavLink>
        ))}
      </nav>

      {/* Storage meter */}
      <div className="px-5 py-4 border-t border-borders">
        <div className="flex items-center justify-between mb-2">
          <div className="flex items-center gap-2 text-muted">
            <HardDrive width={14} height={14} strokeWidth={1.8} />
            <span className="font-mono text-[11px] uppercase tracking-widest">
              STORAGE
            </span>
          </div>
          <span className="font-mono text-[11px] text-primary uppercase tracking-wider">
            64%
          </span>
        </div>
        <div className="h-[1px] w-full bg-borders">
          <div className="h-full bg-primary" style={{ width: '64%' }} />
        </div>
        <p className="font-mono text-[10px] text-muted mt-1.5 tracking-wider">
          128.4 GB / 200 GB
        </p>
      </div>
    </div>
  )
}
