import { describe, expect, it } from "vitest";
import type { ServerSettings } from "../../api/types/models";
import {
  addPath,
  agentRequest,
  blankForm,
  decisionChoices,
  fileTable,
  formOf,
  fromFile,
  modelChoices,
  toggle,
  withChosen,
} from "./model";

const filled = () => ({ ...blankForm("Coder"), model: "qwen" });

describe("what a save sends", () => {
  it("is the name, the settings the agent needs and the instructions", () => {
    const form = { ...filled(), instructions: "Keep changes small.\n", skills: ["tdd"] };
    expect(agentRequest(form)).toEqual({
      name: "Coder",
      settings: { model: "qwen", protocol: "openai", skills: ["tdd"], extensions: [], mcp: [] },
      instructions: "Keep changes small.",
    });
  });

  it("leaves out what is the default and sends the rest", () => {
    const form = {
      ...filled(),
      harness: "claude-code",
      thinking: "high",
      budget: " 2000000 ",
      runner: "box",
      hideAfter: "0",
      engineeringSkills: false,
    };
    const request = agentRequest(form);
    expect(request).toMatchObject({
      settings: {
        harness: "claude-code",
        thinking: "high",
        token_budget: 2000000,
        runner: "box",
        engineering_skills: false,
        tool_output: { hide_after_turns: 0, max_lines: 200, max_bytes: 16384, compress: true },
      },
    });
  });

  it.each([
    [{ name: "bad name" }, "A name is made of letters, digits, - and _."],
    [{ name: "" }, "A name is made of letters, digits, - and _."],
    [{ model: "  " }, "Enter the model."],
    [{ budget: "0" }, "The token budget is a whole number of at least 1."],
    [{ budget: "1.5" }, "The token budget is a whole number of at least 1."],
    [{ maxLines: "many" }, "The tool output fields are whole numbers; 0 turns one off."],
  ])("says what is wrong with %j", (change, problem) => {
    expect(agentRequest({ ...filled(), ...change })).toBe(problem);
  });
});

describe("opening an agent", () => {
  it("fills the fields from the files and marks the agent as one that can be deleted", () => {
    const form = formOf({
      name: "Coder",
      instructions: "Hi.",
      settings: {
        model: "m",
        protocol: "anthropic",
        thinking: "low",
        token_budget: 5,
        skills: ["a"],
        mcp: ["docs"],
        extensions: ["extensions/log.ts"],
        engineering_skills: false,
        tool_output: { hide_after_turns: 1, max_lines: 2, max_bytes: 3, compress: false },
      },
    });
    expect(form).toMatchObject({
      exists: true,
      model: "m",
      protocol: "anthropic",
      thinking: "low",
      budget: "5",
      hideAfter: "1",
      compress: false,
      engineeringSkills: false,
      mcp: ["docs"],
    });
  });

  it("edits a built-in agent without files from blank and keeps what it brings", () => {
    const form = formOf({ name: "Moebius", built_in: { instructions: "Always.", skills: [] } });
    expect(form.exists).toBe(false);
    expect(form.builtIn?.instructions).toBe("Always.");
    expect(form.hideAfter).toBe("10");
  });
});

describe("the lists of names", () => {
  it("toggles a name and keeps the chosen names that are not on offer", () => {
    expect(toggle(["a"], "b")).toEqual(["a", "b"]);
    expect(toggle(["a", "b"], "a")).toEqual(["b"]);
    expect(withChosen(["a"], ["a", "gone"])).toEqual(["a", "gone"]);
  });

  it("adds a path once and not when blank", () => {
    expect(addPath(["a"], " b ")).toEqual(["a", "b"]);
    expect(addPath(["a"], "a")).toEqual(["a"]);
    expect(addPath(["a"], "  ")).toEqual(["a"]);
  });
});

describe("an agent of moebius.toml", () => {
  const server = (agents: unknown): ServerSettings => ({
    file: "/x/moebius.toml",
    values: { agents: { value: agents, default: false } },
    secrets: [],
    restart_needed: false,
    restart_keys: [],
  });

  it("is read from the table and written back as one", () => {
    const detail = fromFile(
      "Coder",
      server({
        Coder: {
          model: "m",
          thinking: "low",
          instructions: "Hi.",
          skills: ["skills/a"],
          token_budget: 7,
          tool_output: { max_lines: 5 },
        },
      }),
    );
    expect(detail?.settings).toMatchObject({
      model: "m",
      protocol: "openai",
      thinking: "low",
      token_budget: 7,
      skills: ["skills/a"],
      tool_output: { hide_after_turns: 10, max_lines: 5, max_bytes: 16384, compress: true },
    });
    expect(fileTable(detail?.settings as never, "Hi.")).toEqual({
      model: "m",
      protocol: "openai",
      thinking: "low",
      instructions: "Hi.",
      token_budget: 7,
      tool_output: { hide_after_turns: 10, max_lines: 5, max_bytes: 16384, compress: true },
      skills: ["skills/a"],
    });
  });

  it("is not there when the file has no such table", () => {
    expect(fromFile("Nobody", server({}))).toBeNull();
  });
});

describe("the choices of a Model field", () => {
  const available = {
    models: [{ id: "m1", protocol: "openai", upstream: "http://u", images: true }],
    errors: [],
    classes: [
      { name: "strong", models: ["m1"], protocols: ["openai"], images: true },
      { name: "mixed", models: ["m1", "m2"], protocols: ["openai", "anthropic"], images: true },
    ],
    decision_models: [],
  };

  it("are the classes first, with a protocol only when there is one", () => {
    expect(modelChoices(available).map((c) => [c.id, c.protocol, c.class])).toEqual([
      ["strong", "openai", true],
      ["mixed", "", true],
      ["m1", "openai", false],
    ]);
  });

  it("are what the decision model's endpoint lists, or the upstreams' models before it lists any", () => {
    expect(decisionChoices(available, "http://d").map((c) => c.id)).toEqual(["m1"]);
    const listed = { ...available, decision_models: ["decider"] };
    expect(decisionChoices(listed, "http://d")).toEqual([
      { id: "decider", protocol: "openai", upstream: "http://d", class: false },
    ]);
  });
});
