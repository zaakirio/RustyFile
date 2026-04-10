import {
  createContext,
  useContext,
  useState,
  useEffect,
  useCallback,
} from 'react'
import type { ReactNode } from 'react'
import { api } from '../api/client'
import type { User, AuthResponse, SetupStatus } from '../lib/types'

interface AuthState {
  user: User | null
  loading: boolean
  setupRequired: boolean | null
  login: (username: string, password: string) => Promise<void>
  setupAdmin: (username: string, password: string) => Promise<void>
  logout: () => void
}

const AuthContext = createContext<AuthState | null>(null)

export function AuthProvider({ children }: { children: ReactNode }) {
  const [user, setUser] = useState<User | null>(null)
  const [loading, setLoading] = useState(true)
  const [setupRequired, setSetupRequired] = useState<boolean | null>(null)

  useEffect(() => {
    const handleExpired = () => {
      setUser(null)
    }
    window.addEventListener('rustyfile:auth-expired', handleExpired)

    const init = async () => {
      try {
        const status = await api.get<SetupStatus>('/api/setup/status')
        setSetupRequired(status.setup_required)
        if (!status.setup_required) {
          // Attempt refresh — the HttpOnly cookie carries the session.
          try {
            const res = await api.post<AuthResponse>('/api/auth/refresh')
            setUser(res.user)
          } catch {
            // No valid session
          }
        }
      } catch {
        // Server unreachable
      } finally {
        setLoading(false)
      }
    }
    init()

    return () => {
      window.removeEventListener('rustyfile:auth-expired', handleExpired)
    }
  }, [])

  const login = useCallback(
    async (username: string, password: string) => {
      const res = await api.post<AuthResponse>('/api/auth/login', {
        username,
        password,
      })
      setUser(res.user)
      setSetupRequired(false)
    },
    [],
  )

  const setupAdmin = useCallback(
    async (username: string, password: string) => {
      const res = await api.post<AuthResponse>('/api/setup/admin', {
        username,
        password,
        password_confirm: password,
      })
      setUser(res.user)
      setSetupRequired(false)
    },
    [],
  )

  const logout = useCallback(() => {
    setUser(null)
    api.post('/api/auth/logout').catch((err) => {
      if (import.meta.env.DEV) console.warn('Logout request failed:', err)
    })
  }, [])

  return (
    <AuthContext.Provider
      value={{ user, loading, setupRequired, login, setupAdmin, logout }}
    >
      {children}
    </AuthContext.Provider>
  )
}

export function useAuth() {
  const ctx = useContext(AuthContext)
  if (!ctx) throw new Error('useAuth must be used within AuthProvider')
  return ctx
}
