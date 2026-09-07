#!/usr/bin/env node
import fs from "node:fs";

const path = "apps/desktop/src/components/FileBrowser.tsx";
let source = fs.readFileSync(path, "utf8");

function replaceOnce(from, to, label) {
  if (source.includes(to)) return;
  const first = source.indexOf(from);
  if (first < 0 || source.indexOf(from, first + from.length) >= 0) {
    throw new Error(`Agent Task UI patch anchor is missing or ambiguous: ${label}`);
  }
  source = source.replace(from, to);
}

replaceOnce(
  "  Trash2,\n} from \"lucide-react\";",
  "  Trash2,\n  Bot,\n} from \"lucide-react\";",
  "Bot icon",
);

replaceOnce(
  'import { TransferQueuePanel } from "./TransferQueuePanel";\n',
  'import { TransferQueuePanel } from "./TransferQueuePanel";\nimport { AgentTaskPrepareDialog } from "./AgentTaskPrepareDialog";\n',
  "AgentTaskPrepareDialog import",
);

replaceOnce(
  '  const [selectedFiles, setSelectedFiles] = useState<Set<string>>(new Set());\n',
  '  const [selectedFiles, setSelectedFiles] = useState<Set<string>>(new Set());\n  const [isAgentTaskDialogOpen, setIsAgentTaskDialogOpen] = useState(false);\n',
  "dialog state",
);

replaceOnce(
  '  const normalizedSearchQuery = searchQuery.toLowerCase();\n',
  '  const selectedAgentTaskPaths = useMemo(() => Array.from(selectedFiles), [selectedFiles]);\n  const normalizedSearchQuery = searchQuery.toLowerCase();\n',
  "memoized selected paths",
);

replaceOnce(
  '                <Button\n                  type="button"\n                  size="icon"\n                  variant="ghost"\n                  onClick={toggleBookmark}',
  '                {selectedFiles.size > 0 ? (\n                  <Button\n                    type="button"\n                    size="sm"\n                    variant="outline"\n                    onClick={() => setIsAgentTaskDialogOpen(true)}\n                    className="h-8 gap-1.5 px-2.5 text-xs"\n                    title="Prepare the selected items for an agent"\n                  >\n                    <Bot className="h-3.5 w-3.5" />\n                    Use with agent\n                  </Button>\n                ) : null}\n                <Button\n                  type="button"\n                  size="icon"\n                  variant="ghost"\n                  onClick={toggleBookmark}',
  "toolbar action",
);

replaceOnce(
  '  return (\n    <>\n      <div\n        className="relative flex h-full bg-background"',
  '  return (\n    <>\n      <AgentTaskPrepareDialog\n        open={isAgentTaskDialogOpen}\n        onOpenChange={setIsAgentTaskDialogOpen}\n        sourceId={sourceId}\n        storageName={storageName}\n        selectedPaths={selectedAgentTaskPaths}\n        onPrepared={(result) => {\n          toast({\n            title: "Agent Task prepared",\n            description: `${result.preparedFiles} file${result.preparedFiles === 1 ? "" : "s"} ready in ${result.taskRoot}.`,\n          });\n          clearSelection();\n        }}\n      />\n      <div\n        className="relative flex h-full bg-background"',
  "dialog mount",
);

fs.writeFileSync(path, source);
