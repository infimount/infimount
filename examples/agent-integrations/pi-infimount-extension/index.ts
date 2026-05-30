import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type, type Static } from "typebox";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";

const PathParams = Type.Object({
  path: Type.String({ description: "Infimount MCP path, for example /StorageName/path/to/file.txt" }),
  session_id: Type.Optional(Type.String({ description: "Optional Infimount MCP scoped session id" })),
});

type PathInput = Static<typeof PathParams>;

const ReadFileParams = Type.Intersect([
  PathParams,
  Type.Object({
    max_bytes: Type.Optional(Type.Number({ minimum: 1, maximum: 2_097_152, default: 65536 })),
    as_text: Type.Optional(Type.Boolean({ default: true })),
  }),
]);

type ReadFileInput = Static<typeof ReadFileParams>;

const ListDirParams = Type.Intersect([
  PathParams,
  Type.Object({
    recursive: Type.Optional(Type.Boolean({ default: false })),
    limit: Type.Optional(Type.Number({ minimum: 1, maximum: 1000, default: 200 })),
  }),
]);

type ListDirInput = Static<typeof ListDirParams>;

const SearchParams = Type.Intersect([
  PathParams,
  Type.Object({
    pattern: Type.String({ description: "Substring to match in paths" }),
    max_results: Type.Optional(Type.Number({ minimum: 1, maximum: 2000, default: 200 })),
  }),
]);

type SearchInput = Static<typeof SearchParams>;

const DownloadLinkParams = Type.Intersect([
  PathParams,
  Type.Object({
    expires_seconds: Type.Optional(Type.Number({ minimum: 60, maximum: 86400, default: 900 })),
  }),
]);

type DownloadLinkInput = Static<typeof DownloadLinkParams>;

let clientPromise: Promise<Client> | undefined;

function parseArgs(): string[] {
  const raw = process.env.INFIMOUNT_MCP_ARGS;
  if (!raw) return ["--transport", "stdio"];
  try {
    const parsed = JSON.parse(raw);
    if (Array.isArray(parsed) && parsed.every((item) => typeof item === "string")) return parsed;
  } catch {
    // Fall through to shell-like whitespace splitting for quick local use.
  }
  return raw.split(/\s+/).filter(Boolean);
}

async function getClient(): Promise<Client> {
  if (!clientPromise) {
    clientPromise = (async () => {
      const transport = new StdioClientTransport({
        command: process.env.INFIMOUNT_MCP_COMMAND ?? "infimount_mcp",
        args: parseArgs(),
      });
      const client = new Client({ name: "pi-infimount-extension", version: "0.1.0" });
      await client.connect(transport);
      return client;
    })();
  }
  return clientPromise;
}

async function callInfimountTool(name: string, args: Record<string, unknown>) {
  const client = await getClient();
  const result = await client.callTool({ name, arguments: args });
  const content = Array.isArray(result.content) && result.content.length > 0
    ? result.content
    : [{ type: "text" as const, text: JSON.stringify(result, null, 2) }];
  return { content, details: result as Record<string, unknown> };
}

function withOptionalSession<T extends { session_id?: string }>(input: T): Record<string, unknown> {
  return Object.fromEntries(Object.entries(input).filter(([, value]) => value !== undefined));
}

export default function (pi: ExtensionAPI) {
  pi.registerCommand("infimount", {
    description: "Show Infimount MCP extension status and setup notes",
    handler: async (_args, ctx) => {
      ctx.ui.notify(
        "Infimount extension loaded. Use infimount_list_storages, infimount_list_dir, infimount_read_file, infimount_search_paths, or infimount_generate_download_link.",
        "info",
      );
    },
  });

  pi.registerTool({
    name: "infimount_list_storages",
    label: "Infimount: List Storages",
    description: "List Infimount storages with secrets masked. Only enabled and MCP-exposed storages are usable for filesystem tools.",
    promptSnippet: "List configured Infimount storages through the local MCP server.",
    promptGuidelines: [
      "Use infimount_list_storages before reading storage paths if the user has not named a storage.",
      "Do not ask for cloud credentials; use only storages the user exposed through Infimount.",
    ],
    parameters: Type.Object({}),
    async execute() {
      return callInfimountTool("list_storages", {});
    },
  });

  pi.registerTool({
    name: "infimount_list_dir",
    label: "Infimount: List Directory",
    description: "List an Infimount MCP directory path such as /StorageName/folder.",
    promptSnippet: "List files and folders from an Infimount-exposed storage path.",
    promptGuidelines: ["Use infimount_list_dir instead of shelling into cloud CLIs when the user asks to inspect exposed storage."],
    parameters: ListDirParams,
    async execute(_toolCallId, params: ListDirInput) {
      return callInfimountTool("list_dir", withOptionalSession(params));
    },
  });

  pi.registerTool({
    name: "infimount_read_file",
    label: "Infimount: Read File",
    description: "Read a file through Infimount MCP policy controls.",
    promptSnippet: "Read file content from an Infimount-exposed storage path.",
    promptGuidelines: ["Use infimount_read_file for agent storage reads; respect truncation and ask before reading large or sensitive paths."],
    parameters: ReadFileParams,
    async execute(_toolCallId, params: ReadFileInput) {
      return callInfimountTool("read_file", withOptionalSession(params));
    },
  });

  pi.registerTool({
    name: "infimount_search_paths",
    label: "Infimount: Search Paths",
    description: "Search path names below an Infimount MCP directory path.",
    promptSnippet: "Search exposed Infimount storage paths by substring.",
    parameters: SearchParams,
    async execute(_toolCallId, params: SearchInput) {
      return callInfimountTool("search_paths", withOptionalSession(params));
    },
  });

  pi.registerTool({
    name: "infimount_generate_download_link",
    label: "Infimount: Generate Download Link",
    description: "Generate a short-lived download link when the backend and policy allow presigned reads. Infimount may require user confirmation.",
    promptSnippet: "Request an Infimount-generated download link for exposed storage paths when explicitly needed.",
    promptGuidelines: ["Use infimount_generate_download_link only when the user asks for a link; do not generate links speculatively."],
    parameters: DownloadLinkParams,
    async execute(_toolCallId, params: DownloadLinkInput) {
      return callInfimountTool("generate_download_link", withOptionalSession(params));
    },
  });
}
