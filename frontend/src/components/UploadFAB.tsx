import { Upload } from 'iconoir-react'

interface UploadFABProps {
  onClick: () => void
}

export default function UploadFAB({ onClick }: UploadFABProps) {
  return (
    <button
      onClick={onClick}
      className="fixed right-6 bottom-24 md:hidden bg-primary-container text-background w-14 h-14 flex items-center justify-center border border-black z-[60]"
      style={{ boxShadow: '4px 4px 0px #000000' }}
      aria-label="Upload files"
    >
      <Upload width={24} height={24} strokeWidth={2} />
    </button>
  )
}
