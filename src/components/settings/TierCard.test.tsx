import { fireEvent, render, screen } from "@testing-library/react";
import React from "react";
import { describe, expect, it, vi } from "vitest";
import { TierCard } from "./TierCard";
import { INTELLIGENCE_TIERS } from "../../types";

describe("TierCard", () => {
  it("renders tier details and triggers selection", () => {
    const onSelect = vi.fn();
    const meta = INTELLIGENCE_TIERS.raw_verbatim;

    render(<TierCard metadata={meta} selected={true} recommended={true} onSelect={onSelect} />);

    // Read the label from the metadata rather than repeating it: this assertion
    // previously spelled out "Exact Voice", so renaming the tier for consistency
    // with the HUD and tray broke a test that was not about naming at all.
    expect(screen.getByText(meta.label)).toBeInTheDocument();
    expect(screen.getByText("Active")).toBeInTheDocument();
    expect(screen.getByText("Recommended")).toBeInTheDocument();

    const card = screen.getByRole("button");
    fireEvent.click(card);
    expect(onSelect).toHaveBeenCalledWith("raw_verbatim");
  });

  it("renders download button when not installed", () => {
    const onDownload = vi.fn();
    const meta = INTELLIGENCE_TIERS.smart_flow;

    render(
      <TierCard
        metadata={meta}
        selected={false}
        installed={false}
        onSelect={vi.fn()}
        onDownload={onDownload}
      />,
    );

    const downloadBtn = screen.getByText("Get Model");
    fireEvent.click(downloadBtn);
    expect(onDownload).toHaveBeenCalledWith("smart_flow");
  });
});
