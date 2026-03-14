import "@testing-library/jest-dom";
import { vi } from "vitest";

// Mock ResizeObserver for virtualizer
class ResizeObserverMock {
    observe = vi.fn();
    unobserve = vi.fn();
    disconnect = vi.fn();
}

global.ResizeObserver = ResizeObserverMock as unknown as typeof ResizeObserver;

Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
    value: vi.fn(),
    writable: true,
});

Object.defineProperty(Range.prototype, "getBoundingClientRect", {
    value: () => ({
        x: 0,
        y: 0,
        width: 0,
        height: 0,
        top: 0,
        right: 0,
        bottom: 0,
        left: 0,
        toJSON: () => ({}),
    }),
    writable: true,
});

Object.defineProperty(Range.prototype, "getClientRects", {
    value: () =>
        ({
            length: 0,
            item: () => null,
            [Symbol.iterator]: function* () {
                yield* [];
            },
        }) as DOMRectList,
    writable: true,
});

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
