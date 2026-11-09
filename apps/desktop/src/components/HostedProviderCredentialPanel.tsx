import { useEffect, useState } from "react";
import { KeyRound, ShieldAlert } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  getHostedProviderCredentialStatus,
  removeHostedProviderCredential,
  saveHostedProviderCredential,
  type HostedProviderCredentialStatus,
} from "@/lib/hostedProviderCredentials";

export function HostedProviderCredentialPanel() {
  const [status, setStatus] = useState<HostedProviderCredentialStatus | null>(null);
  const [key, setKey] = useState("");
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [confirmRemoval, setConfirmRemoval] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let current = true;
    void getHostedProviderCredentialStatus()
      .then((result) => {
        if (current) setStatus(result);
      })
      .catch(() => {
        if (current) setError("Native provider credential status is unavailable.");
      })
      .finally(() => {
        if (current) setLoading(false);
      });
    return () => {
      current = false;
    };
  }, []);

  const save = async () => {
    if (busy || !status?.nativeStoreAvailable || !key) return;
    setBusy(true);
    setError(null);
    try {
      const result = await saveHostedProviderCredential(key);
      setStatus(result);
    } catch {
      // Never show potentially sensitive native keyring error messages.
      setError("Could not save credential in native OS secret storage.");
    } finally {
      setKey("");
      setBusy(false);
    }
  };

  const remove = async () => {
    if (busy || !status?.nativeStoreAvailable || !confirmRemoval) return;
    setBusy(true);
    setError(null);
    try {
      const result = await removeHostedProviderCredential();
      setStatus(result);
    } catch {
      setError("Could not remove credential from native OS secret storage.");
    } finally {
      setConfirmRemoval(false);
      setKey("");
      setBusy(false);
    }
  };

  return (
    <section className="mb-4 rounded-xl border bg-background p-4" data-testid="hosted-provider-credential-panel">
      <div className="flex items-center gap-2 text-sm font-medium">
        <KeyRound className="h-4 w-4 text-primary" />
        Optional hosted compute credentials
      </div>
      <p className="mt-1 text-xs text-muted-foreground">
        Store an OpenAI Agents API project key in the operating system keyring.
        This does not verify API access, upload task files, or start a paid session.
      </p>
      {loading ? (
        <p className="mt-3 text-xs text-muted-foreground">Checking native keyring…</p>
      ) : status && !status.nativeStoreAvailable ? (
        <div className="mt-3 flex items-center gap-2 text-xs text-destructive">
          <ShieldAlert className="h-4 w-4" />
          Native OS secret storage is unavailable. No plaintext fallback is permitted.
        </div>
      ) : status ? (
        <div className="mt-3 space-y-2">
          <p className="text-xs">
            {status.configured
              ? "Credential is stored locally, but provider access is not verified."
              : "No hosted provider credential is configured."}
          </p>
          <div className="flex flex-wrap items-end gap-2">
            <label className="min-w-56 flex-1 space-y-1 text-xs">
              <span>Project API key</span>
              <Input
                type="password"
                aria-label="Hosted provider API key"
                autoComplete="off"
                spellCheck={false}
                value={key}
                onChange={(event) => setKey(event.target.value)}
                placeholder={status.configured ? "Replace stored key" : "Paste project API key"}
                disabled={busy}
              />
            </label>
            <Button type="button" size="sm" variant="outline" disabled={busy || !key} onClick={() => void save()}>
              {busy ? "Updating…" : status.configured ? "Replace key" : "Save in keyring"}
            </Button>
          </div>
          {status.configured && !confirmRemoval ? (
            <Button size="sm" variant="ghost" disabled={busy} onClick={() => setConfirmRemoval(true)}>
              Remove stored key
            </Button>
          ) : null}
          {status.configured && confirmRemoval ? (
            <div className="flex flex-wrap gap-2">
              <Button size="sm" variant="destructive" disabled={busy} onClick={() => void remove()}>
                Confirm key removal
              </Button>
              <Button size="sm" variant="outline" disabled={busy} onClick={() => setConfirmRemoval(false)}>
                Cancel
              </Button>
            </div>
          ) : null}
        </div>
      ) : null}
      {error ? <p role="alert" className="mt-2 text-xs text-destructive">{error}</p> : null}
      <p className="mt-2 text-[11px] text-muted-foreground">
        A separate provider export and spending approval remains mandatory.
        The Agents API currently has U.S.-only residency and does not support Zero Data Retention.
      </p>
    </section>
  );
}
