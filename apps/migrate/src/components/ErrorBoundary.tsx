import { Component } from "react";
import type { ErrorInfo, ReactNode } from "react";

interface State {
  error: Error | null;
}

export default class ErrorBoundary extends Component<{ children: ReactNode }, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("UI error:", error, info.componentStack);
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="empty" style={{ margin: 40 }}>
        <div className="empty__icon">⚠</div>
        <div className="empty__title">Beklenmeyen bir hata oluştu / Something went wrong</div>
        <div className="empty__text mono">{this.state.error.message}</div>
        <button type="button" className="btn btn--primary" onClick={() => this.setState({ error: null })}>
          Tekrar dene / Retry
        </button>
      </div>
    );
  }
}
