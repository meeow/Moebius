import { describe, expect, it } from "vitest";
import type { Agent } from "../api/types/chat";
import { newChatBody } from "./NewChat";
import { chatAgents, modelChoices } from "./useChoices";
import { runEventOf } from "./useThread";

const agent = (over: Partial<Agent>): Agent => ({
  name: "A",
  model: "m",
  protocol: "openai",
  harness: "pi-bolt",
  built_in: false,
  ...over,
});

describe("chatAgents", () => {
  it("offers a built-in agent without a model, but not a broken one", () => {
    const listed = chatAgents([
      agent({ name: "Moebius", model: "", built_in: true }),
      agent({ name: "Broken", model: "", error: "bad file" }),
      agent({ name: "Coder" }),
      agent({ name: "Grilling-partner", model: "", built_in: true, error: "bad file" }),
    ]);
    expect(listed.map((each) => each.name)).toEqual(["Moebius", "Coder"]);
  });
});

describe("modelChoices", () => {
  it("lists the classes first, then each model once, sorted", () => {
    expect(
      modelChoices({
        models: [
          { id: "b", protocol: "openai", upstream: "u", images: false },
          { id: "a", protocol: "openai", upstream: "u", images: true },
          { id: "a", protocol: "anthropic", upstream: "v", images: true },
        ],
        errors: [],
        classes: [{ name: "strong", models: ["a"], protocols: ["openai"], images: true }],
        decision_models: [],
      }),
    ).toEqual(["strong", "a", "b"]);
    expect(modelChoices(undefined)).toEqual([]);
  });
});

describe("newChatBody", () => {
  const draft = { agent: "Moebius", repository: "", model: "", thinking: "", message: " hello " };

  it("starts a chat with the Moebius agent without a model", () => {
    expect(newChatBody(draft)).toEqual({ agent: "Moebius", message: "hello" });
  });

  it("does not start without a message", () => {
    expect(newChatBody({ ...draft, message: "  " })).toBeNull();
  });

  it("carries the place, the model and the reasoning the person chose", () => {
    expect(newChatBody({ ...draft, repository: "3", model: "strong", thinking: "high" })).toEqual({
      agent: "Moebius",
      message: "hello",
      repository: 3,
      model: "strong",
      thinking: "high",
    });
    expect(newChatBody({ ...draft, repository: "harness" })).toMatchObject({ harness: true });
  });

  it("needs a repository for a grilling", () => {
    expect(newChatBody({ ...draft, agent: "Grilling-partner" })).toBeNull();
    expect(newChatBody({ ...draft, agent: "Grilling-partner", repository: "2" })).toMatchObject({
      repository: 2,
    });
  });
});

describe("runEventOf", () => {
  const event = (run: number) => ({
    kind: "run_event",
    data: { run, seq: 5, kind: "said", data: { message: "hi" } },
  });

  it("reads the run event of the chat it is for", () => {
    expect(runEventOf(3, event(3))).toMatchObject({
      seq: 5,
      kind: "said",
      data: { message: "hi" },
    });
  });

  it("drops another chat's event and every other kind", () => {
    expect(runEventOf(3, event(4))).toBeNull();
    expect(runEventOf(3, { kind: "run_ended", data: { run: 3 } })).toBeNull();
    expect(runEventOf(3, { kind: "run_event", data: { run: 3 } })).toBeNull();
  });
});
