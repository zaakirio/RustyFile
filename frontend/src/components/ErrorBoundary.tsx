import { Component, type ReactNode } from 'react'

interface Props {
  children: ReactNode
}

interface State {
  hasError: boolean
  error: Error | null
}

export default class ErrorBoundary extends Component<Props, State> {
  constructor(props: Props) {
    super(props)
    this.state = { hasError: false, error: null }
  }

  static getDerivedStateFromError(error: Error) {
    return { hasError: true, error }
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error('ErrorBoundary caught:', error, info.componentStack)
  }

  render() {
    if (this.state.hasError) {
      return (
        <div className="flex items-center justify-center min-h-screen bg-background">
          <div className="font-mono text-center p-8 border border-borders bg-surface max-w-md">
            <h1 className="text-xl font-bold text-text-main uppercase tracking-wider mb-4">
              [ SYSTEM ERROR ]
            </h1>
            <p className="text-muted text-sm mb-6">
              {this.state.error?.message || 'An unexpected error occurred'}
            </p>
            <button
              onClick={() => window.location.reload()}
              className="px-4 py-2 border border-borders text-primary hover:bg-primary hover:text-background transition-colors font-mono text-sm uppercase tracking-wider"
            >
              [ RELOAD ]
            </button>
          </div>
        </div>
      )
    }

    return this.props.children
  }
}
