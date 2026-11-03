import { describe, expect, it } from "vitest";
import type { Card, CardRun, Snapshot } from "../../api/types/board";
import type { Chat } from "../../api/types/chat";
import type { Entry } from "../../chat/conversation";
import {
  afterSteps,
  context,
  elapsed,
  latestSteps,
  statusWord,
  thinkingNow,
  thoughtSeconds,
  tokensText,
  working,
  workStartedAt,
} from "./model";

const card = (id: number, number: number, run: CardRun | null, tokens = 0): Card => ({
  id,
  repository_id: 1,
  number,
  title: `Card ${number}`,
  body: "",
  author: "person",
  column: "In progress",
  created_at: 0,
  note: null,
  retried_after: null,
  rounds: 0,
  tokens,
  run,
  blocked_by: [],
  refusals: 0,
  change: null,
});

const run = (id: number, status: CardRun["status"]): CardRun => ({
  id,
  agent: "Coder",
  purpose: "code",
  status,
});

const chat = (id: number, extra: Partial<Chat>): Chat => ({
  id,
  agent: "Moebius",
  status: "running",
  repository_id: 1,
  harness_repository: false,
  started_at: 1000,
  ended_at: null,
  title: "A chat",
  answered: 0,
  seen: 0,
  changed_files: 0,
  asks: false,
  origin: "chat",
  model: null,
  thinking: null,
  ...extra,
});

const snapshot: Snapshot = {
  boards: [
    {
      repository: {
        id: 1,
        name: "radiance",
        path: "/r",
        forge: "none",
        default_branch: "main",
        github: null,
        trusted: [],
      },
      cards: [
        card(1, 48, run(9, "running"), 412_000),
        card(2, 47, run(11, "waiting")),
        card(3, 45, run(8, "succeeded")),
        card(4, 44, null),
      ],
      round_limit: 5,
      waiting: {},
      changes: [],
    },
  ],
  harness_changes: [],
  inbox: [],
};

describe("which runs are working", () => {
  it("lists the cards whose run is starting, running or waiting and the chats that are, by run", () => {
    const list = working(snapshot, [
      chat(7, { title: "" }),
      chat(5, { status: "succeeded" }),
      chat(6, { status: "waiting", asks: true, repository_id: null, harness_repository: true }),
    ]);
    expect(list.map((each) => each.run)).toEqual([6, 7, 9, 11]);
    expect(list.find((each) => each.run === 9)).toMatchObject({
      title: "#48 Card 48",
      repository: "radiance",
      tokens: 412_000,
      kind: "code",
      open: { kind: "card", repository: 1, card: 1 },
    });
    expect(list.find((each) => each.run === 11)?.tokens).toBeNull();
    expect(list.find((each) => each.run === 7)).toMatchObject({
      title: "Chat",
      kind: "chat",
      open: { kind: "chat", chat: 7 },
    });
    expect(list.find((each) => each.run === 6)).toMatchObject({
      asks: true,
      repository: "harness configuration",
    });
  });

  it("says how many agents work", () => {
    expect(context(0)).toBe("Nothing is working.");
    expect(context(1)).toBe("1 agent working");
    expect(context(3)).toBe("3 agents working");
  });
});

describe("the word a row leads with", () => {
  it("says what the run is doing and whether it waits for the person", () => {
    expect(statusWord("running", false)).toEqual({ word: "Running", tone: "progress" });
    expect(statusWord("starting", false)).toEqual({ word: "Starting", tone: "progress" });
    expect(statusWord("waiting", true)).toEqual({ word: "Asks you", tone: "attention" });
    expect(statusWord("waiting", false)).toEqual({ word: "Your turn", tone: "attention" });
  });
});

describe("how a tile tells its run", () => {
  it("tells tokens and time", () => {
    expect(tokensText(412_000)).toBe("412k tokens");
    expect(tokensText(1_500_000)).toBe("1.5M tokens");
    expect(tokensText(12)).toBe("12 tokens");
    expect(elapsed(42)).toBe("42s");
    expect(elapsed(437)).toBe("7m 17s");
    expect(elapsed(3720)).toBe("1h 02m");
    expect(elapsed(-5)).toBe("0s");
  });

  it("says how long a thought went on, and which one is still going", () => {
    const thought = (at: number): Entry => ({ at, seq: at, kind: "thinking", text: "t" });
    const tool = (at: number): Entry =>
      ({ at, seq: at, kind: "tool", call: { at } }) as unknown as Entry;
    const text = (at: number): Entry => ({ at, seq: at, kind: "text", text: "a" });
    // A thought runs until whatever follows it begins.
    expect(thoughtSeconds([thought(100), thought(105), tool(112)], 0)).toBe(5);
    expect(thoughtSeconds([thought(100), thought(105), tool(112)], 1)).toBe(7);
    // The last line is measured against whatever followed the steps at all, and a tool is not a thought.
    expect(thoughtSeconds([thought(100), thought(105)], 1, 112)).toBe(7);
    expect(thoughtSeconds([thought(100), thought(105)], 1)).toBeNull();
    expect(thoughtSeconds([thought(100), tool(105)], 1, 112)).toBeNull();
    // The time whatever came after the last step arrived, and none when the steps are the end of it.
    expect(afterSteps([text(0), thought(100), thought(105), text(109)])).toBe(109);
    expect(afterSteps([text(0), thought(100)])).toBeNull();
  });

  it("measures the work from the start of the run when only its newest messages are held", () => {
    const person = (at: number): Entry => ({
      at,
      seq: at,
      kind: "person",
      said: "said",
      message: "m",
      pictures: [],
    });
    // Everything is held: the first message is where the work began.
    expect(workStartedAt([person(300), person(400)], false, 100)).toBe(300);
    // A window of a long run begins long after the run did, so its first message would understate the work.
    expect(workStartedAt([person(300), person(400)], true, 100)).toBe(100);
    expect(workStartedAt([person(300)], true, 0)).toBeNull();
    // Nothing read yet says nothing of its own.
    expect(workStartedAt([], false, 100)).toBe(100);
    expect(workStartedAt([], false, 0)).toBeNull();
  });

  it("knows a run that waits for the person is not thinking", () => {
    expect(thinkingNow("starting")).toBe(true);
    expect(thinkingNow("running")).toBe(true);
    expect(thinkingNow("waiting")).toBe(false);
    expect(thinkingNow("done")).toBe(false);
  });

  it("shows the last run of steps", () => {
    const text: Entry = { at: 0, seq: 0, kind: "text", text: "a" };
    const thought: Entry = { at: 0, seq: 1, kind: "thinking", text: "t" };
    expect(latestSteps([text])).toEqual([]);
    expect(latestSteps([thought, text, thought, thought, text])).toHaveLength(2);
    expect(latestSteps([thought, thought])).toHaveLength(2);
    expect(latestSteps([])).toEqual([]);
  });
});
