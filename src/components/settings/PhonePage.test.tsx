import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { PhonePage } from "./PhonePage";
import { api } from "../../services/tauriApi";

it.each(["revoke", "permission", "rotate"])(
  "ignores an older status poll after a successful %s mutation",
  async (mutation) => {
    const settings = await api.getSettings();
    const status = {
      ...(await api.getApiStatus()),
      pairing_code: "OLD123",
      devices: [
        {
          id: "phone",
          name: "Test phone",
          created_at: "2026-09-30",
          permissions: { stream: true, history: false, injection: false },
        },
      ],
    };
    let finishPoll!: (value: typeof status) => void;
    const polling = vi
      .spyOn(api, "getApiStatus")
      .mockResolvedValueOnce(status)
      .mockImplementation(
        () =>
          new Promise((resolve) => {
            finishPoll = resolve;
          }),
      );
    vi.spyOn(api, "revokeApiDevice").mockResolvedValue(true);
    vi.spyOn(api, "setApiDevicePermissions").mockResolvedValue(true);
    vi.spyOn(api, "rotatePairingCode").mockResolvedValue({ ...status, pairing_code: "NEW456" });
    vi.useFakeTimers();
    render(<PhonePage settings={{ ...settings, api_enabled: true }} onUpdateSettings={vi.fn()} />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(screen.getByText("Test phone")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Code" }));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2500);
    });
    expect(polling).toHaveBeenCalledTimes(2);
    vi.useRealTimers();
    if (mutation === "revoke") {
      fireEvent.click(screen.getByRole("button", { name: "Revoke" }));
      await waitFor(() => expect(screen.queryByText("Test phone")).toBeNull());
    } else if (mutation === "permission") {
      fireEvent.click(screen.getByRole("checkbox", { name: "Read history for Test phone" }));
      await waitFor(() =>
        expect(screen.getByRole("checkbox", { name: "Read history for Test phone" })).toBeChecked(),
      );
    } else {
      fireEvent.click(screen.getByRole("button", { name: "Rotate" }));
      await screen.findByText("NEW456");
    }
    await act(async () => finishPoll(status));
    if (mutation === "revoke") expect(screen.queryByText("Test phone")).toBeNull();
    else if (mutation === "permission")
      expect(screen.getByRole("checkbox", { name: "Read history for Test phone" })).toBeChecked();
    else expect(screen.getByText("NEW456")).toBeInTheDocument();
  },
);

it("shows new phones with streaming enabled and sensitive permissions disabled", async () => {
  const settings = await api.getSettings();
  const status = await api.getApiStatus();
  vi.spyOn(api, "getApiStatus").mockResolvedValue({
    ...status,
    devices: [{ id: "phone", name: "Test phone", created_at: "2026-09-30" }],
  });
  render(<PhonePage settings={settings} onUpdateSettings={vi.fn()} />);
  expect(
    await screen.findByRole("checkbox", { name: "Stream audio for Test phone" }),
  ).toBeChecked();
  expect(screen.getByRole("checkbox", { name: "Read history for Test phone" })).not.toBeChecked();
  expect(
    screen.getByRole("checkbox", { name: "Paste on desktop for Test phone" }),
  ).not.toBeChecked();
});

it("keeps a failed revocation visible and reports that the phone remains paired", async () => {
  const settings = await api.getSettings();
  const status = await api.getApiStatus();
  vi.spyOn(api, "getApiStatus").mockResolvedValue({
    ...status,
    devices: [{ id: "phone", name: "Test phone", created_at: "2026-09-30" }],
  });
  vi.spyOn(api, "revokeApiDevice").mockRejectedValue(new Error("disk unavailable"));
  render(<PhonePage settings={settings} onUpdateSettings={vi.fn()} />);
  fireEvent.click(await screen.findByRole("button", { name: "Revoke" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("It is still paired");
  expect(screen.getByText("Test phone")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Revoke" })).toBeEnabled();
});

it("reports status failures instead of claiming that no phones are paired", async () => {
  const settings = await api.getSettings();
  vi.spyOn(api, "getApiStatus").mockRejectedValue(new Error("API unavailable"));
  render(<PhonePage settings={settings} onUpdateSettings={vi.fn()} />);
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not refresh phone status");
});
