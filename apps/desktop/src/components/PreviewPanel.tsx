import React, { useEffect, useState } from "react";
import { Entry } from "../lib/api";
import { readFile, writeFile } from "../lib/api";
import { Button } from "./Button";

export const PreviewPanel: React.FC<{
  entry: Entry | null | undefined;
  sourceId?: string;
}> = ({ entry, sourceId }) => {
  const [content, setContent] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setContent(null);
    setEditing(false);
    setError(null);
    if (!entry || entry.is_dir || !sourceId) return;
    setLoading(true);
    readFile(sourceId, entry.path)
      .then((data) => {
        try {
          const text = new TextDecoder().decode(data);
          setContent(text);
        } catch (err) {
          setError("Binary or non-text file - preview not available.");
        }
      })
      .catch((e) => setError((e && e.message) || String(e)))
      .finally(() => setLoading(false));
  }, [entry, sourceId]);

  const handleSave = async () => {
    if (!entry || !sourceId) return;
    try {
      const arr = new TextEncoder().encode(content || "");
      await writeFile(sourceId, entry.path, arr);
      setEditing(false);
    } catch (e: any) {
      setError(e?.message || String(e));
    }
  };

  if (!entry) {
    return (
      <div className="tile-card p-4">
        <div className="text-sm text-muted-foreground">No selection</div>
        <div className="mt-2 text-xs text-muted-foreground">Select a file to preview</div>
      </div>
    );
  }

  return (
    <div className="tile-card p-4 h-full">
      <div className="flex items-start justify-between">
        <div>
          <h3 className="font-semibold">{entry.name}</h3>
          <div className="text-xs text-muted-foreground">{entry.path}</div>
        </div>
        <div className="text-xs text-muted-foreground">{entry.is_dir ? "Folder" : "File"}</div>
      </div>

      <div className="mt-4">
        {loading ? (
          <div className="text-muted-foreground">Loading...</div>
        ) : error ? (
          <div className="text-sm text-destructive">{error}</div>
        ) : entry.is_dir ? (
          <div className="text-sm text-muted-foreground">Directory - preview not available.</div>
        ) : (
          <div>
            {!editing ? (
              <div>
                <pre className="max-h-[60vh] overflow-auto whitespace-pre-wrap break-words text-xs">{content}</pre>
                <div className="mt-3 flex gap-2">
                  <Button variant="outline" size="sm" onClick={() => setEditing(true)}>
                    Edit
                  </Button>
                </div>
              </div>
            ) : (
              <div>
                <textarea
                  className="w-full h-48 rounded-md border border-input bg-input px-3 py-2 text-sm text-foreground"
                  value={content ?? ""}
                  onChange={(e) => setContent(e.target.value)}
                />
                <div className="mt-2 flex gap-2">
                  <Button variant="default" size="sm" onClick={handleSave}>
                    Save
                  </Button>
                  <Button variant="ghost" size="sm" onClick={() => setEditing(false)}>
                    Cancel
                  </Button>
                </div>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
};

export default PreviewPanel;
