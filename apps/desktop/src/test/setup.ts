import "@testing-library/jest-dom";
import { vi } from "vitest";

// Mock ResizeObserver for virtualizer
global.ResizeObserver = vi.fn().mockImplementation(() => ({
    observe: vi.fn(),
    unobserve: vi.fn(),
    disconnect: vi.fn(),
}));
