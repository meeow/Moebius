import { describe, expect, it } from "vitest";
import type { Entry, ToolCall } from "./conversation";
import { groupTurns, stepViews } from "./turns";

const call = (id: string, tool: string, input: ToolCall["input"], refusedCall = false): Entry => ({
  at: 0,
  kind: "tool",
  call: {
    id,
    tool,
    input,
    check: refusedCall ? { allowed: false, reason: "no" } : null,
    mcp: null,
    execution: null,
    output: "",
  },
});
const read = (id: string, path: string) => call(id, "read", { type: "read", path });
const text = (value: string): Entry => ({ at: 0, kind: "text", text: value });
const person = (message: string): Entry => ({
  at: 0,
  kind: "person",
  said: "said",
  message,
  pictures: [],
});

describe("groupTurns", () => {
  it("heads a turn with the person's message and puts what follows in its body", () => {
    const turns = groupTurns([person("hi"), text("Hello."), person("again"), text("Yes.")], false);
    expect(turns).toHaveLength(2);
    expect(turns[0]?.head?.message).toBe("hi");
    expect(turns[0]?.blocks).toMatchObject([{ kind: "text", text: "Hello." }]);
    expect(turns[1]?.blocks).toMatchObject([{ kind: "text", text: "Yes." }]);
  });

  it("makes a turn without a head for what comes before any message", () => {
    const turns = groupTurns([text("Welcome.")], false);
    expect(turns).toHaveLength(1);
    expect(turns[0]?.head).toBeNull();
  });

  it("makes the thinking and tool calls that follow one another one block of steps", () => {
    const turns = groupTurns(
      [
        person("go"),
        { at: 0, kind: "thinking", text: "hm" },
        call("a", "bash", { type: "command", command: "ls" }),
        text("Done."),
        call("b", "bash", { type: "command", command: "pwd" }),
      ],
      true,
    );
    const blocks = turns[0]?.blocks ?? [];
    expect(blocks.map((block) => block.kind)).toEqual(["steps", "text", "steps"]);
    // Only the last block of steps works, and only while the agent does.
    expect(blocks.map((block) => block.kind === "steps" && block.working)).toEqual([
      false,
      false,
      true,
    ]);
    expect(
      groupTurns([call("b", "bash", { type: "command", command: "pwd" })], false)[0]?.blocks[0],
    ).toMatchObject({
      working: false,
    });
  });

  it("keeps the keys of earlier turns when the conversation grows", () => {
    const before = groupTurns([person("hi"), text("a")], false);
    const after = groupTurns([person("hi"), text("ab"), person("more")], false);
    expect(after[0]?.key).toBe(before[0]?.key);
    expect(after[0]?.blocks[0]?.key).toBe(before[0]?.blocks[0]?.key);
  });

  it("shows errors as blocks of their own", () => {
    const turns = groupTurns(
      [person("hi"), { at: 0, kind: "error", source: "model", message: "401" }],
      false,
    );
    expect(turns[0]?.blocks).toMatchObject([{ kind: "error", source: "model", message: "401" }]);
  });
});

describe("stepViews", () => {
  const indexed = (entries: Entry[]) => entries.map((entry, index) => ({ index, entry }));

  it("makes two or more reads in a row one line, and leaves a single read alone", () => {
    const views = stepViews(
      indexed([
        read("a", "1"),
        read("b", "2"),
        call("c", "bash", { type: "command", command: "ls" }),
        read("d", "3"),
      ]),
    );
    expect(views.map((view) => view.kind)).toEqual(["reads", "tool", "tool"]);
    expect(views[0]).toMatchObject({ kind: "reads" });
    expect(views[0]?.kind === "reads" && views[0].calls).toHaveLength(2);
  });

  it("does not hide a refused read among the others", () => {
    const views = stepViews(
      indexed([
        read("a", "1"),
        call("b", "read", { type: "read", path: "2" }, true),
        read("c", "3"),
      ]),
    );
    expect(views.map((view) => view.kind)).toEqual(["tool", "tool", "tool"]);
  });
});
