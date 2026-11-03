import { describe, expect, it } from "vitest";
import type { RunEvent } from "../api/types/chat";
import { Conversation, type Entry, type ToolCall } from "./conversation";
import { prepended } from "./follow";
import { groupTurns, stepViews } from "./turns";

/** Entries are named by the event that started them, so a test gives each one the next `seq`. */
let next = 0;
const seq = () => {
  next += 1;
  return next;
};

const call = (id: string, tool: string, input: ToolCall["input"], refusedCall = false): Entry => ({
  at: 0,
  seq: seq(),
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
const text = (value: string): Entry => ({ at: 0, seq: seq(), kind: "text", text: value });
const person = (message: string): Entry => ({
  at: 0,
  seq: seq(),
  kind: "person",
  said: "said",
  message,
  pictures: [],
});
const thinking = (value: string): Entry => ({
  at: 0,
  seq: seq(),
  kind: "thinking",
  text: value,
});
/** The same entry, arrived at `when`, so a test can space a conversation out in time. */
const at = (entry: Entry, when: number): Entry => ({ ...entry, at: when });
/** The same entry with more of the answer on it, as the conversation replaces one that is still streaming. */
const grown = (entry: Entry, more: string): Entry =>
  entry.kind === "text" ? { ...entry, text: entry.text + more } : entry;
/** The entries of a block of steps, numbered as the conversation numbers them. */
const indexed = (entries: Entry[]) => entries.map((entry, index) => ({ index, entry }));

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
        thinking("hm"),
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

  it("keeps the keys of a turn when the conversation grows above it as well as below", () => {
    const greeting = person("hi");
    const answer = text("a");
    const before = groupTurns([greeting, answer], false);
    // The answer arrived in full, and a page of older messages came in above it. A turn is named by what it is, not by
    // how many came before it, so what a person had opened stays open and the list keeps its place.
    const after = groupTurns(
      [person("what came before"), greeting, grown(answer, "b"), person("more")],
      false,
    );
    expect(after).toHaveLength(3);
    expect(after[1]?.key).toBe(before[0]?.key);
    expect(after[1]?.blocks[0]?.key).toBe(before[0]?.blocks[0]?.key);
  });

  it("places a turn by what it holds when the page above joins an answer cut in two", () => {
    // A page opened in the middle of an answer: its first event is a piece of text, so the top turn has no head.
    const piece = (n: number, delta: string): RunEvent => ({
      seq: n,
      kind: "message_update",
      data: { assistantMessageEvent: { type: "text_delta", delta } },
      at: n,
    });
    const said = (n: number, message: string): RunEvent => ({
      seq: n,
      kind: "said",
      data: { message },
      at: n,
    });
    const chat = new Conversation();
    chat.backfill([piece(5, "second half. "), said(6, "next")]);
    const before = groupTurns(chat.entries(), false);
    expect(before.map((turn) => turn.head)).toEqual([null, expect.anything()]);
    const top = before[0]?.from ?? 0;

    // The page above holds the question and the start of the answer: the top turn is now headed and named for it, but
    // it is still the turn that held what was at the top, and the one before it is all that arrived above it.
    chat.backfill([
      said(1, "earlier"),
      piece(2, "ok. "),
      said(3, "question"),
      piece(4, "first half, "),
    ]);
    const after = groupTurns(chat.entries(), false);
    expect(after).toHaveLength(3);
    expect(after[1]?.head?.seq).toBe(3);
    expect(after[1]?.blocks[0]).toMatchObject({ kind: "text", text: "first half, second half. " });
    expect(after[1]?.key).not.toBe(before[0]?.key);
    expect(prepended(after, top)).toBe(1);
  });

  it("shows errors as blocks of their own", () => {
    const turns = groupTurns(
      [person("hi"), { at: 0, seq: seq(), kind: "error", source: "model", message: "401" }],
      false,
    );
    expect(turns[0]?.blocks).toMatchObject([{ kind: "error", source: "model", message: "401" }]);
  });
});

describe("stepViews", () => {
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

/**
 * A thought says how long it took once the model has moved on. The events never say when a thought stopped, so the
 * only length there is is the gap from the moment its first piece arrived to the moment whatever came after it
 * arrived; the thought the model is at this second has no length yet and is said in the present tense instead.
 */
describe("the length of a thought", () => {
  const command = (id: string, when: number) =>
    at(call(id, "bash", { type: "command", command: "ls" }), when);

  it("measures a thought by the gap to the entry that came after it", () => {
    const [thought] = stepViews(
      indexed([at(thinking("Reading the issue."), 100), command("a", 112)]),
      {
        nextAt: 200,
        working: true,
      },
    );
    expect(thought).toMatchObject({ kind: "thinking", seconds: 12, ongoing: false });
  });

  it("measures the last thought of a block by the entry that ended the block", () => {
    const [thought] = stepViews(indexed([at(thinking("Decided."), 100)]), {
      nextAt: 130,
      working: true,
    });
    expect(thought).toMatchObject({ kind: "thinking", seconds: 30, ongoing: false });
  });

  it("leaves the newest thought open while the model is at it", () => {
    const [thought] = stepViews(indexed([at(thinking("Still weighing it."), 100)]), {
      nextAt: null,
      working: true,
    });
    expect(thought).toMatchObject({ kind: "thinking", seconds: null, ongoing: true });
  });

  it("gives no length to a thought nothing came after and nothing will", () => {
    const [thought] = stepViews(indexed([at(thinking("That is all."), 100)]));
    expect(thought).toMatchObject({ kind: "thinking", seconds: null, ongoing: false });
  });

  it("does not count a thought backwards when the next entry is stamped earlier", () => {
    const [thought] = stepViews(indexed([at(thinking("Out of order."), 100), command("a", 90)]));
    expect(thought).toMatchObject({ seconds: 0, ongoing: false });
  });

  it("has the conversation tell a block the time its last thought stopped at", () => {
    const blocks =
      groupTurns(
        [at(person("go"), 0), at(thinking("First the tests."), 100), at(text("Done."), 130)],
        false,
      )[0]?.blocks ?? [];
    const [steps] = blocks;
    expect(steps?.kind === "steps" && steps.steps[0]).toMatchObject({
      seconds: 30,
      ongoing: false,
    });
  });

  it("has the newest block hold its last thought open while the agent works", () => {
    const blocks =
      groupTurns([at(person("go"), 0), at(thinking("Still at it."), 100)], true)[0]?.blocks ?? [];
    const [steps] = blocks;
    expect(steps?.kind === "steps" && steps.working).toBe(true);
    expect(steps?.kind === "steps" && steps.steps[0]).toMatchObject({
      seconds: null,
      ongoing: true,
    });
  });
});
