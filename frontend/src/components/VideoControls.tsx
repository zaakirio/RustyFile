import { useRef, useState, useCallback } from 'react'

interface VideoControlsProps {
  videoRef: React.RefObject<HTMLVideoElement | null>
  duration: number
  currentTime: number
  playing: boolean
  onTogglePlay: () => void
  onSeek: (time: number) => void
  onToggleFullscreen: () => void
}

function formatTime(seconds: number): string {
  const mins = Math.floor(seconds / 60)
  const secs = Math.floor(seconds % 60)
  return `${String(mins).padStart(2, '0')}:${String(secs).padStart(2, '0')}`
}

export default function VideoControls({
  videoRef,
  duration,
  currentTime,
  playing,
  onTogglePlay,
  onSeek,
  onToggleFullscreen,
}: VideoControlsProps) {
  const scrubberRef = useRef<HTMLDivElement>(null)
  const [volume, setVolume] = useState(1)
  const [muted, setMuted] = useState(false)
  const [scrubberHover, setScrubberHover] = useState(false)

  const progress = duration > 0 ? (currentTime / duration) * 100 : 0

  const handleScrubberClick = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      const bar = scrubberRef.current
      if (!bar || duration <= 0) return
      const rect = bar.getBoundingClientRect()
      const x = e.clientX - rect.left
      const ratio = Math.max(0, Math.min(1, x / rect.width))
      onSeek(ratio * duration)
    },
    [duration, onSeek],
  )

  const handleVolumeToggle = useCallback(() => {
    const video = videoRef.current
    if (!video) return
    if (muted) {
      video.muted = false
      video.volume = volume
      setMuted(false)
    } else {
      video.muted = true
      setMuted(true)
    }
  }, [videoRef, muted, volume])

  // Keep volume in sync when user unmutes
  const handleVolumeUp = useCallback(() => {
    const video = videoRef.current
    if (!video) return
    const next = Math.min(1, volume + 0.25)
    video.volume = next
    video.muted = false
    setVolume(next)
    setMuted(false)
  }, [videoRef, volume])

  return (
    <div className="absolute inset-x-0 bottom-0 bg-gradient-to-t from-background/90 to-transparent pt-16 px-4 pb-4">
      {/* Scrubber bar */}
      <div
        ref={scrubberRef}
        className="relative w-full h-6 flex items-center cursor-pointer group"
        onClick={handleScrubberClick}
        onMouseEnter={() => setScrubberHover(true)}
        onMouseLeave={() => setScrubberHover(false)}
      >
        {/* Track */}
        <div className="absolute inset-x-0 h-[2px] bg-borders top-1/2 -translate-y-1/2">
          {/* Progress fill */}
          <div
            className="absolute inset-y-0 left-0 bg-primary"
            style={{ width: `${progress}%` }}
          />
        </div>

        {/* Playhead - square, appears on hover */}
        <div
          className={`absolute top-1/2 -translate-y-1/2 -translate-x-1/2 w-2 h-4 bg-text-main transition-opacity duration-200 ${
            scrubberHover ? 'opacity-100' : 'opacity-0'
          }`}
          style={{ left: `${progress}%` }}
        />
      </div>

      {/* Controls row */}
      <div className="flex items-center justify-between mt-2">
        {/* Left controls */}
        <div className="flex items-center gap-3">
          <button
            onClick={onTogglePlay}
            className="font-mono text-[14px] uppercase tracking-wider text-text-main hover:text-primary transition-colors"
          >
            {playing ? '[ || ]' : '[ > ]'}
          </button>
          <button
            onClick={handleVolumeToggle}
            onDoubleClick={handleVolumeUp}
            className="font-mono text-[14px] uppercase tracking-wider text-text-main hover:text-primary transition-colors"
            title={muted ? 'Unmute' : 'Mute'}
          >
            {muted ? '[ MUTE ]' : '[ VOL ]'}
          </button>
        </div>

        {/* Right controls */}
        <div className="flex items-center gap-4">
          <span className="font-mono text-[14px] uppercase tracking-wider">
            <span className="text-text-main">{formatTime(currentTime)}</span>
            <span className="text-muted"> / {formatTime(duration)}</span>
          </span>
          <button
            onClick={onToggleFullscreen}
            className="font-mono text-[14px] uppercase tracking-wider text-text-main hover:text-primary transition-colors"
          >
            [ FULL ]
          </button>
        </div>
      </div>
    </div>
  )
}
