import { create } from "zustand";
import type { Source } from "../../types/source";

interface SourceStore {
  sources: Source[];
  addSource: (source: Source) => void;
  removeSource: (sourceId: string) => void;
  updateSource: (sourceId: string, source: Partial<Source>) => void;
  setSources: (sources: Source[]) => void;
}

export const useSourceStore = create<SourceStore>((set) => ({
  sources: [],
  addSource: (source) =>
    set((state) => ({
      sources: [...state.sources, source],
    })),
  removeSource: (sourceId) =>
    set((state) => ({
      sources: state.sources.filter((s) => s.id !== sourceId),
    })),
  updateSource: (sourceId, updates) =>
    set((state) => ({
      sources: state.sources.map((s) =>
        s.id === sourceId ? { ...s, ...updates } : s
      ),
    })),
  setSources: (sources) => set({ sources }),
}));
