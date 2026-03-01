import "@testing-library/jest-dom";
import { vi } from "vitest";

// Mock ResizeObserver for virtualizer
class ResizeObserverMock {
    observe = vi.fn();
    unobserve = vi.fn();
    disconnect = vi.fn();
}

global.ResizeObserver = ResizeObserverMock as unknown as typeof ResizeObserver;

// Override localStorage — jsdom provides a broken implementation in some setups
const store: Record<string, string> = {};
Object.defineProperty(window, "localStorage", {
    value: {
        getItem: (key: string) => store[key] ?? null,
        setItem: (key: string, value: string) => {
            store[key] = value;
        },
        removeItem: (key: string) => {
            delete store[key];
        },
        clear: () => {
            for (const k of Object.keys(store)) delete store[k];
        },
        get length() {
            return Object.keys(store).length;
        },
        key: (index: number) => Object.keys(store)[index] ?? null,
    } as Storage,
    writable: true,
});

