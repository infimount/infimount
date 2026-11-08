# Infimount v0.8.2 C2.7: desktop reviewed remote-export preflight

Status: **desktop metadata review only**, no live execution or trusted
human export approval. The primary product remains local-first browsing.

The selected Agent Task now has an optional collapsed **Review remote sandbox
export** panel. A user must write a *separate* reviewed and redacted title,
objective and explicit output filenames. The original private TASK.md is not
automatically copied into the provider instruction field.

The new Tauri `preview_hosted_task_export` command:

1. Reuses the workspace-bound Agent Task output-review context, requiring
   a valid workspace/task binding, current storage namespace, Local Filesystem
   storage and a supported task manifest.
2. Projects the reviewed brief and the *declared* prepared manifest entries
   through the existing provider-neutral `build_sandbox_task_bundle`.
3. Rejects an empty requested-output list and inputs beyond the first hosted
   inline pilot's 32-file, 8 MiB aggregate and 5 MiB per-file limits.
4. Returns only reviewed text, task-relative input names, manifest-declared
   byte counts and SHA-256 hashes, selected deliverables and a
   domain-separated **metadata** digest. No raw input bytes, source storage
   IDs, original private title, credentials or host paths are included.
5. Presents provider residency and retention caveats, explicit disabled
   sandbox network posture, and the current $1 synthetic pilot session
   ceiling in the UI.

**This is not the export authorization boundary.** The command does not
read and rehash prepared input bytes, upload them, call OpenAI, configure an
API key, spend money or approve publication. A future authenticated,
trusted user-facing transfer must re-read and rehash every prepared input,
obtain **separate explicit provider export and spend consent**, and bind that
approval to the exact byte hashes and task scope under the appropriate
workspace locks and OS no-follow confinement. Browser and publication
invariants, including v0.8.1 stability, stay unchanged.

The managed Agents API currently stores session data only in the United
States and does not support Zero Data Retention. `network.access: "disabled"`
controls sandbox egress, not the provider's storage region or retention.

Official data controls: https://help.openai.com/en/articles/20001551-agents-api-beta-faq
Official hosted limits: https://developers.openai.com/api/docs/guides/agents-api/environments/files
