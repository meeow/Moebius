// @vitest-environment jsdom
/**
 * The note under the Model field: what an upstream that did not answer says and what to do about it, the control that
 * asks them again, and the quiet check that says the last ask came back with nothing wrong. Asking usually works, so
 * the check is a mark rather than a pill, and it appears only once the button has been pressed.
 */
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AvailableModels } from "../../api/types/chat";
import { ModelNote } from "./ModelField";

afterEach(cleanup);

const answered: AvailableModels = {
  models: [{ id: "big", protocol: "openai", upstream: "https://api.one.example", images: true }],
  errors: [],
  classes: [],
  decision_models: [],
};

const oneFailed: AvailableModels = {
  ...answered,
  errors: [{ upstream: "https://api.two.example", message: "401" }],
};

/** The row of the note, so the mark and the button can be met in the order they stand in. */
function row() {
  return screen.getByRole("button", { name: "Refresh" }).parentElement;
}

describe("the note under the Model field", () => {
  it("says nothing about the answer until the upstreams are asked again", () => {
    render(<ModelNote available={answered} onRefresh={vi.fn()} />);
    expect(screen.getByText("1 model from 1 provider")).toBeTruthy();
    expect(screen.queryByRole("img", { name: "Answered" })).toBeNull();
  });

  it("marks a clean answer with a check to the left of the button", () => {
    render(<ModelNote available={answered} onRefresh={vi.fn()} answered />);
    const mark = screen.getByRole("img", { name: "Answered" });
    const where = row();
    expect(where?.contains(mark)).toBe(true);
    expect(where?.textContent).toBe("Refresh");
  });

  it("marks nothing when an upstream did not answer, which is said in words instead", () => {
    render(<ModelNote available={oneFailed} onRefresh={vi.fn()} />);
    expect(screen.getByText("Could not ask https://api.two.example: 401")).toBeTruthy();
    expect(screen.queryByRole("img", { name: "Answered" })).toBeNull();
  });

  it("asks the upstreams again when the button is pressed", () => {
    const onRefresh = vi.fn();
    render(<ModelNote available={answered} onRefresh={onRefresh} />);
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(onRefresh).toHaveBeenCalledOnce();
  });
});
