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
        <header className="border-b border-border bg-background/80 backdrop-blur supports-backdrop-blur:bg-background/60">
          <div className="mx-auto flex max-w-7xl items-center justify-between px-4 py-3">
            <div className="flex items-center gap-3">
              <div className="flex h-9 w-9 items-center justify-center rounded-lg bg-primary text-primary-foreground text-lg shadow-sm">
                ⬢
              </div>
              <div>
                <h1 className="text-sm font-semibold tracking-tight text-foreground">
                  OpenHSB
                </h1>
                <p className="text-xs text-muted-foreground">
                  Hybrid Storage Browser
                </p>
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Button
                variant="ghost"
                size="sm"
                onClick={toggleTheme}
                title={`Switch to ${theme === "dark" ? "light" : "dark"} theme`}
                className="h-8 w-8 p-0"
              >
                {theme === "dark" ? "☀️" : "🌙"}
              </Button>
            </div>
          </div>
        </header>

        {/* Main layout: sidebar + explorer + preview */}
        <main className="flex flex-1 overflow-hidden bg-muted/20">
          <div className="flex h-full w-full max-w-7xl mx-auto">
            {/* Sidebar: sources */}
            <aside className="w-64 flex-shrink-0 flex-col border-r border-border/60 bg-[hsl(var(--muted))]">
              <div className="h-full flex flex-col">
                <div className="p-3 border-b border-border/60">
                  <h2 className="text-xs font-semibold text-foreground tracking-wide uppercase mb-1">
                    Locations
                  </h2>
                  <p className="text-xs text-muted-foreground">
                    Connected storage sources
                  </p>
                </div>
                <div className="flex-1 overflow-y-auto p-3">
                  <SourceList
                    onSelectSource={handleSelectSource}
                    selectedSourceId={selectedSource?.id ?? null}
                  />
                </div>
              </div>
            </aside>

            {/* Explorer */}
            <section className="flex min-w-0 flex-1 flex-col bg-background">
              <div className="h-full flex flex-col">
                {selectedSource ? (
                  <Explorer
                    sourceId={selectedSource.id}
                    sourceName={selectedSource.name}
                  />
                ) : (
                  <div className="flex h-full items-center justify-center p-8">
                    <div className="max-w-md text-center space-y-4">
                      <div className="w-16 h-16 mx-auto rounded-full bg-muted flex items-center justify-center">
                        <span className="text-2xl">📂</span>
                      </div>
                      <h2 className="text-xl font-semibold text-foreground">
                        Welcome to OpenHSB
                      </h2>
                      <p className="text-muted-foreground">
                        Connect a storage source to start browsing your files across different locations in one unified interface.
                      </p>
                      <div className="pt-4">
                        <Button
                          onClick={() => document.querySelector('button[children*="Add Source"]')?.closest('button')?.click()}
                        >
                          Add Your First Source
                        </Button>
                      </div>
                    </div>
                  </div>
                )}
              </div>
            </section>

            {/* Preview placeholder - hidden by default, shown when file is selected */}
            <aside className="w-80 flex-shrink-0 border-l border-border bg-card hidden lg:flex lg:flex-col">
              <div className="p-4 border-b border-border">
                <h3 className="text-sm font-medium text-foreground">Preview</h3>
              </div>
              <div className="flex flex-1 flex-col p-4">
                <div className="flex h-full items-center justify-center text-center text-muted-foreground">
                  <p className="text-sm">
                    Select a file to preview its contents
                  </p>
                </div>
              </div>
            </aside>
          </div>
        </main>

        {/* Footer */}
        <footer className="border-t border-border bg-background/80 px-4 py-2">
          <div className="mx-auto max-w-7xl flex items-center justify-between">
            <p className="text-xs text-muted-foreground">
              OpenHSB v0.1.0 • Hybrid Storage Browser
            </p>
            <div className="flex items-center gap-4">
              <a
                href="https://github.com/openhsb/openhsb"
                target="_blank"
                rel="noopener noreferrer"
                className="text-xs text-muted-foreground hover:text-foreground transition-colors"
              >
                GitHub
              </a>
              <span className="text-xs text-muted-foreground">
                Powered by Rust & React
              </span>
            </div>
          </div>
        </footer>

        <ToastContainer />
      </div>
    </ErrorBoundary>
  );
};
