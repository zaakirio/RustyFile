import { useState } from 'react'
import type { FormEvent } from 'react'
import { useNavigate } from 'react-router'
import { LogIn } from 'iconoir-react'
import { useAuth } from '../hooks/useAuth'
import { ApiClientError } from '../api/client'

export default function LoginPage() {
  const { setupRequired, login, setupAdmin } = useAuth()
  const navigate = useNavigate()

  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState('')
  const [submitting, setSubmitting] = useState(false)

  const isSetup = setupRequired === true

  async function handleSubmit(e: FormEvent) {
    e.preventDefault()
    setError('')
    setSubmitting(true)

    try {
      if (isSetup) {
        await setupAdmin(username, password)
      } else {
        await login(username, password)
      }
      navigate('/browse')
    } catch (err) {
      if (err instanceof ApiClientError) {
        setError(err.message)
      } else {
        setError('CONNECTION_FAILED')
      }
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <div className="flex items-center justify-center min-h-screen px-4">
      <form
        onSubmit={handleSubmit}
        className="w-full max-w-[400px] bg-surface border border-borders p-8"
      >
        <div className="text-center mb-8">
          <h1 className="font-mono font-bold text-[32px] text-primary tracking-widest uppercase">
            SYS_DIR
          </h1>
          <p className="font-mono text-[13px] text-muted uppercase tracking-wider mt-1">
            {isSetup ? 'INITIALIZE NODE' : 'SECURE GATEWAY'}
          </p>
        </div>

        <div className="space-y-5">
          <div>
            <label className="block font-mono text-[13px] text-muted uppercase tracking-wider mb-2">
              USERNAME
            </label>
            <input
              type="text"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              className="w-full h-12 bg-surface border border-borders text-text-main font-mono px-4 rounded-none focus:border-primary focus:outline-none transition-colors"
              autoComplete="username"
              autoFocus
              required
            />
          </div>

          <div>
            <label className="block font-mono text-[13px] text-muted uppercase tracking-wider mb-2">
              PASSWORD
            </label>
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              className="w-full h-12 bg-surface border border-borders text-text-main font-mono px-4 rounded-none focus:border-primary focus:outline-none transition-colors"
              autoComplete={isSetup ? 'new-password' : 'current-password'}
              required
            />
          </div>

          <button
            type="submit"
            disabled={submitting}
            className="w-full h-12 bg-primary text-background font-mono font-bold text-[14px] uppercase tracking-widest flex items-center justify-center gap-2 transition-all hover:-translate-x-0.5 hover:-translate-y-0.5 hover:shadow-[4px_4px_0px_#F2F2F2] disabled:opacity-50 disabled:hover:translate-x-0 disabled:hover:translate-y-0 disabled:hover:shadow-none"
          >
            <LogIn width={18} height={18} strokeWidth={2} />
            {isSetup ? 'INITIALIZE_NODE' : 'ACCESS_NODE'}
          </button>

          {error && (
            <p className="font-mono text-[13px] text-primary uppercase text-center">
              {error}
            </p>
          )}
        </div>
      </form>
    </div>
  )
}
