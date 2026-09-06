import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { DictateHome } from "./DictateHome";
import { api } from "../services/tauriApi";

async function home() {
  const settings = await api.getSettings();
  vi.spyOn(api, "getHistory").mockResolvedValue([]);
  return render(
    <DictateHome
      settings={settings}
      appState="READY"
      modelStatus={null}
      transcript={{
        full_text: "Hello world",
        committed_prefix: "",
        mutable_suffix: "",
        language: "en",
        audio_level: 0,
        stage: "",
      }}
      latencyMetrics={null}
      latencyPercentiles={null}
      onStartRecording={vi.fn()}
      onStopRecording={vi.fn()}
      onUpdateSettings={vi.fn()}
      onOpenHistory={vi.fn()}
    />,
  );
}

describe("transcript actions", () => {
  it("reports clipboard failures without showing success", async () => {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: vi.fn().mockRejectedValue(new Error("denied")) },
    });
    await home();
    fireEvent.click(screen.getByRole("button", { name: "Copy" }));
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("Could not copy"));
    expect(screen.queryByText("Copied")).not.toBeInTheDocument();
  });
});
