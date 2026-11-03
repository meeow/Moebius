import { describe, expect, it } from "vitest";
import type { AvailableModels } from "../api/types/chat";
import { levelChoices } from "./levels";

const available = (over: Partial<AvailableModels>): AvailableModels => ({
  models: [],
  errors: [],
  classes: [],
  decision_models: [],
  ...over,
});

// An endpoint whose hardest level is `xhigh`, with nothing for `high` or `max`.
const listed = available({
  models: [
    {
      id: "one",
      protocol: "openai",
      upstream: "u",
      images: true,
      levels: ["none", "minimal", "low", "medium", "xhigh"],
      thinking_levels: ["off", "minimal", "low", "medium", "xhigh"],
      thinking: { off: "none", high: "xhigh", max: "xhigh" },
    },
  ],
});

describe("levelChoices", () => {
  it("offers only what the model takes", () => {
    expect(levelChoices(listed, "one", "").map((option) => option.value)).toEqual([
      "off",
      "minimal",
      "low",
      "medium",
      "xhigh",
    ]);
  });

  it("keeps a level the model has no name for, saying what goes out", () => {
    const options = levelChoices(listed, "one", "max");
    const kept = options[options.length - 1];
    expect(kept).toEqual({ value: "max", label: "max (sent as xhigh)" });
  });

  it("offers a level an override makes valid, and labels a kept one as the override sends it", () => {
    const overridden = available({
      models: [
        {
          id: "one",
          protocol: "openai",
          upstream: "u",
          images: true,
          levels: ["low", "medium"],
          thinking_levels: ["low", "medium", "max"],
          thinking: { max: "medium", high: "medium" },
        },
      ],
    });
    expect(levelChoices(overridden, "one", "max").map((option) => option.value)).toEqual([
      "low",
      "medium",
      "max",
    ]);
    const kept = levelChoices(overridden, "one", "high");
    expect(kept.at(-1)).toEqual({ value: "high", label: "high (sent as medium)" });
  });

  it("offers every level for a model class", () => {
    const withClass = available({
      classes: [{ name: "strong", models: ["one"], protocols: ["openai"], images: true }],
      models: listed.models,
    });
    expect(levelChoices(withClass, "strong", "")).toHaveLength(7);
  });

  it("keeps every level of a model whose upstream does not say", () => {
    expect(levelChoices(available({}), "one", "")).toHaveLength(7);
  });
});
