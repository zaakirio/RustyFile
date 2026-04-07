import { NavLink } from 'react-router'
import { Folder, EditPencil, MediaVideo, Bookmark } from 'iconoir-react'

const TABS = [
  { to: '/browse', label: 'BROWSE', icon: Folder },
  { to: '/edit', label: 'EDITOR', icon: EditPencil },
  { to: '/play', label: 'MEDIA', icon: MediaVideo },
  { to: '/stash', label: 'STASH', icon: Bookmark },
] as const

export default function BottomNav() {
  return (
    <nav className="md:hidden fixed bottom-0 left-0 right-0 h-20 bg-surface-lowest flex items-stretch z-50">
      {TABS.map(({ to, label, icon: Icon }) => (
        <NavLink
          key={to}
          to={to}
          className={({ isActive }) =>
            `flex-1 flex flex-col items-center justify-center gap-1.5 transition-colors ${
              isActive
                ? 'text-primary-light border-t-[4px] border-primary-light'
                : 'text-outline border-t-[4px] border-transparent'
            }`
          }
        >
          <Icon width={20} height={20} strokeWidth={1.8} />
          <span className="font-mono text-[10px] uppercase tracking-widest">
            {label}
          </span>
        </NavLink>
      ))}
    </nav>
  )
}
