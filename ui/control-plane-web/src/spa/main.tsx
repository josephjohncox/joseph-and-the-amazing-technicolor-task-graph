import React from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import "@xyflow/react/dist/style.css";
import "@chatscope/chat-ui-kit-styles/dist/default/styles.min.css";
import "./styles.css";
import { App } from "./App";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      refetchInterval: 10_000,
      retry: 1,
      staleTime: 4_000,
    },
  },
});

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>
  </React.StrictMode>,
);
