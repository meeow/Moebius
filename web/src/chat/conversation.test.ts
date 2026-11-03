import { describe, expect, it } from "vitest";
import type { RunEvent } from "../api/types/chat";
import type { Entry } from "./conversation";
import {
  Conversation,
  inputPath,
  inputSummary,
  refused,
  toolInput,
  toolTitle,
  verdict,
} from "./conversation";

const clock = 1_790_000_000;
const event = (seq: number, kind: string, data: Record<string, unknown>): RunEvent => ({
  seq,
  kind,
  data,
  at: clock + seq,
});
const delta = (seq: number, text: string) =>
  event(seq, "message_update", {
    assistantMessageEvent: { type: "text_delta", contentIndex: 0, delta: text },
  });
const thinking = (seq: number, text: string) =>
  event(seq, "message_update", { assistantMessageEvent: { type: "thinking_delta", delta: text } });
const start = (seq: number, id: string, tool: string, args: unknown) =>
  event(seq, "tool_execution_start", { toolCallId: id, toolName: tool, args });
const end = (seq: number, id: string, tool: string, text: string, isError = false) =>
  event(seq, "tool_execution_end", {
    toolCallId: id,
    toolName: tool,
    isError,
    result: { content: [{ type: "text", text }] },
  });
const check = (seq: number, id: string, tool: string, decision: string, reason: string) =>
  event(seq, "tool_check", { tool, tool_call_id: id, decision, reason });

function fold(events: RunEvent[]): Conversation {
  const conversation = new Conversation();
  for (const each of events) conversation.push(each);
  return conversation;
}

describe("Conversation", () => {
  it("keeps the pictures a message was written with", () => {
    const conversation = new Conversation(12);
    conversation.push(
      event(1, "said", {
        message: "What is in this picture?",
        images: [
          { name: "screenshot.png", media_type: "image/jpeg", file: "9f2c.png" },
          // The same bytes sent twice are kept under the same name, so they are shown once.
          { name: "again.png", media_type: "image/jpeg", file: "9f2c.png" },
          // A picture with no stored bytes has nothing to show.
          { media_type: "image/png" },
        ],
      }),
    );
    const [entry] = conversation.entries();
    expect(entry?.kind).toBe("person");
    if (entry?.kind !== "person") return;
    expect(entry.message).toBe("What is in this picture?");
    // Each picture is read back from the chat's own directory, under the run the conversation is of.
    expect(entry.pictures).toEqual([
      {
        key: "9f2c.png",
        name: "screenshot.png",
        mediaType: "image/jpeg",
        src: "/v1/chats/12/images/9f2c.png",
      },
    ]);
  });

  it("holds no pictures for a message written in words alone", () => {
    const [entry] = fold([event(1, "said", { message: "Summarise the branch." })]).entries();
    expect(entry?.kind === "person" && entry.pictures).toEqual([]);
  });

  it("joins the streamed pieces of an answer into one text", () => {
    const entries = fold([delta(1, "Review found "), delta(2, "a problem.")]).entries();
    expect(entries).toHaveLength(1);
    expect(entries[0]).toMatchObject({ kind: "text", text: "Review found a problem." });
  });

  it("starts a new text after a tool call and keeps thinking apart", () => {
    const entries = fold([
      thinking(1, "Let me "),
      thinking(2, "look."),
      delta(3, "Reading."),
      start(4, "c1", "read", { path: "a.rs" }),
      delta(5, "Done."),
    ]).entries();
    expect(entries.map((entry) => entry.kind)).toEqual(["thinking", "text", "tool", "text"]);
    expect(entries[0]).toMatchObject({ text: "Let me look." });
  });

  it("puts the check, the start and the end of a tool call into one entry", () => {
    const entries = fold([
      check(1, "c1", "read", "allow", "no rule is broken"),
      start(2, "c1", "read", { path: "core/tree.cpp" }),
      end(3, "c1", "read", "int main() {}"),
    ]).entries();
    expect(entries).toHaveLength(1);
    const entry = entries[0];
    if (entry?.kind !== "tool") throw new Error("not a tool call");
    expect(entry.call.input).toEqual({ type: "read", path: "core/tree.cpp" });
    expect(entry.call.output).toBe("int main() {}");
    expect(entry.call.execution).toEqual({ state: "finished", failed: false });
    expect(refused(entry.call)).toBe(false);
  });

  it("shows a refused call as refused and a failed tool as failed", () => {
    const entries = fold([
      check(1, "c1", "bash", "refuse", "no-force-push: never force push"),
      start(2, "c2", "bash", { command: "false" }),
      end(3, "c2", "bash", "exit 1", true),
    ]).entries();
    const [refusedEntry, failedEntry] = entries;
    if (refusedEntry?.kind !== "tool" || failedEntry?.kind !== "tool") throw new Error("tools");
    expect(refused(refusedEntry.call)).toBe(true);
    expect(verdict(refusedEntry.call)?.reason).toBe("no-force-push: never force push");
    expect(failedEntry.call.execution).toEqual({ state: "finished", failed: true });
  });

  it("puts a gateway call into the harness's call of the same tool and names it by server", () => {
    const entries = fold([
      start(1, "c1", "mcp__moebius__docs__search", { q: "x" }),
      event(2, "mcp_call", {
        server: "docs",
        tool: "search",
        tool_call_id: "other",
        input: { q: "x" },
        decision: "allow",
        reason: "ok",
        result: { content: [{ type: "text", text: "3 hits" }] },
      }),
    ]).entries();
    expect(entries).toHaveLength(1);
    const entry = entries[0];
    if (entry?.kind !== "tool") throw new Error("not a tool call");
    expect(toolTitle(entry.call)).toBe("docs · search");
    expect(entry.call.output).toBe("3 hits");
  });

  it("keeps the person's messages, steers and errors in order", () => {
    const entries = fold([
      event(1, "said", { message: "hello" }),
      delta(2, "Hi."),
      event(3, "steered", { message: "stop that" }),
      event(4, "moebius_error", { message: "the agent changed nothing" }),
    ]).entries();
    expect(entries).toMatchObject([
      { kind: "person", said: "said", message: "hello" },
      { kind: "text" },
      { kind: "person", said: "steered", message: "stop that" },
      { kind: "error", source: "run", message: "the agent changed nothing" },
    ]);
  });

  describe("errors of the model", () => {
    it("shows a provider error of Pi's assistant message", () => {
      const entries = fold([
        event(1, "said", { message: "hello" }),
        event(2, "message_start", { message: { role: "assistant" } }),
        event(3, "message_end", {
          message: {
            role: "assistant",
            content: [],
            stopReason: "error",
            errorMessage: "401 invalid x-api-key (OAuth token refused)",
          },
        }),
      ]).entries();
      expect(entries[1]).toMatchObject({
        kind: "error",
        source: "model",
        message: "401 invalid x-api-key (OAuth token refused)",
      });
    });

    it("says something when the provider gave no message", () => {
      const entries = fold([
        event(1, "message_end", { message: { role: "assistant", stopReason: "error" } }),
      ]).entries();
      expect(entries[0]).toMatchObject({ kind: "error", message: "The model returned an error." });
    });

    it("shows no error for an answer that stopped well, or for a user message", () => {
      const entries = fold([
        event(1, "message_end", { message: { role: "assistant", stopReason: "stop" } }),
        event(2, "message_end", { message: { role: "user", stopReason: "error" } }),
        event(3, "agent_end", { messages: [], is_error: false }),
      ]).entries();
      expect(entries).toEqual([]);
    });

    it("shows the error once when Pi's agent_end repeats it, and in a terminal's record that has only agent_end", () => {
      const failed = {
        role: "assistant",
        content: [],
        stopReason: "error",
        errorMessage: "429 rate limit",
      };
      const both = fold([
        event(1, "message_end", { message: failed }),
        event(2, "agent_end", { messages: [failed] }),
      ]).entries();
      expect(both.filter((entry) => entry.kind === "error")).toHaveLength(1);
      const alone = fold([event(1, "agent_end", { messages: [failed] })]).entries();
      expect(alone).toMatchObject([{ kind: "error", source: "model", message: "429 rate limit" }]);
    });

    it("shows the same error again in a later turn", () => {
      const failed = { role: "assistant", stopReason: "error", errorMessage: "529 overloaded" };
      const entries = fold([
        event(1, "message_end", { message: failed }),
        event(2, "said", { message: "try again" }),
        event(3, "message_end", { message: failed }),
      ]).entries();
      expect(entries.filter((entry) => entry.kind === "error")).toHaveLength(2);
    });

    it("shows the error a Claude Code run ended with", () => {
      const entries = fold([
        event(1, "agent_end", {
          is_error: true,
          subtype: "error_during_execution",
          messages: [
            { role: "assistant", content: [{ type: "text", text: "429 quota exceeded" }] },
          ],
        }),
      ]).entries();
      expect(entries[0]).toMatchObject({
        kind: "error",
        source: "model",
        message: "429 quota exceeded",
      });
    });
  });

  it("counts an event seen twice once and puts a late event in its place", () => {
    const events = [delta(1, "a"), delta(2, "b"), delta(3, "c")];
    const once = fold(events).entries();
    const twice = fold([...events, events[1] as RunEvent]).entries();
    expect(twice).toEqual(once);
    const shuffled = fold([
      events[0] as RunEvent,
      events[2] as RunEvent,
      events[1] as RunEvent,
    ]).entries();
    expect(shuffled).toEqual(once);
    expect(fold(events).lastSeq).toBe(3);
  });

  it("replaces only the entry that changed", () => {
    const conversation = fold([event(1, "said", { message: "hi" }), delta(2, "a")]);
    const before = conversation.entries();
    conversation.push(delta(3, "b"));
    const after = conversation.entries();
    expect(after[0]).toBe(before[0]);
    expect(after[1]).not.toBe(before[1]);
  });
});

describe("toolInput", () => {
  it("reads Pi's edit with several replacements and its older single form", () => {
    expect(
      toolInput("edit", {
        path: "a",
        edits: [
          { oldText: "x", newText: "y" },
          { oldText: "p", newText: "q" },
        ],
      }),
    ).toEqual({
      type: "edit",
      path: "a",
      changes: [
        { oldText: "x", newText: "y" },
        { oldText: "p", newText: "q" },
      ],
    });
    expect(toolInput("edit", { path: "a", oldText: "x", newText: "y" })).toMatchObject({
      changes: [{ oldText: "x", newText: "y" }],
    });
  });

  it("reads Claude Code's tools, whose names are capitalised", () => {
    expect(toolInput("Edit", { file_path: "a", old_string: "x", new_string: "y" })).toEqual({
      type: "edit",
      path: "a",
      changes: [{ oldText: "x", newText: "y" }],
    });
    expect(toolInput("Bash", { command: "ls\nwc" })).toEqual({
      type: "command",
      command: "ls\nwc",
    });
    expect(
      toolInput("TodoWrite", { todos: [{ content: "Write tests", status: "in_progress" }] }),
    ).toEqual({ type: "todos", todos: [{ content: "Write tests", status: "in_progress" }] });
  });

  it("summarises an input in one line", () => {
    expect(inputSummary(toolInput("bash", { command: "ls\nwc" }))).toBe("ls");
    // A file is named by its name: the directory it sits in is in the tooltip, not in the row.
    expect(inputSummary(toolInput("read", { path: "/srv/app/docs/a.md" }))).toBe("a.md");
    expect(inputPath(toolInput("read", { path: "/srv/app/docs/a.md" }))).toBe("/srv/app/docs/a.md");
    expect(inputPath(toolInput("bash", { command: "ls" }))).toBeNull();
    expect(inputSummary(toolInput("grep", { pattern: "x" }))).toBe('{"pattern":"x"}');
    expect(inputSummary(toolInput("grep", null))).toBe("");
  });
});

describe("a page of history read behind the stream", () => {
  // Reading what was missed and being sent what happens now are two paths into one conversation, so the page often
  // arrives holding events older than the newest the stream already brought. Putting each in its place one at a time
  // folded the whole log again per event, which is seconds on a long chat that is still answering.
  const late = (seq: number, text: string, at: number): RunEvent => ({
    ...delta(seq, text),
    at,
  });
  const textOf = (entry?: Entry) => (entry?.kind === "text" ? entry.text : "");

  it("folds a live event and the page that follows it into one conversation", () => {
    const conversation = new Conversation();
    // The stream is ahead: 101 landed before the read of 1..100 came back.
    conversation.push(late(101, " and the end", clock + 101));
    const history: RunEvent[] = [];
    for (let seq = 1; seq <= 100; seq += 1) history.push(delta(seq, ` ${seq}`));
    conversation.backfill(history);
    const entries = conversation.entries();
    expect(entries).toHaveLength(1);
    expect(textOf(entries[0])).toMatch(/^ 1 2 3/);
    expect(textOf(entries[0]).endsWith(" 100 and the end")).toBe(true);
    expect(conversation.lastSeq).toBe(101);
  });

  it("keeps the server's time over the clock of the browser that saw the event first", () => {
    // An event off the stream has no time on it, so the browser's own clock stands in; a duration is worked out
    // between two of these, so a machine an hour ahead read a forty second thought as two hours until the stored
    // event arrived to say when it really happened.
    const conversation = new Conversation();
    conversation.push(late(2, " the answer", 1_700_007_200));
    conversation.backfill([
      event(1, "said", { message: "the question" }),
      { ...delta(2, " the answer"), at: clock + 2 },
    ]);
    const at = conversation.entries().map((each) => each.at);
    expect(at).toEqual([clock + 1, clock + 2]);
    expect(conversation.lastAt).toBe(clock + 2);
  });

  it("takes a page that is only new events without folding anything again", () => {
    const conversation = new Conversation();
    conversation.backfill([delta(1, "a"), delta(2, "b")]);
    conversation.backfill([delta(3, "c")]);
    expect(conversation.entries()).toHaveLength(1);
    expect(textOf(conversation.entries()[0])).toBe("abc");
    expect(conversation.lastSeq).toBe(3);
  });
});
