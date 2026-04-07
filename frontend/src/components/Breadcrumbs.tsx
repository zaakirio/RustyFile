interface BreadcrumbsProps {
  path: string
  onNavigate: (path: string) => void
}

export default function Breadcrumbs({ path, onNavigate }: BreadcrumbsProps) {
  const segments = path ? path.split('/').filter(Boolean) : []

  return (
    <div className="flex items-center gap-0 font-mono text-[14px] min-w-0">
      <button
        onClick={() => onNavigate('')}
        className={`shrink-0 transition-colors hover:text-primary ${
          segments.length === 0
            ? 'text-primary font-bold'
            : 'text-muted'
        }`}
      >
        ~
      </button>

      {segments.map((segment, i) => {
        const segmentPath = segments.slice(0, i + 1).join('/')
        const isLast = i === segments.length - 1

        return (
          <span key={segmentPath} className="flex items-center min-w-0">
            <span className="text-muted mx-1 shrink-0">/</span>
            <button
              onClick={() => onNavigate(segmentPath)}
              className={`truncate transition-colors hover:text-primary ${
                isLast
                  ? 'text-primary font-bold'
                  : 'text-muted'
              }`}
            >
              {segment}
            </button>
          </span>
        )
      })}
    </div>
  )
}
