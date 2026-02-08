import React from "react";
import { Toaster } from "@/components/ui/toaster";
import { Toaster as Sonner } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { BrowserRouter, Routes, Route } from "react-router-dom";
import { ThemeProvider } from "next-themes";
import { IconThemeProvider } from "@/hooks/use-icon-theme";
import { FileClipboardProvider } from "@/hooks/use-file-clipboard";
import { AppZoomProvider } from "@/hooks/use-app-zoom";
import Index from "./pages/Index";
import NotFound from "./pages/NotFound";

const queryClient = new QueryClient();

export const App: React.FC = () => {
  React.useEffect(() => {
    const isExternalFileDrag = (dt: DataTransfer | null) => {
      if (!dt) return false;
      const types = Array.from(dt.types ?? []);
      if (types.includes("Files")) return true;
      if (dt.files && dt.files.length > 0) return true;
      const items = Array.from(dt.items ?? []);
      return items.some((item) => item.kind === "file");
    };

    const handleDragOver = (event: DragEvent) => {
      if (!isExternalFileDrag(event.dataTransfer)) return;
      event.preventDefault();
    };

    const handleDrop = (event: DragEvent) => {
      if (!isExternalFileDrag(event.dataTransfer)) return;
      event.preventDefault();
    };

    // Prevent the webview's default behavior of navigating to a dropped file.
    window.addEventListener("dragover", handleDragOver, true);
    window.addEventListener("drop", handleDrop, true);
    return () => {
      window.removeEventListener("dragover", handleDragOver, true);
      window.removeEventListener("drop", handleDrop, true);
    };
  }, []);

  return (
    <div className="h-full w-full bg-background text-foreground rounded-[12px] overflow-hidden">
      <QueryClientProvider client={queryClient}>
        <ThemeProvider attribute="class" defaultTheme="system" enableSystem>
          <IconThemeProvider>
            <FileClipboardProvider>
              <AppZoomProvider>
                <TooltipProvider>
                  <Toaster />
                  <Sonner />
                  <BrowserRouter>
                    <Routes>
                      <Route path="/" element={<Index />} />
                      {/* ADD ALL CUSTOM ROUTES ABOVE THE CATCH-ALL "*" ROUTE */}
                      <Route path="*" element={<NotFound />} />
                    </Routes>
                  </BrowserRouter>
                </TooltipProvider>
              </AppZoomProvider>
            </FileClipboardProvider>
          </IconThemeProvider>
        </ThemeProvider>
      </QueryClientProvider>
    </div>
  );
};
