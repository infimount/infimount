import { useMemo } from "react";
import CodeMirror from "@uiw/react-codemirror";
import { json } from "@codemirror/lang-json";
import { EditorState } from "@codemirror/state";
import { lintGutter, linter, type Diagnostic } from "@codemirror/lint";
import { keymap, EditorView } from "@codemirror/view";
import { indentWithTab } from "@codemirror/commands";

interface JsonCodeEditorProps {
  value: string;
  onChange: (value: string) => void;
  readOnly?: boolean;
  minHeight?: string;
}

const jsonTheme = EditorView.theme({
  "&": {
    backgroundColor: "transparent",
    color: "hsl(var(--card-foreground))",
    fontSize: "12px",
    lineHeight: "1.65",
  },
  ".cm-scroller": {
    fontFamily:
      '"SFMono-Regular", "JetBrains Mono", "Fira Code", "Ubuntu Mono", "Consolas", monospace',
  },
  ".cm-content": {
    padding: "14px 0",
    caretColor: "hsl(var(--foreground))",
  },
  ".cm-line": {
    padding: "0 14px 0 8px",
  },
  ".cm-gutters": {
    backgroundColor: "transparent",
    color: "hsl(var(--muted-foreground))",
    border: "none",
    paddingRight: "6px",
  },
  ".cm-activeLine": {
    backgroundColor: "hsl(var(--sidebar-accent) / 0.38)",
  },
  ".cm-activeLineGutter": {
    backgroundColor: "transparent",
    color: "hsl(var(--foreground))",
  },
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground, ::selection": {
    backgroundColor: "hsl(var(--accent) / 0.55) !important",
  },
  "&.cm-focused": {
    outline: "none",
  },
  ".cm-cursor, .cm-dropCursor": {
    borderLeftColor: "hsl(var(--foreground))",
  },
  ".cm-tooltip": {
    border: "1px solid hsl(var(--border))",
    backgroundColor: "hsl(var(--popover))",
    color: "hsl(var(--popover-foreground))",
  },
  ".cm-panels": {
    backgroundColor: "transparent",
    color: "hsl(var(--muted-foreground))",
  },
  ".cm-lintPoint-error": {
    borderBottomColor: "hsl(var(--destructive))",
  },
  ".cm-diagnosticText": {
    color: "hsl(var(--destructive))",
  },
});

function jsonDiagnosticsLinter() {
  return linter((view) => {
    try {
      JSON.parse(view.state.doc.toString());
      return [];
    } catch (error) {
      const message = error instanceof Error ? error.message : "Invalid JSON";
      const from = extractJsonErrorOffset(message);
      const diagnostics: Diagnostic[] = [
        {
          from,
          to: Math.min(from + 1, view.state.doc.length),
          severity: "error",
          message,
        },
      ];
      return diagnostics;
    }
  });
}

function extractJsonErrorOffset(message: string): number {
  const match = message.match(/position (\d+)/i);
  if (!match) return 0;
  const value = Number.parseInt(match[1] ?? "0", 10);
  return Number.isFinite(value) && value >= 0 ? value : 0;
}

export function JsonCodeEditor({
  value,
  onChange,
  readOnly = false,
  minHeight = "320px",
}: JsonCodeEditorProps) {
  const extensions = useMemo(
    () => [
      json(),
      jsonTheme,
      lintGutter(),
      jsonDiagnosticsLinter(),
      keymap.of([indentWithTab]),
      EditorView.lineWrapping,
      EditorState.readOnly.of(readOnly),
    ],
    [readOnly],
  );

  return (
    <div className="infimount-json-editor" style={{ minHeight }}>
      <CodeMirror
        value={value}
        height={minHeight}
        basicSetup={{
          highlightActiveLine: true,
          highlightActiveLineGutter: true,
          foldGutter: true,
          autocompletion: false,
          searchKeymap: true,
        }}
        editable={!readOnly}
        extensions={extensions}
        onChange={onChange}
      />
    </div>
  );
}
