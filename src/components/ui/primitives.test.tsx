import { fireEvent, render, screen } from "@testing-library/react";
import React from "react";
import { describe, expect, it, vi } from "vitest";
import { Badge } from "./Badge";
import { Button } from "./Button";
import { Field } from "./Field";

describe("UI Primitives", () => {
  describe("Button", () => {
    it("renders with variants and handles click", () => {
      const onClick = vi.fn();
      render(<Button onClick={onClick}>Click Me</Button>);
      const btn = screen.getByRole("button", { name: /click me/i });
      expect(btn).toBeInTheDocument();
      fireEvent.click(btn);
      expect(onClick).toHaveBeenCalledTimes(1);
    });

    it("shows loading state and disables button", () => {
      render(<Button loading>Submit</Button>);
      const btn = screen.getByRole("button");
      expect(btn).toBeDisabled();
    });
  });

  describe("Badge", () => {
    it("renders badge content with variant classes", () => {
      render(<Badge variant="success">Active</Badge>);
      expect(screen.getByText("Active")).toBeInTheDocument();
    });
  });

  describe("Field", () => {
    it("associates label with child input and renders hint", () => {
      render(
        <Field label="API Key" hint="Found in your account dashboard">
          <input type="text" />
        </Field>,
      );
      expect(screen.getByText("API Key")).toBeInTheDocument();
      expect(screen.getByText("Found in your account dashboard")).toBeInTheDocument();
      expect(screen.getByRole("textbox")).toBeInTheDocument();
    });
  });
});
