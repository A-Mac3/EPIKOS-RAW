import { Component, useEffect, useState, type ErrorInfo, type ReactNode } from "react";

interface Props {
  /** What this boundary guards, shown in the fallback ("the side panel"). */
  area: string;
  children: ReactNode;
}

interface State {
  error: Error | null;
}

/**
 * Catches render errors below it so one failing panel shows a recoverable message
 * instead of blanking the whole window. "Try again" re-mounts the children.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error(`[EPIKOS] ${this.props.area} crashed`, error, info.componentStack);
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="crash" role="alert">
        <strong>Something went wrong in {this.props.area}.</strong>
        <span className="hint">{this.state.error.message}</span>
        <span className="hint">Your edits are saved. The rest of the app keeps working.</span>
        <button type="button" className="btn" onClick={() => this.setState({ error: null })}>
          Try again
        </button>
      </div>
    );
  }
}

/**
 * Errors outside React rendering (event handlers, timers, rejected promises) would
 * otherwise vanish or leave the UI half-updated; show the latest one in a banner.
 */
export function GlobalErrorBanner() {
  const [message, setMessage] = useState<string | null>(null);
  useEffect(() => {
    const onError = (e: ErrorEvent) => setMessage(e.message || String(e.error));
    const onRejection = (e: PromiseRejectionEvent) => {
      const r = e.reason;
      setMessage(r instanceof Error ? r.message : String(r));
    };
    window.addEventListener("error", onError);
    window.addEventListener("unhandledrejection", onRejection);
    return () => {
      window.removeEventListener("error", onError);
      window.removeEventListener("unhandledrejection", onRejection);
    };
  }, []);
  if (!message) return null;
  return (
    <div className="error-banner" role="alert">
      <span>{message}</span>
      <button type="button" className="btn icon" aria-label="Dismiss" onClick={() => setMessage(null)}>
        ×
      </button>
    </div>
  );
}
