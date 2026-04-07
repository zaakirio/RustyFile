import { Outlet } from 'react-router'
import Sidebar from './Sidebar'
import BottomNav from './BottomNav'
import { Settings } from 'iconoir-react'

export default function Layout() {
  return (
    <div className="flex h-screen overflow-hidden">
      {/* Desktop sidebar */}
      <aside className="hidden md:flex w-[250px] shrink-0 flex-col border-r border-borders bg-background h-screen">
        <Sidebar />
      </aside>

      {/* Main content */}
      <main className="flex-1 flex flex-col h-screen overflow-hidden pb-20 md:pb-0">
        {/* Mobile header */}
        <header className="md:hidden h-16 flex items-center justify-between px-4 border-b border-borders bg-background shrink-0">
          <h1 className="font-mono text-xl font-bold text-primary-container tracking-tighter uppercase">
            RUSTYFILE
          </h1>
          <button className="text-muted hover:text-primary transition-colors">
            <Settings width={22} height={22} strokeWidth={1.8} />
          </button>
        </header>

        {/* Routed content */}
        <Outlet />
      </main>

      {/* Mobile bottom nav */}
      <BottomNav />
    </div>
  )
}
