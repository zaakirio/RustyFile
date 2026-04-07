import { useRef, useState, useCallback } from 'react'
import { useLocation, Link } from 'react-router'
import VideoControls from '../components/VideoControls'

export default function PlayerPage() {
  const location = useLocation()
  const videoRef = useRef<HTMLVideoElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)

  const [playing, setPlaying] = useState(false)
  const [duration, setDuration] = useState(0)
  const [currentTime, setCurrentTime] = useState(0)
  const [controlsVisible, setControlsVisible] = useState(false)

  // Extract path from URL: /play/path/to/file.mp4 -> "path/to/file.mp4"
  const filePath = location.pathname
    .replace(/^\/play\/?/, '')
    .replace(/\/$/, '')

  const fileName = filePath.split('/').pop() ?? 'Unknown'

  // Build parent directory path for RETURN navigation
  const parentSegments = filePath.split('/')
  parentSegments.pop()
  const parentPath = parentSegments.join('/')

  const videoSrc = `/api/fs/download/${encodeURIComponent(filePath)}?inline=true`

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

  return (
    <div className="flex-1 flex flex-col h-full overflow-hidden">
      {/* Header */}
      <header className="h-14 flex items-center justify-between px-4 md:px-6 border-b border-borders bg-background shrink-0 z-50">
        {/* Left: filename */}
        <h2 className="font-display text-xl font-bold uppercase tracking-tight text-text-main truncate mr-4">
          {fileName}
        </h2>

        {/* Right: return link */}
        <Link
          to={`/browse/${parentPath}`}
          className="font-mono text-[14px] uppercase tracking-wider text-muted hover:text-primary transition-colors shrink-0"
        >
          [ &lt;- ] RETURN
        </Link>
      </header>

      {/* Video container */}
      <div className="flex-1 flex items-center justify-center bg-black p-0 md:p-6 overflow-hidden">
        <div
          ref={containerRef}
          className="relative w-full md:w-[80vw] max-w-[1400px] aspect-video border-0 md:border md:border-borders"
          onMouseEnter={() => setControlsVisible(true)}
          onMouseLeave={() => setControlsVisible(false)}
        >
          <video
            ref={videoRef}
            src={videoSrc}
            className="w-full h-full object-contain bg-black"
            onClick={togglePlay}
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
              controlsVisible ? 'opacity-100' : 'opacity-0'
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
