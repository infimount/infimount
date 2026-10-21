import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  cancelClaudePilot,
  claudePilotBuildInfo,
  runClaudePilot,
  type ClaudePilotOutput,
} from "@/lib/agentTasks";

interface ClaudePilotPanelProps {
  workspaceId: string;
  taskId: string;
  workspaceMcpExposed: boolean;
  onRunningChange?: (running: boolean) => void;
}

/**
 * Never shown in default/stable builds. Even an opt-in pilot requires a
 * separately confirmed human action after task preparation. Only A2 summary
 * evidence is displayed, not CLI logs or task contents.
 */
export function ClaudePilotPanel({
  workspaceId,
  taskId,
  workspaceMcpExposed,
  onRunningChange,
}: ClaudePilotPanelProps) {
  const [available, setAvailable] = useState(false);
  const [executablePath, setExecutablePath] = useState("");
  const [consent, setConsent] = useState(false);
  const [running, setRunning] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [outcome, setOutcome] = useState<ClaudePilotOutput | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void claudePilotBuildInfo().then((info) => {
      if (active) setAvailable(info.enabled && info.platformSupported);
    });
    return () => { active = false; };
  }, []);

  async function execute() {
    if (!workspaceMcpExposed || !consent || !executablePath.trim() || running) return;
    setError(null);
    setOutcome(null);
    setRunning(true);
    onRunningChange?.(true);
    try {
      const result = await runClaudePilot({
        workspaceId,
        taskId,
        executablePath: executablePath.trim(),
        confirmExternalInference: true,
      });
      setOutcome(result);
    } catch (value) {
      setError(value instanceof Error ? value.message : "Claude pilot failed.");
    } finally {
      setRunning(false);
      setCancelling(false);
      onRunningChange?.(false);
    }
  }

  async function cancel() {
    if (!running || cancelling) return;
    setCancelling(true);
    const accepted = await cancelClaudePilot({ workspaceId, taskId });
    if (!accepted) {
      setError("Cancellation request was not accepted; execution may have already finished.");
      setCancelling(false);
    }
  }

  if (!available) return null;
  return (
    <section className="space-y-3 rounded-lg border border-amber-500/40 p-3" data-testid="claude-pilot-panel">
      <div>
        <div className="text-sm font-medium">Experimental Claude Code qualification</div>
        <p className="mt-1 text-xs text-muted-foreground">
          Opt-in local pilot build only. Claude will receive only the prepared Agent Task
          through Infimount's scoped MCP server. Its content may be sent to the configured
          external Claude inference service. Outputs still require separate review and
          explicit publication.
        </p>
      </div>
      <div className="space-y-1">
        <label className="text-xs font-medium" htmlFor="claude-pilot-executable">
          Absolute Claude Code CLI path
        </label>
        <Input
          id="claude-pilot-executable"
          placeholder="/home/user/.local/share/claude/versions/..."
          value={executablePath}
          onChange={(event) => setExecutablePath(event.target.value)}
          disabled={running}
        />
      </div>
      <label className="flex items-start gap-2 text-xs">
        <input
          type="checkbox"
          checked={consent}
          disabled={running}
          onChange={(event) => setConsent(event.target.checked)}
          aria-label="I approve sending prepared task data to Claude for this pilot"
        />
        <span>
          I approve sending this prepared task's content to my configured Claude provider
          for this one qualification run.
        </span>
      </label>
      {!workspaceMcpExposed ? (
        <p className="text-xs text-amber-700 dark:text-amber-300">
          Enable the Agent Workspace's existing MCP scope before starting the pilot.
        </p>
      ) : null}
      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => void execute()}
          disabled={running || !consent || !workspaceMcpExposed || !executablePath.trim()}
        >
          {running ? "Running Claude pilot…" : "Run Claude pilot"}
        </Button>
        {running ? (
          <Button type="button" size="sm" variant="outline" onClick={() => void cancel()} disabled={cancelling}>
            {cancelling ? "Stopping…" : "Stop pilot"}
          </Button>
        ) : null}
      </div>
      {error ? <p role="alert" className="text-xs text-destructive">{error}</p> : null}
      {outcome ? (
        <div className="text-xs" data-testid="claude-pilot-outcome">
          Claude pilot: <strong>{outcome.status}</strong>; verified output files:{" "}
          {outcome.fileCount}; bytes: {outcome.totalBytes}.
          {outcome.errorCode ? <> Failure code: {outcome.errorCode}.</> : null}
          <p className="mt-1 text-muted-foreground">
            Execution ID: <code>{outcome.executionId}</code>. Use the independent review panel
            for output inspection; successful execution does not approve publication.
          </p>
        </div>
      ) : null}
    </section>
  );
}
