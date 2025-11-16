import React, { useState } from "react";
import { SourceList, Source } from "./features/sources";
import { FileBrowser as Explorer } from "./features/explorer";
import { ToastContainer } from "./components/ToastContainer";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { useThemeStore } from "./store/themeStore";
import { Button } from "./components/Button";

export const App: React.FC = () => {
  const [selectedSource, setSelectedSource] = useState<Source | null>(null);
  const { theme, toggleTheme } = useThemeStore();

  const handleSelectSource = (source: Source) => {
    setSelectedSource(source);
  };

  return (
    <ErrorBoundary>
      <div className="flex h-screen flex-col bg-background text-foreground">
        {/* Header */}
        <header className="border-b border-border/40 bg-background/95 backdrop-blur">
          <div className="mx-auto flex max-w-6xl items-center justify-between px-4 py-3">
            <div className="flex items-center gap-3">
              <div className="flex h-8 w-8 items-center justify-center rounded-md bg-primary/90 text-primary-foreground text-lg shadow-sm">
                ⬢
              </div>
              <div>
                <h1 className="text-xs font-semibold tracking-[0.18em] uppercase text-muted-foreground">
                  OpenHSB
                </h1>
                <p className="text-[11px] text-muted-foreground">
                  Hybrid Storage Browser
                </p>
              </div>
            </div>
            <Button
              variant="ghost"
              size="sm"
              onClick={toggleTheme}
              title={`Switch to ${theme === "dark" ? "light" : "dark"} theme`}
            >
              {theme === "dark" ? "☀️ Light" : "🌙 Dark"}
            </Button>
          </div>
        </header>

        {/* Main layout: sidebar + explorer + preview */}
        <main className="flex flex-1 justify-center overflow-hidden bg-background">
          <div className="flex h-full w-full max-w-6xl">
            {/* Sidebar: sources */}
            <aside className="flex w-72 flex-shrink-0 flex-col bg-card/90 border-r border-border/40">
              <div className="h-full px-4 py-3">
                <SourceList
                  onSelectSource={handleSelectSource}
                  selectedSourceId={selectedSource?.id ?? null}
                />
              </div>
            </aside>

            {/* Explorer */}
            <section className="flex min-w-0 flex-1 flex-col bg-card/95">
              <div className="h-full px-4 py-3">
                {selectedSource ? (
                  <Explorer
                    sourceId={selectedSource.id}
                    sourceName={selectedSource.name}
                  />
                ) : (
                  <div className="flex h-full items-center justify-center rounded-lg border border-dashed border-border/40 bg-background/40">
                    <div className="space-y-2 text-center">
                      <p className="text-sm font-medium text-muted-foreground">
                        Select a source from the left to browse files.
                      </p>
                    </div>
                  </div>
                )}
              </div>
            </section>

            {/* Preview placeholder */}
            <aside className="hidden w-80 flex-shrink-0 border-l border-border/40 bg-card/90 lg:flex">
              <div className="flex h-full w-full items-center justify-center px-4 py-3">
                <p className="text-xs text-muted-foreground">
                  File preview will appear here in a future version.
                </p>
              </div>
            </aside>
          </div>
        </main>

        {/* Footer */}
        <footer className="border-t border-border/40 bg-background/95 px-4 py-2 text-center text-[11px] text-muted-foreground">
          <p>OpenHSB v0.1.0 • Powered by Rust, React & OpenDAL</p>
        </footer>

        <ToastContainer />
      </div>
    </ErrorBoundary>
  );
};
