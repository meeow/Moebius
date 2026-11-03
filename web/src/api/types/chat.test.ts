import { describe, expect, it } from "vitest";
import {
  type AvailableModels,
  listedModel,
  offeredLevels,
  sentLevel,
  THINKING_LEVELS,
} from "./chat";

const available = (over: Partial<AvailableModels>): AvailableModels => ({
  models: [],
  errors: [],
  classes: [],
  decision_models: [],
  ...over,
});

describe("listedModel", () => {
  const listed = available({
    models: [
      { id: "one", protocol: "openai", upstream: "u", images: true, levels: ["low", "xhigh"] },
      { id: "two", protocol: "openai", upstream: "u", images: false },
    ],
    classes: [{ name: "strong", models: ["one", "two"], protocols: ["openai"], images: true }],
  });

  it("finds the model by its id", () => {
    expect(listedModel(listed, "one")?.levels).toEqual(["low", "xhigh"]);
    expect(listedModel(listed, "")).toBeUndefined();
    expect(listedModel(undefined, "one")).toBeUndefined();
  });

  it("knows nothing of a model class, which is offered every level by its own name", () => {
    const strong = listedModel(listed, "strong");
    expect(strong).toBeUndefined();
    expect(offeredLevels(strong)).toEqual([...THINKING_LEVELS]);
    expect(sentLevel(strong, "max")).toBe("max");
  });
});

describe("offeredLevels", () => {
  it("keeps every level of a model that does not say", () => {
    expect(offeredLevels(undefined)).toEqual([...THINKING_LEVELS]);
    expect(offeredLevels({ id: "a", protocol: "openai", upstream: "u", images: true })).toEqual([
      ...THINKING_LEVELS,
    ]);
  });

  it("offers what the server says, overrides included", () => {
    const model = {
      id: "a",
      protocol: "openai",
      upstream: "u",
      images: true,
      thinking_levels: ["off", "medium", "max"],
    };
    expect(offeredLevels(model)).toEqual(["off", "medium", "max"]);
  });
});

describe("sentLevel", () => {
  const model = {
    id: "a",
    protocol: "openai",
    upstream: "u",
    images: true,
    thinking: { off: "none", high: "xhigh", max: "xhigh" },
  };

  it("names what the server sends, not what the page works out", () => {
    expect(sentLevel(model, "off")).toBe("none");
    expect(sentLevel(model, "max")).toBe("xhigh");
    // A level the server renames nowhere goes out by its own name.
    expect(sentLevel(model, "low")).toBe("low");
    expect(sentLevel(undefined, "max")).toBe("max");
  });
});
