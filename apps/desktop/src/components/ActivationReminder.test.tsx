import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ActivationReminder } from "./ActivationReminder";

describe("ActivationReminder", () => {
  it("continues setup when requested", () => {
    const onFinishSetup = vi.fn();

    render(<ActivationReminder onFinishSetup={onFinishSetup} />);

    fireEvent.click(screen.getByRole("button", { name: "Finish setup" }));

    expect(onFinishSetup).toHaveBeenCalledTimes(1);
  });

  it("dismisses only for the current component lifetime", () => {
    const { unmount } = render(
      <ActivationReminder onFinishSetup={() => undefined} />,
    );

    fireEvent.click(
      screen.getByRole("button", { name: "Dismiss activation reminder" }),
    );

    expect(
      screen.queryByText(
        "Activation is incomplete. Agent access remains unverified.",
      ),
    ).not.toBeInTheDocument();

    unmount();

    render(<ActivationReminder onFinishSetup={() => undefined} />);

    expect(
      screen.getByText(
        "Activation is incomplete. Agent access remains unverified.",
      ),
    ).toBeInTheDocument();
  });
});
