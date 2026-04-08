import { useState, useEffect, useCallback, useRef } from 'react'
import { api } from '../api/client'
import type { FileEntry, SearchParams } from '../lib/types'

interface UseSearchResult {
  results: FileEntry[]
  total: number
  loading: boolean
  error: string | null
  search: (params: SearchParams) => void
  clear: () => void
  isActive: boolean
}

export function useSearch(): UseSearchResult {
  const [results, setResults] = useState<FileEntry[]>([])
  const [total, setTotal] = useState(0)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [isActive, setIsActive] = useState(false)
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const mountedRef = useRef(true)

  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
      if (debounceRef.current) clearTimeout(debounceRef.current)
    }
  }, [])

  const search = useCallback((params: SearchParams) => {
    if (debounceRef.current) clearTimeout(debounceRef.current)

    if (!params.q || params.q.length < 2) {
      setResults([])
      setTotal(0)
      setIsActive(false)
      setError(null)
      return
    }

    setIsActive(true)
    setLoading(true)

    debounceRef.current = setTimeout(async () => {
      try {
        const resp = await api.search(params)
        if (mountedRef.current) {
          setResults(resp.results)
          setTotal(resp.total)
          setError(null)
        }
      } catch (e: unknown) {
        if (mountedRef.current) {
          setError(e instanceof Error ? e.message : 'Search failed')
        }
      } finally {
        if (mountedRef.current) setLoading(false)
      }
    }, 300)
  }, [])

  const clear = useCallback(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current)
    setResults([])
    setTotal(0)
    setIsActive(false)
    setError(null)
    setLoading(false)
  }, [])

  return { results, total, loading, error, search, clear, isActive }
}
