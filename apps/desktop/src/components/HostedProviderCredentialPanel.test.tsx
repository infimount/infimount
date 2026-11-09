import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { HostedProviderCredentialPanel } from "./HostedProviderCredentialPanel";
import {
  getHostedProviderCredentialStatus,
  removeHostedProviderCredential,
  saveHostedProviderCredential,
} from "@/lib/hostedProviderCredentials";

vi.mock("@/lib/hostedProviderCredentials", () => ({
  getHostedProviderCredentialStatus: vi.fn(),
  removeHostedProviderCredential: vi.fn(),
  saveHostedProviderCredential: vi.fn(),
}));

const missing = {
  provider: "openai_agents_api" as const,
  nativeStoreAvailable: true,
  configured: false,
  verified: false as const,
};
const configured = { ...missing, configured: true };

describe("HostedProviderCredentialPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(getHostedProviderCredentialStatus).mockResolvedValue(missing);
    vi.mocked(saveHostedProviderCredential).mockResolvedValue(configured);
    vi.mocked(removeHostedProviderCredential).mockResolvedValue(missing);
  });

  it("stores an entered project key without displaying or returning it", async () => {
    render(<HostedProviderCredentialPanel />);
    expect(await screen.findByText(/No hosted provider credential/i)).toBeInTheDocument();
    const key = "sk-proj-test-secret-value-1234";
    fireEvent.change(screen.getByLabelText("Hosted provider API key"), { target: { value: key } });
    fireEvent.click(screen.getByRole("button", { name: "Save in keyring" }));
    await waitFor(() => {
      expect(saveHostedProviderCredential).toHaveBeenCalledWith(key);
    });
    expect(await screen.findByText(/Credential is stored locally/i)).toBeInTheDocument();
    expect(screen.getByLabelText("Hosted provider API key")).toHaveValue("");
    expect(screen.getByTestId("hosted-provider-credential-panel").outerHTML).not.toContain(key);
    expect(screen.queryByRole("button", { name: /run|execute|upload/i })).not.toBeInTheDocument();
  });

  it("requires confirmation before deleting an existing key", async () => {
    vi.mocked(getHostedProviderCredentialStatus).mockResolvedValueOnce(configured);
    render(<HostedProviderCredentialPanel />);
    fireEvent.click(await screen.findByRole("button", { name: "Remove stored key" }));
    expect(removeHostedProviderCredential).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Confirm key removal" }));
    await waitFor(() => expect(removeHostedProviderCredential).toHaveBeenCalledTimes(1));
    expect(await screen.findByText(/No hosted provider credential/i)).toBeInTheDocument();
  });

  it("refuses to save without a native keyring and does not use plaintext", async () => {
    vi.mocked(getHostedProviderCredentialStatus).mockResolvedValueOnce({
      ...missing,
      nativeStoreAvailable: false,
    });
    render(<HostedProviderCredentialPanel />);
    expect(await screen.findByText(/No plaintext fallback/i)).toBeInTheDocument();
    expect(screen.queryByLabelText("Hosted provider API key")).not.toBeInTheDocument();
    expect(saveHostedProviderCredential).not.toHaveBeenCalled();
  });

  it("never renders an unsafe error message containing the credential", async () => {
    vi.mocked(saveHostedProviderCredential).mockRejectedValueOnce(
      new Error("sk-proj-must-not-leak native keyring failure"),
    );
    render(<HostedProviderCredentialPanel />);
    await screen.findByText(/No hosted provider credential/i);
    fireEvent.change(screen.getByLabelText("Hosted provider API key"), {
      target: { value: "sk-proj-test-secret-value-1234" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save in keyring" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not save credential");
    expect(screen.queryByText(/sk-proj-must-not-leak/)).not.toBeInTheDocument();
    expect(screen.getByLabelText("Hosted provider API key")).toHaveValue("");
  });
});
