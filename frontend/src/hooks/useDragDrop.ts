import { useState, useCallback, useRef, type DragEvent } from 'react'
import { getToken } from '../api/client'
import { encodeFsPath } from '../lib/paths'

interface UploadProgress {
  name: string
  done: boolean
}

interface UploadState {
  isDragging: boolean
  uploading: boolean
  progress: UploadProgress[]
}

export function useDragDrop(currentPath: string, onComplete: () => void) {
  const [state, setState] = useState<UploadState>({
    isDragging: false,
    uploading: false,
    progress: [],
  })
  const [errors, setErrors] = useState<string[]>([])

  // Track drag counter with ref to handle nested element enter/leave events
  const dragCounter = useRef(0)

  const onDragEnter = useCallback((e: DragEvent) => {
    e.preventDefault()
    dragCounter.current++
    setState((s) => ({ ...s, isDragging: true }))
  }, [])

  const onDragLeave = useCallback((e: DragEvent) => {
    e.preventDefault()
    dragCounter.current--
    if (dragCounter.current === 0) {
      setState((s) => ({ ...s, isDragging: false }))
    }
  }, [])

  const onDragOver = useCallback((e: DragEvent) => {
    e.preventDefault()
  }, [])

  const uploadFile = useCallback(
    async (file: File) => {
      const dest = currentPath ? `${currentPath}/${file.name}` : file.name
      const token = getToken()
      const res = await fetch(`/api/fs/${encodeFsPath(dest)}`, {
        method: 'PUT',
        headers: {
          ...(token ? { Authorization: `Bearer ${token}` } : {}),
          'Content-Type': 'application/octet-stream',
        },
        body: file,
      })
      if (!res.ok) {
        throw new Error(`Upload failed: ${res.status} ${res.statusText}`)
      }
    },
    [currentPath],
  )

  const processFiles = useCallback(
    async (files: File[]) => {
      if (files.length === 0) return

      const progress: UploadProgress[] = files.map((f) => ({
        name: f.name,
        done: false,
      }))
      setState({ isDragging: false, uploading: true, progress })
      setErrors([])

      for (let i = 0; i < files.length; i++) {
        try {
          await uploadFile(files[i])
          progress[i].done = true
          setState((s) => ({ ...s, progress: [...progress] }))
        } catch (err) {
          console.error(`Upload failed: ${files[i].name}`, err)
          setErrors((prev) => [...prev, files[i].name])
        }
      }

      setState({ isDragging: false, uploading: false, progress: [] })
      onComplete()
    },
    [uploadFile, onComplete],
  )

  const onDrop = useCallback(
    async (e: DragEvent) => {
      e.preventDefault()
      dragCounter.current = 0
      const files = Array.from(e.dataTransfer.files)
      if (files.length === 0) {
        setState((s) => ({ ...s, isDragging: false }))
        return
      }
      await processFiles(files)
    },
    [processFiles],
  )

  // For mobile: open native file picker
  const uploadFromPicker = useCallback(() => {
    const input = document.createElement('input')
    input.type = 'file'
    input.multiple = true
    input.style.display = 'none'
    document.body.appendChild(input)
    input.onchange = async () => {
      const files = Array.from(input.files ?? [])
      if (files.length > 0) {
        await processFiles(files)
      }
      input.remove()
    }
    input.click()
  }, [processFiles])

  const clearErrors = useCallback(() => setErrors([]), [])

  return {
    ...state,
    errors,
    clearErrors,
    dragHandlers: { onDragEnter, onDragLeave, onDragOver, onDrop },
    uploadFromPicker,
  }
}
