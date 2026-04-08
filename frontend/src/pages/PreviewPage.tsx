import { useLocation, useNavigate } from 'react-router'
import { NavArrowLeft, NavArrowRight, Download } from 'iconoir-react'
import { encodeFsPath, extractFsPath } from '../lib/paths'
import Breadcrumbs from '../components/Breadcrumbs'

export default function PreviewPage() {
  const location = useLocation()
  const navigate = useNavigate()

  const filePath = extractFsPath(location.pathname, '/preview/')
  const filename = filePath.split('/').pop() ?? ''

  const parentSegments = filePath.split('/')
  parentSegments.pop()
  const parentPath = parentSegments.join('/')

  const imageUrl = `/api/fs/download/${encodeFsPath(filePath)}?inline=true`

  return (
    <div className="flex-1 flex flex-col h-full overflow-hidden">
      {/* Header with navigation */}
      <header className="h-14 border-b border-borders flex items-center px-4 md:px-6 shrink-0 gap-4">
        <div className="flex items-center gap-1 shrink-0">
          <button
            onClick={() => navigate(-1)}
            className="p-1.5 text-muted hover:text-primary transition-colors"
            title="Back"
          >
            <NavArrowLeft width={18} height={18} strokeWidth={1.8} />
          </button>
          <button
            onClick={() => navigate(1)}
            className="p-1.5 text-muted hover:text-primary transition-colors"
            title="Forward"
          >
            <NavArrowRight width={18} height={18} strokeWidth={1.8} />
          </button>
        </div>
        <Breadcrumbs
          path={filePath}
          onNavigate={(p) => navigate(`/browse/${encodeFsPath(p)}`)}
        />
        <div className="ml-auto flex items-center gap-2 shrink-0">
          <a
            href={`/api/fs/download/${encodeFsPath(filePath)}`}
            className="p-2 text-muted hover:text-primary transition-colors"
            title="Download"
          >
            <Download width={18} height={18} strokeWidth={1.8} />
          </a>
          <button
            onClick={() => navigate(`/browse/${encodeFsPath(parentPath)}`)}
            className="font-mono text-[13px] font-bold uppercase tracking-widest px-3 py-1.5 bg-transparent border border-borders text-text-main hover:border-text-main transition-colors"
          >
            CLOSE
          </button>
        </div>
      </header>

      {/* Image display */}
      <div className="flex-1 flex items-center justify-center bg-black/50 p-4 overflow-hidden">
        <img
          src={imageUrl}
          alt={filename}
          className="max-w-full max-h-full object-contain"
        />
      </div>

      {/* Footer with filename */}
      <footer className="h-8 bg-surface border-t border-borders flex items-center px-4 shrink-0">
        <span className="font-mono text-[11px] text-muted uppercase tracking-wider truncate">
          {filename}
        </span>
      </footer>
    </div>
  )
}
