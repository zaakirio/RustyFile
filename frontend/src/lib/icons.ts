import type { FunctionComponent, SVGProps } from 'react'
import {
  Folder,
  MediaVideo,
  MusicDoubleNote,
  MediaImage,
  Code,
  Css3,
  CodeBrackets,
  DataTransferBoth,
  Settings,
  Page,
  Journal,
  Archive,
} from 'iconoir-react'

type IconComponent = FunctionComponent<SVGProps<SVGSVGElement> & { title?: string }>

const MIME_PREFIX_MAP: [string, IconComponent][] = [
  ['video/', MediaVideo],
  ['audio/', MusicDoubleNote],
  ['image/', MediaImage],
]

const EXT_MAP: Record<string, IconComponent> = {
  html: Code,
  htm: Code,
  css: Css3,
  js: CodeBrackets,
  ts: CodeBrackets,
  jsx: CodeBrackets,
  tsx: CodeBrackets,
  json: DataTransferBoth,
  yaml: Settings,
  yml: Settings,
  toml: Settings,
  md: Page,
  txt: Page,
  pdf: Journal,
  zip: Archive,
  tar: Archive,
  gz: Archive,
  rs: CodeBrackets,
  py: CodeBrackets,
  go: CodeBrackets,
  sh: CodeBrackets,
}

export function getFileIcon(entry: {
  is_dir: boolean
  extension: string | null
  mime_type: string | null
}): IconComponent {
  if (entry.is_dir) return Folder

  const mime = entry.mime_type ?? ''
  for (const [prefix, icon] of MIME_PREFIX_MAP) {
    if (mime.startsWith(prefix)) return icon
  }

  const ext = entry.extension?.toLowerCase()
  if (ext && ext in EXT_MAP) return EXT_MAP[ext]

  return Page
}
