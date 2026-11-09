import { invoke } from "@tauri-apps/api/core";

/** Credential values never cross the native boundary in the return direction. */
export interface HostedProviderCredentialStatus {
  provider: "openai_agents_api";
  nativeStoreAvailable: boolean;
  configured: boolean;
  verified: false;
}

const GENERIC_ERROR = "The hosted provider credential operation failed. Check native OS secret storage.";

export async function getHostedProviderCredentialStatus(): Promise<HostedProviderCredentialStatus> {
  try {
    return await invoke<HostedProviderCredentialStatus>("hosted_provider_credential_status");
  } catch {
    throw new Error(GENERIC_ERROR);
  }
}

export async function saveHostedProviderCredential(apiKey: string): Promise<HostedProviderCredentialStatus> {
  try {
    return await invoke<HostedProviderCredentialStatus>("save_hosted_provider_credential", {
      request: { apiKey },
    });
  } catch {
    // Never interpolate or log the Tauri error. It may originate from an
    // OS keyring backend with confidential information in its message.
    throw new Error(GENERIC_ERROR);
  }
}

export async function removeHostedProviderCredential(): Promise<HostedProviderCredentialStatus> {
  try {
    return await invoke<HostedProviderCredentialStatus>("remove_hosted_provider_credential");
  } catch {
    throw new Error(GENERIC_ERROR);
  }
}
