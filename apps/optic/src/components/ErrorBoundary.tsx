import { Component } from "react";
import type { ErrorInfo, ReactNode } from "react";

interface Props {
  children: ReactNode;
}

interface State {
  hasError: boolean;
  error: Error | null;
}

export default class ErrorBoundary extends Component<Props, State> {
  state: State = { hasError: false, error: null };

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("ErrorBoundary caught:", error, info.componentStack);
  }

  handleReset = () => {
    this.setState({ hasError: false, error: null });
  };

  render() {
    if (this.state.hasError) {
      return (
        <div style={{
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          justifyContent: "center",
          minHeight: "100vh",
          background: "#1a1a2e",
          color: "#e0e0e0",
          padding: "2rem",
          textAlign: "center",
        }}>
          <h1 style={{ fontSize: "1.5rem", marginBottom: "1rem" }}>Something went wrong</h1>
          <p style={{ color: "#aaa", marginBottom: "1.5rem", maxWidth: 480 }}>
            {this.state.error?.message ?? "An unexpected error occurred."}
          </p>
          <button
            onClick={this.handleReset}
            style={{
              padding: "0.6rem 1.5rem",
              background: "#6c63ff",
              color: "#fff",
              border: "none",
              borderRadius: "6px",
              cursor: "pointer",
              fontSize: "0.95rem",
            }}
          >
            Try Again
          </button>
        </div>
      );
    }

    return this.props.children;
  }
}
