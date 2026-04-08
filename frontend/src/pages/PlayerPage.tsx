import { useRef, useState, useCallback } from 'react'
import { useLocation, useNavigate } from 'react-router'
import { NavArrowLeft, NavArrowRight } from 'iconoir-react'
import { encodeFsPath, extractFsPath } from '../lib/paths'
import Breadcrumbs from '../components/Breadcrumbs'
import VideoControls from '../components/VideoControls'

export default function PlayerPage() {
  const location = useLocation()
  const navigate = useNavigate()
  const videoRef = useRef<HTMLVideoElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)

  const [playing, setPlaying] = useState(false)
  const [duration, setDuration] = useState(0)
  const [currentTime, setCurrentTime] = useState(0)
  const [controlsVisible, setControlsVisible] = useState(false)

  // Extract path from URL: /play/path/to/file.mp4 -> "path/to/file.mp4"
  const filePath = extractFsPath(location.pathname, '/play/')

  const fileName = filePath.split('/').pop() ?? 'Unknown'

  // Build parent directory path for RETURN navigation
  const parentSegments = filePath.split('/')
  parentSegments.pop()
  const parentPath = parentSegments.join('/')

  // Detect touch device for persistent controls
  const isTouchDevice = 'ontouchstart' in window

  // Direct URL — backend checks rustyfile_token cookie for auth,
  // enabling HTTP Range requests for proper seeking/streaming.
  const videoUrl = `/api/fs/download/${encodeFsPath(filePath)}?inline=true`

  const togglePlay = useCallback(() => {
    const video = videoRef.current
    if (!video) return
    if (video.paused) {
      video.play()
    } else {
      video.pause()
    }
  }, [])

  const handleSeek = useCallback((time: number) => {
    const video = videoRef.current
    if (!video) return
    video.currentTime = time
  }, [])

  const handleToggleFullscreen = useCallback(() => {
    const el = containerRef.current
    if (!el) return
    if (document.fullscreenElement) {
      document.exitFullscreen()
    } else {
      el.requestFullscreen()
    }
  }, [])

  // Keyboard handler for container
  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    const video = videoRef.current
    if (!video) return

    switch (e.key) {
      case ' ':
        e.preventDefault()
        togglePlay()
        break
      case 'ArrowLeft':
        e.preventDefault()
        video.currentTime = Math.max(0, video.currentTime - 5)
        break
      case 'ArrowRight':
        e.preventDefault()
        video.currentTime = Math.min(video.duration, video.currentTime + 5)
        break
      case 'f':
        e.preventDefault()
        handleToggleFullscreen()
        break
    }
  }, [togglePlay, handleToggleFullscreen])

  return (
    <div className="flex-1 flex flex-col h-full overflow-hidden">
      {/* Header with navigation */}
      <header className="h-14 border-b border-borders flex items-center px-4 md:px-6 shrink-0 gap-4 z-50">
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
        <button
          onClick={() => navigate(`/browse/${encodeFsPath(parentPath)}`)}
          className="ml-auto font-mono text-[13px] font-bold uppercase tracking-widest px-3 py-1.5 bg-transparent border border-borders text-text-main hover:border-text-main transition-colors shrink-0"
        >
          CLOSE
        </button>
      </header>

      {/* Video container */}
      <div className="flex-1 flex items-center justify-center bg-black p-0 md:p-6 overflow-hidden">
        <div
          ref={containerRef}
          tabIndex={0}
          className="relative w-full md:w-[80vw] max-w-[1400px] aspect-video border-0 md:border md:border-borders outline-none"
          onMouseEnter={() => setControlsVisible(true)}
          onMouseLeave={() => { if (!isTouchDevice) setControlsVisible(false) }}
          onClick={() => { if (isTouchDevice) setControlsVisible((v) => !v) }}
          onKeyDown={handleKeyDown}
        >
          <video
            ref={videoRef}
            src={videoUrl}
            className="w-full h-full object-contain bg-black"
            onClick={(e) => { if (!isTouchDevice) { e.stopPropagation(); togglePlay() } }}
            onTimeUpdate={() =>
              setCurrentTime(videoRef.current?.currentTime ?? 0)
            }
            onLoadedMetadata={() =>
              setDuration(videoRef.current?.duration ?? 0)
            }
            onPlay={() => setPlaying(true)}
            onPause={() => setPlaying(false)}
          />

          {/* Custom controls overlay */}
          <div
            className={`absolute inset-0 transition-opacity duration-200 ${
              controlsVisible || isTouchDevice ? 'opacity-100' : 'opacity-0'
            }`}
          >
            <VideoControls
              videoRef={videoRef}
              duration={duration}
              currentTime={currentTime}
              playing={playing}
              onTogglePlay={togglePlay}
              onSeek={handleSeek}
              onToggleFullscreen={handleToggleFullscreen}
            />
          </div>
        </div>
      </div>
    </div>
  )
}
