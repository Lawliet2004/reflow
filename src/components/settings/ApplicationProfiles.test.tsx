import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { api } from "../../services/tauriApi";
import { ApplicationProfiles } from "./ApplicationProfiles";

it("keeps an application profile draft when saving fails and retries without overwriting global terms", async () => {
  const settings = await api.getSettings();
  const save = vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true);
  render(<ApplicationProfiles settings={settings} onUpdateSettings={save} />);
  fireEvent.change(screen.getByLabelText("Application process"), { target: { value: "code.exe" } });
  fireEvent.change(screen.getByLabelText("Application dictionary"), {
    target: { value: "re flow=Reflow" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Add application profile" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("not saved");
  expect(screen.getByLabelText("Application process")).toHaveValue("code.exe");
  fireEvent.click(screen.getByRole("button", { name: "Add application profile" }));
  await waitFor(() => expect(screen.getByLabelText("Application process")).toHaveValue(""));
  expect(save.mock.lastCall?.[0]).toMatchObject({
    application_profiles: [
      {
        process: "code.exe",
        dictionary_terms: [{ term: "re flow", preferred_spelling: "Reflow" }],
      },
    ],
  });
  expect(save.mock.lastCall?.[0]).not.toHaveProperty("dictionary_terms");
});
