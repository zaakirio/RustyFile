import { useRef, useState, useCallback, useEffect } from 'react'
import { useLocation, Link } from 'react-router'
import { getToken } from '../api/client'
import { encodeFsPath, extractFsPath } from '../lib/paths'
import VideoControls from '../components/VideoControls'

export default function PlayerPage() {
  const location = useLocation()
  const videoRef = useRef<HTMLVideoElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)

  const [playing, setPlaying] = useState(false)
  const [duration, setDuration] = useState(0)
  const [currentTime, setCurrentTime] = useState(0)
  const [controlsVisible, setControlsVisible] = useState(false)
  const [videoUrl, setVideoUrl] = useState<string | null>(null)

  // Extract path from URL: /play/path/to/file.mp4 -> "path/to/file.mp4"
  const filePath = extractFsPath(location.pathname, '/play/')

  const fileName = filePath.split('/').pop() ?? 'Unknown'

  // Build parent directory path for RETURN navigation
  const parentSegments = filePath.split('/')
  parentSegments.pop()
  const parentPath = parentSegments.join('/')

  // Detect touch device for persistent controls
  const isTouchDevice = 'ontouchstart' in window

  // MVP: Fetch video with auth headers and use blob URL.
  // Production should use HttpOnly cookies set by the server.
  useEffect(() => {
    let revoke: string | null = null
    const controller = new AbortController()

    fetch(`/api/fs/download/${encodeFsPath(filePath)}?inline=true`, {
      headers: { 'Authorization': `Bearer ${getToken() ?? ''}` },
      signal: controller.signal,
    })
      .then(r => r.blob())
      .then(blob => {
        revoke = URL.createObjectURL(blob)
        setVideoUrl(revoke)
      })
      .catch(() => {})

    return () => {
      controller.abort()
      if (revoke) URL.revokeObjectURL(revoke)
    }
  }, [filePath])

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
          tabIndex={0}
          className="relative w-full md:w-[80vw] max-w-[1400px] aspect-video border-0 md:border md:border-borders outline-none"
          onMouseEnter={() => setControlsVisible(true)}
          onMouseLeave={() => { if (!isTouchDevice) setControlsVisible(false) }}
          onClick={() => { if (isTouchDevice) setControlsVisible((v) => !v) }}
          onKeyDown={handleKeyDown}
        >
          {videoUrl ? (
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
          ) : (
            <div className="w-full h-full flex items-center justify-center">
              <span className="font-mono text-muted text-sm uppercase tracking-widest">
                [ LOADING... ]
              </span>
            </div>
          )}

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
