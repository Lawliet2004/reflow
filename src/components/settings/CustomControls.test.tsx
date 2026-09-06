import { fireEvent, render, screen } from "@testing-library/react";
import React from "react";
import { describe, expect, it, vi } from "vitest";
import { CustomControls } from "./CustomControls";

describe("CustomControls", () => {
  it("renders custom override fields and triggers change events", () => {
    const onChange = vi.fn();
    render(<CustomControls overrides={{}} onChangeOverrides={onChange} />);

    expect(screen.getByText("Manual Runtime Overrides")).toBeInTheDocument();
    expect(screen.getByText("ASR Compute Device")).toBeInTheDocument();
    expect(screen.getByText("Model Precision")).toBeInTheDocument();

    const selects = screen.getAllByRole("combobox");
    fireEvent.change(selects[0], { target: { value: "cuda" } });
    expect(onChange).toHaveBeenCalledWith({ asr_device: "cuda" });

    const resetBtn = screen.getByText("Reset to Defaults");
    fireEvent.click(resetBtn);
    expect(onChange).toHaveBeenCalledWith({});
  });
});
