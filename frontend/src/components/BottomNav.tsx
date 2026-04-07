import { NavLink, useLocation } from 'react-router'
import { Folder, EditPencil, MediaVideo, Bookmark } from 'iconoir-react'

const TABS = [
  { to: '/browse', label: 'BROWSE', icon: Folder, needsFile: false },
  { to: '/edit', label: 'EDITOR', icon: EditPencil, needsFile: true },
  { to: '/play', label: 'MEDIA', icon: MediaVideo, needsFile: true },
  { to: '/stash', label: 'STASH', icon: Bookmark, needsFile: false },
] as const

export default function BottomNav() {
  const location = useLocation()

  return (
    <nav className="md:hidden fixed bottom-0 left-0 right-0 h-20 bg-surface-lowest flex items-stretch z-50">
      {TABS.map(({ to, label, icon: Icon, needsFile }) => {
        // For file-dependent tabs, check if a file is currently open
        const hasFile = needsFile && location.pathname.startsWith(to + '/')
        const isDisabled = needsFile && !hasFile && !location.pathname.startsWith(to)
        const dest = isDisabled ? '/browse' : to

        return (
          <NavLink
            key={to}
            to={dest}
            className={({ isActive }) =>
              `flex-1 flex flex-col items-center justify-center gap-1.5 transition-colors ${
                isActive
                  ? 'text-primary-light border-t-[4px] border-primary-light'
                  : isDisabled
                    ? 'text-muted/40 border-t-[4px] border-transparent'
                    : 'text-outline border-t-[4px] border-transparent'
              }`
            }
          >
            <Icon width={20} height={20} strokeWidth={1.8} />
            <span className="font-mono text-[10px] uppercase tracking-widest">
              {label}
            </span>
          </NavLink>
        )
      })}
    </nav>
  )
}
