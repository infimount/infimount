import type { McpSettings } from "@/types/storage";

export const MAX_MCP_READ_BYTES = 2 * 1024 * 1024;
export const MAX_MCP_WRITE_BYTES = 4 * 1024 * 1024;

export function validMcpByteLimits(
  settings: Pick<McpSettings, "maxReadBytes" | "maxWriteBytes">,
): boolean {
  return Number.isInteger(settings.maxReadBytes)
    && settings.maxReadBytes >= 1
    && settings.maxReadBytes <= MAX_MCP_READ_BYTES
    && Number.isInteger(settings.maxWriteBytes)
    && settings.maxWriteBytes >= 1
    && settings.maxWriteBytes <= MAX_MCP_WRITE_BYTES;
}
