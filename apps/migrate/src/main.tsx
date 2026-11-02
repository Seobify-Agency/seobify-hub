import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import ErrorBoundary from "./components/ErrorBoundary";
import { I18nProvider } from "./i18n";
import { ProjectProvider, ToastProvider } from "./lib/context";
import "./styles/globals.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary>
      <I18nProvider>
        <ToastProvider>
          <ProjectProvider>
            <App />
          </ProjectProvider>
        </ToastProvider>
      </I18nProvider>
    </ErrorBoundary>
  </React.StrictMode>,
);
