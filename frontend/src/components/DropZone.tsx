import { UploadSquare } from 'iconoir-react'

interface DropZoneProps {
  visible: boolean
  targetPath: string
}

export default function DropZone({ visible, targetPath }: DropZoneProps) {
  if (!visible) return null

  return (
    <div className="absolute inset-0 z-50 bg-surface/90 backdrop-blur-sm p-4">
      <div className="w-full h-full border-2 border-dashed border-primary flex items-center justify-center flex-col gap-4">
        <UploadSquare
          width={48}
          height={48}
          strokeWidth={1.5}
          className="text-primary"
        />
        <p className="font-mono text-primary text-xl font-bold tracking-widest uppercase">
          [ DROP TO UPLOAD ]
        </p>
        <p className="font-mono text-muted text-sm tracking-wider uppercase">
          TARGET: /{targetPath || 'root'}
        </p>
      </div>
    </div>
  )
}
