import { describe, expect, it } from "vitest";
import { endedHint, isOpen, isWorking, statusTag, takesMessage } from "./format";
import { pageContext } from "./pageContext";

describe("statusTag", () => {
  it("has no tag for a chat that waits without a question", () => {
    expect(statusTag("waiting", false)).toBeNull();
    expect(statusTag("waiting", true)).toEqual({ label: "Asks you", tone: "attention" });
    expect(statusTag("failed", false)).toEqual({ label: "Failed", tone: "error" });
  });

  it("says what a chat whose agent is gone is tagged", () => {
    expect(statusTag("stopped", false)).toEqual({ label: "Agent disconnected", tone: "plain" });
  });

  it("knows which chats take messages and which work", () => {
    expect([isOpen("waiting"), isOpen("running"), isOpen("succeeded")]).toEqual([
      true,
      true,
      false,
    ]);
    expect([isWorking("starting"), isWorking("waiting")]).toEqual([true, false]);
  });

  it("takes a message again once the agent has gone away", () => {
    // The chat a person is reading is taken up by their next message; only a chat that ended for good sends them on.
    expect([takesMessage("waiting"), takesMessage("stopped"), takesMessage("succeeded")]).toEqual([
      true,
      true,
      false,
    ]);
    expect([takesMessage("starting"), takesMessage("failed")]).toEqual([true, false]);
  });
});

describe("endedHint", () => {
  it("names the agent, the state, and the way on", () => {
    expect(endedHint("succeeded", "Coder")).toBe(
      "Coder ended this chat. Start a new chat to continue.",
    );
    // A state the app has not met is still answered rather than left blank.
    expect(endedHint("vanished", "Moebius")).toBe(
      "This chat has ended. Start a new chat to continue.",
    );
  });
});

describe("pageContext", () => {
  const snapshot = {
    boards: [{ repository: { id: 4, name: "demo" } }],
    harness_changes: [],
    inbox: [],
  } as never;

  it("names the page and its group", () => {
    expect(pageContext("/inbox", undefined)).toEqual({
      text: "Page: Inbox",
      name: "Inbox",
      repository: null,
    });
    expect(pageContext("/agents", undefined).text).toBe("Page: Build > Agents");
    // The name is what a floating window says it is looking at, so it is the short form, without the group.
    expect(pageContext("/agents", undefined).name).toBe("Agents");
  });

  it("names the repository of a board and starts chats there", () => {
    const context = pageContext("/board/4", snapshot);
    expect(context.repository).toBe(4);
    expect(context.text).toContain("Repository: demo (id 4)");
    expect(context.name).toBe("demo");
  });

  it("names an open chat", () => {
    expect(pageContext("/chat/12", undefined).text).toBe("Page: Chat\nOpen chat: 12");
  });
});
