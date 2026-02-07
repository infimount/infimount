import React from "react";

const ZOOM_STORAGE_KEY = "infimount.zoom";
const DEFAULT_ZOOM = 1;
const MIN_ZOOM = 0.5;
const MAX_ZOOM = 2;
const ZOOM_STEP = 0.1;

const clamp = (value: number, min: number, max: number) =>
  Math.min(max, Math.max(min, value));

const round2 = (value: number) => Math.round(value * 100) / 100;

const normalizeZoom = (value: number) =>
  clamp(round2(value), MIN_ZOOM, MAX_ZOOM);

type AppZoomContextValue = {
  zoom: number;
  setZoom: React.Dispatch<React.SetStateAction<number>>;
  zoomIn: () => void;
  zoomOut: () => void;
  resetZoom: () => void;
};

const AppZoomContext = React.createContext<AppZoomContextValue | null>(null);

const isInZoomRegion = (target: EventTarget | null) => {
  if (!target || !(target instanceof HTMLElement)) return false;
  return !!target.closest('[data-infimount-zoom-region="true"]');
};

export function AppZoomProvider({ children }: { children: React.ReactNode }) {
  const [zoom, setZoom] = React.useState<number>(() => {
    if (typeof window === "undefined") return DEFAULT_ZOOM;
    const raw = window.localStorage.getItem(ZOOM_STORAGE_KEY);
    const parsed = raw ? Number(raw) : Number.NaN;
    return Number.isFinite(parsed) ? normalizeZoom(parsed) : DEFAULT_ZOOM;
  });

  React.useEffect(() => {
    if (typeof window === "undefined") return;
    window.localStorage.setItem(ZOOM_STORAGE_KEY, String(zoom));
  }, [zoom]);

  const zoomIn = React.useCallback(() => {
    setZoom((prev) => normalizeZoom(prev + ZOOM_STEP));
  }, []);

  const zoomOut = React.useCallback(() => {
    setZoom((prev) => normalizeZoom(prev - ZOOM_STEP));
  }, []);

  const resetZoom = React.useCallback(() => {
    setZoom(DEFAULT_ZOOM);
  }, []);

  React.useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return;
      if (!(event.ctrlKey || event.metaKey) || event.altKey) return;

      const key = event.key;
      const code = event.code;

      const isZoomInKey =
        key === "+" ||
        key === "=" ||
        key === "Add" ||
        key === "Plus" ||
        code === "NumpadAdd" ||
        code === "Equal";

      const isZoomOutKey =
        key === "-" ||
        key === "_" ||
        key === "Subtract" ||
        key === "Minus" ||
        code === "NumpadSubtract" ||
        code === "Minus";

      const isResetKey =
        key === "0" ||
        code === "Digit0" ||
        code === "Numpad0";

      if (!isZoomInKey && !isZoomOutKey && !isResetKey) return;

      event.preventDefault();
      if (isResetKey) resetZoom();
      else if (isZoomInKey) zoomIn();
      else if (isZoomOutKey) zoomOut();
    };

    // Capture ensures we still see the shortcut even if a child stops propagation.
    window.addEventListener("keydown", onKeyDown, { capture: true });
    return () =>
      window.removeEventListener("keydown", onKeyDown, { capture: true } as any);
  }, [resetZoom, zoomIn, zoomOut]);

  React.useEffect(() => {
    const onWheel = (event: WheelEvent) => {
      if (event.defaultPrevented) return;
      if (!(event.ctrlKey || event.metaKey)) return;

      // Never allow browser/global zoom: we handle zoom ourselves.
      event.preventDefault();

      // Only zoom when the gesture happens over the main content region.
      if (!isInZoomRegion(event.target)) return;

      const magnitude = Math.abs(event.deltaY);
      const step = magnitude < 50 ? 0.05 : ZOOM_STEP;
      const direction = event.deltaY < 0 ? 1 : -1;
      setZoom((prev) => normalizeZoom(prev + direction * step));
    };

    window.addEventListener("wheel", onWheel, { passive: false, capture: true });
    return () =>
      window.removeEventListener("wheel", onWheel, { capture: true } as any);
  }, []);

  const value = React.useMemo<AppZoomContextValue>(
    () => ({ zoom, setZoom, zoomIn, zoomOut, resetZoom }),
    [resetZoom, zoom, zoomIn, zoomOut],
  );

  return <AppZoomContext.Provider value={value}>{children}</AppZoomContext.Provider>;
}

export function useAppZoom() {
  const ctx = React.useContext(AppZoomContext);
  if (!ctx) {
    throw new Error("useAppZoom must be used within AppZoomProvider");
  }
  return ctx;
}
