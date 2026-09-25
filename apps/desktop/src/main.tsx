import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";

async function start() {
  // Dev-only: `?mock` runs the UI in a normal browser against a fake engine.
  if (import.meta.env.DEV && new URLSearchParams(location.search).has("mock")) {
    const { installMockBackend } = await import("./dev/mockBackend");
    installMockBackend();
  }
  // Imported after the mock is installed so api.ts sees the (mock) Tauri runtime.
  const { default: App } = await import("./App");
  const { ErrorBoundary, GlobalErrorBanner } = await import("./components/ErrorBoundary");
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <ErrorBoundary area="EPIKOS RAW">
        <App />
      </ErrorBoundary>
      <GlobalErrorBanner />
    </StrictMode>,
  );
}

void start();
