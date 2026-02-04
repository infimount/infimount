import "@testing-library/jest-dom";
import { vi } from "vitest";

// Mock ResizeObserver for virtualizer
class ResizeObserverMock {
    observe = vi.fn();
    unobserve = vi.fn();
    disconnect = vi.fn();

    // eslint-disable-next-line @typescript-eslint/no-useless-constructor
    constructor(_callback: ResizeObserverCallback) { }
}

global.ResizeObserver = ResizeObserverMock as unknown as typeof ResizeObserver;
