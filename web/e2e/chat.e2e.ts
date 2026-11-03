/**
 * The chat page and the floating chat in a real browser: an answer that streams in is followed unless the person
 * scrolled up, tool calls are pills, a provider's error is an entry of the chat, Copy puts text on the clipboard,
 * a new chat with the Moebius agent starts without a model, and the floating window opens over another page. The
 * agent is played by the fake server (e2e/fake/chat.ts), which sends the events exactly as the real one does; against
 * a real server only the page itself is looked at, as it has no model to answer with. Every wait is on something the
 * page shows, never on time.
 */
import { type Browser, type BrowserContext, chromium, type Locator, type Page } from "playwright";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import type { FakeChat } from "./fake/chat";
import { startServer, type TestServer, usesRealServer } from "./server";

let browser: Browser;
let server: TestServer;
let context: BrowserContext;
let page: Page;

beforeAll(async () => {
  browser = await chromium.launch({
    executablePath: process.env.MOEBIUS_E2E_CHROMIUM || undefined,
  });
});

afterAll(async () => {
  await browser.close();
});

/** A new server and a new browser context that may use the clipboard, signed in and on `path`. */
async function open(path: string) {
  server = await startServer();
  context = await browser.newContext();
  await context.grantPermissions(["clipboard-read", "clipboard-write"], { origin: server.url });
  page = await context.newPage();
  await page.goto(server.loginUrl);
  await page.getByRole("navigation", { name: "Pages" }).waitFor();
  if (path !== "/inbox") await page.goto(`${server.url}${path}`);
}

afterEach(async () => {
  await context.close();
  await server.stop().catch(() => {});
});

/** The chat routes of the fake; only the tests that skip a real server ask for it. */
function fake(): FakeChat {
  if (!server.chat) throw new Error("these tests run against the fake server");
  return server.chat;
}

const field = () => page.getByRole("textbox", { name: "Message" });
const thread = () => page.getByTestId("thread");

/** Writes `text` in the field and sends it with Enter. */
async function say(text: string) {
  await field().fill(text);
  await field().press("Enter");
}

/** How far the conversation's list is from its end, in pixels. */
function distanceFromEnd(list: Locator): Promise<number> {
  return list.evaluate(
    (element) => element.scrollHeight - element.scrollTop - element.clientHeight,
  );
}

describe("the chat page", () => {
  it("loads, lists the chats and offers a new one", async () => {
    await open("/chat");
    await page.getByRole("button", { name: /^Chats \(\d+\)/ }).waitFor();
    await page.getByRole("link", { name: "+ New" }).waitFor();
    await page.getByRole("button", { name: /^Agent: / }).waitFor();
    await field().waitFor();
  });

  it.skipIf(usesRealServer)(
    "lists the chats newest first and opens one from the list",
    async () => {
      await open("/chat");
      const first = fake().seed({
        title: "Older chat",
        events: [["said", { message: "Older chat" }]],
      });
      const second = fake().seed({
        title: "Newer chat",
        events: [["said", { message: "Newer chat" }]],
      });
      await page.reload();
      await page.getByRole("button", { name: "Chats (2)" }).click();
      const rows = page.getByTestId("chat-row");
      await rows.first().waitFor();
      expect(await rows.allInnerTexts()).toEqual([
        expect.stringContaining("Newer chat"),
        expect.stringContaining("Older chat"),
      ]);
      await rows.nth(1).click();
      await page.waitForURL(`**/chat/${first}`);
      await page.getByTestId("person-message").filter({ hasText: "Older chat" }).waitFor();
      expect(second).toBeGreaterThan(first);
    },
  );

  it.skipIf(usesRealServer)("shows the sidebar's count of unread chats", async () => {
    await open("/chat");
    fake().seed({ title: "Done", answered: 5, seen: 0, events: [["said", { message: "Done" }]] });
    await page.reload();
    await page
      .getByRole("navigation", { name: "Pages" })
      .getByRole("link", { name: /^Chat 1 new/ })
      .waitFor();
  });
});

describe("an answer that streams in", () => {
  it.skipIf(usesRealServer)("is followed to its end unless the person scrolled up", async () => {
    await open("/chat");
    fake().autoAnswer = false;
    await say("Tell me a long story");
    await page.waitForURL(/\/chat\/\d+$/);
    const run = fake().chats[0]?.id as number;
    await page.getByTestId("person-message").filter({ hasText: "Tell me a long story" }).waitFor();

    fake().agentStart(run);
    const paragraph = (n: number) => `Paragraph ${n}: ${"the story goes on and on ".repeat(8)}\n\n`;
    for (let n = 1; n <= 40; n++) fake().delta(run, paragraph(n));
    // The view follows: the last paragraph is on screen, at the end of the list.
    await page.getByText("Paragraph 40:").waitFor();
    await expect.poll(() => distanceFromEnd(thread())).toBeLessThan(40);

    // The person scrolls up; what comes next does not pull the view down.
    await thread().hover();
    await page.mouse.wheel(0, -100_000);
    await expect.poll(() => thread().evaluate((element) => element.scrollTop)).toBe(0);
    for (let n = 41; n <= 60; n++) fake().delta(run, paragraph(n));
    await expect
      .poll(() => thread().evaluate((element) => element.scrollHeight))
      .toBeGreaterThan(await thread().evaluate((element) => element.clientHeight * 3));
    expect(await thread().evaluate((element) => element.scrollTop)).toBe(0);

    // Reaching the end follows again.
    await thread().evaluate((element) => element.scrollTo({ top: element.scrollHeight }));
    fake().delta(run, paragraph(61));
    await page.getByText("Paragraph 61:").waitFor();
    await expect.poll(() => distanceFromEnd(thread())).toBeLessThan(40);

    // So does sending a message, wherever the person had scrolled to.
    await thread().evaluate((element) => element.scrollTo({ top: 0 }));
    fake().settle(run);
    await say("And then?");
    await page.getByTestId("person-message").filter({ hasText: "And then?" }).waitFor();
    await expect.poll(() => distanceFromEnd(thread())).toBeLessThan(40);
  });

  it.skipIf(usesRealServer)("shows the agent working until it settles", async () => {
    await open("/chat");
    fake().autoAnswer = false;
    await say("Work for a while");
    await page.waitForURL(/\/chat\/\d+$/);
    const run = fake().chats[0]?.id as number;
    fake().agentStart(run);
    fake().toolCall(run, { id: "t1", tool: "bash", args: { command: "sleep 1" }, output: "" });
    await page.getByTestId("working").waitFor();
    fake().delta(run, "Finished.");
    fake().settle(run, "Finished.");
    await page.getByText("Finished.").waitFor();
    await page.getByTestId("working").waitFor({ state: "detached" });
  });

  it.skipIf(usesRealServer)("marks the chat read once per answer, not at every piece", async () => {
    await open("/chat");
    fake().autoAnswer = false;
    await say("Hello");
    await page.waitForURL(/\/chat\/\d+$/);
    const run = fake().chats[0]?.id as number;
    fake().agentStart(run);
    for (let n = 0; n < 30; n++) fake().delta(run, `piece ${n} `);
    await page.getByText("piece 29").waitFor();
    expect(fake().seen).toEqual([]);

    fake().settle(run, "piece");
    await expect.poll(() => fake().seen.length).toBe(1);
    expect(fake().seen[0]?.chat).toBe(run);

    // The second answer is marked once more.
    await say("Again");
    fake().agentStart(run);
    for (let n = 0; n < 10; n++) fake().delta(run, `more ${n} `);
    fake().settle(run, "more");
    await expect.poll(() => fake().seen.length).toBe(2);
    expect(fake().seen[1]?.seq).toBeGreaterThan(fake().seen[0]?.seq ?? 0);
  });
});

describe("what the agent does", () => {
  it.skipIf(usesRealServer)(
    "shows tool calls as pills, reads together and an edit as a diff",
    async () => {
      await open("/chat");
      fake().autoAnswer = false;
      await say("Fix it");
      await page.waitForURL(/\/chat\/\d+$/);
      const run = fake().chats[0]?.id as number;
      fake().agentStart(run);
      fake().thinking(run, "I should read first.");
      fake().toolCall(run, {
        id: "r1",
        tool: "read",
        args: { path: "src/a.rs" },
        output: "fn a() {}",
      });
      fake().toolCall(run, {
        id: "r2",
        tool: "read",
        args: { path: "src/b.rs" },
        output: "fn b() {}",
      });
      fake().toolCall(run, {
        id: "b1",
        tool: "bash",
        args: { command: "cargo test --quiet" },
        output: "test result: ok. 3 passed",
      });
      fake().toolCall(run, {
        id: "e1",
        tool: "edit",
        args: {
          path: "src/a.rs",
          edits: [{ oldText: "fn a() {}", newText: "fn a() -> u8 { 1 }" }],
        },
        output: "edited",
      });

      const steps = page.getByTestId("steps");
      await steps.getByRole("button", { name: /Read · 2 files/ }).waitFor();
      await steps.getByText("bash", { exact: true }).waitFor();
      await steps.getByText("cargo test --quiet").waitFor();
      // The thinking is folded until it is opened.
      expect(await page.getByText("I should read first.").count()).toBe(0);
      await steps.getByRole("button", { name: "Thinking" }).click();
      await page.getByText("I should read first.").waitFor();

      // A pill opens to show what the tool printed.
      await steps.getByRole("button", { name: /cargo test --quiet/ }).click();
      await page.getByText("test result: ok. 3 passed").waitFor();

      // The edit is a file with its added and removed lines.
      const file = steps.getByRole("button", { name: /src\/a\.rs/ });
      await file.waitFor();
      await file.click();
      await page.getByText("+fn a() -> u8 { 1 }").waitFor();
      await page.getByText("-fn a() {}").waitFor();
    },
  );

  it.skipIf(usesRealServer)(
    "shows an answer as Markdown with a highlighted code block",
    async () => {
      await open("/chat");
      fake().autoAnswer = false;
      await say("Show code");
      await page.waitForURL(/\/chat\/\d+$/);
      const run = fake().chats[0]?.id as number;
      fake().agentStart(run);
      fake().delta(
        run,
        "## Result\n\nIt is **done**:\n\n```js\nconst answer = 42;\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n",
      );
      fake().settle(run);
      await page.getByRole("heading", { name: "Result" }).waitFor();
      await page.getByText("done", { exact: true }).waitFor();
      await page.locator("pre .hljs-keyword").filter({ hasText: "const" }).waitFor();
      await page.getByRole("table").getByRole("cell", { name: "2" }).waitFor();
    },
  );

  it.skipIf(usesRealServer)("shows an error of the model's provider in the chat", async () => {
    await open("/chat");
    fake().autoAnswer = false;
    await say("Hello");
    await page.waitForURL(/\/chat\/\d+$/);
    const run = fake().chats[0]?.id as number;
    fake().agentStart(run);
    fake().providerError(run, '401 {"error":"OAuth token has been refused by the provider"}');
    const alert = page.getByRole("alert").filter({ hasText: "The model returned an error" });
    await alert.waitFor();
    await alert
      .getByText("OAuth token has been refused by the provider", { exact: false })
      .waitFor();
    // The error is one entry, though Pi says it twice (the message and the end of the turn).
    expect(await alert.count()).toBe(1);
  });

  it.skipIf(usesRealServer)("shows the run's own error too", async () => {
    await open("/chat");
    const id = fake().seed({
      title: "Broken",
      status: "failed",
      events: [
        ["said", { message: "Broken" }],
        ["moebius_error", { message: "cannot start pi-bolt: not found" }],
      ],
    });
    await page.goto(`${server.url}/chat/${id}`);
    await page
      .getByRole("alert")
      .filter({ hasText: "The run failed" })
      .getByText("cannot start pi-bolt: not found")
      .waitFor();
  });
});

describe("copying", () => {
  it.skipIf(usesRealServer)("puts a message and a code block on the clipboard", async () => {
    await open("/chat");
    fake().autoAnswer = false;
    await say("Copy me");
    await page.waitForURL(/\/chat\/\d+$/);
    const run = fake().chats[0]?.id as number;
    fake().agentStart(run);
    fake().delta(run, "Here is code:\n\n```sh\necho hello\n```\n\nThat is all.");
    fake().settle(run);
    await page.getByText("That is all.").waitFor();

    await page.getByRole("button", { name: "Copy message" }).click();
    await page.getByRole("button", { name: "Copy message" }).getByText("Copied").waitFor();
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(
      "Here is code:\n\n```sh\necho hello\n```\n\nThat is all.",
    );

    await page.getByRole("button", { name: "Copy code" }).click();
    await page.getByRole("button", { name: "Copy code" }).getByText("Copied").waitFor();
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("echo hello");

    await page.getByRole("button", { name: "Copy your message" }).click();
    await page.getByRole("button", { name: "Copy your message" }).getByText("Copied").waitFor();
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("Copy me");
  });
});

describe("a new chat", () => {
  it.skipIf(usesRealServer)("with the Moebius agent starts without a model", async () => {
    await open("/chat");
    // The agent has no model of its own, and the composer does not ask for one.
    await page.getByRole("button", { name: "Agent: Moebius" }).waitFor();
    await page.getByRole("button", { name: "Model: Server default" }).waitFor();
    await say("Hello Moebius");
    await page.waitForURL(/\/chat\/\d+$/);
    await page.getByText("You said: Hello Moebius").first().waitFor();
    expect(fake().started).toEqual([{ agent: "Moebius", message: "Hello Moebius" }]);
  });

  it.skipIf(usesRealServer)(
    "sends the agent, repository, model and reasoning that were chosen",
    async () => {
      await open("/chat");
      await page.getByRole("button", { name: "Repository: No repository" }).click();
      await page.getByRole("menuitemradio", { name: "demo" }).click();
      await page.getByRole("button", { name: /^Model: / }).click();
      await page.getByRole("textbox", { name: "Filter models" }).fill("small");
      await page.getByRole("menuitemradio", { name: "fake-small" }).click();
      await page.getByRole("button", { name: /^Reasoning: / }).click();
      await page.getByRole("menuitemradio", { name: "high", exact: true }).click();
      await page.getByRole("button", { name: "Agent: Moebius" }).click();
      await page.getByRole("menuitemradio", { name: /Coder/ }).click();
      await say("Build it");
      await page.waitForURL(/\/chat\/\d+$/);
      expect(fake().started).toEqual([
        {
          agent: "Coder",
          message: "Build it",
          repository: 1,
          model: "fake-small",
          thinking: "high",
        },
      ]);
    },
  );

  it.skipIf(usesRealServer)("sends with Enter and makes a new line with Shift+Enter", async () => {
    await open("/chat");
    await field().fill("first");
    await field().press("Shift+Enter");
    await field().pressSequentially("second");
    expect(fake().started).toEqual([]);
    expect(await field().inputValue()).toBe("first\nsecond");
    await field().press("Enter");
    await page.waitForURL(/\/chat\/\d+$/);
    expect(fake().started[0]?.message).toBe("first\nsecond");
  });

  it.skipIf(usesRealServer)(
    "shows the server's refusal above the field and keeps the text",
    async () => {
      await open("/chat");
      await page.getByRole("button", { name: "Agent: Moebius" }).click();
      await page.getByRole("menuitemradio", { name: /Grilling-partner/ }).click();
      await say("Question me");
      // A grilling needs a repository, so Send waits until one is chosen.
      expect(fake().started).toEqual([]);
      await page.getByRole("button", { name: "Repository: No repository" }).click();
      await page.getByRole("menuitemradio", { name: "demo" }).click();
      await field().press("Enter");
      await page.waitForURL(/\/chat\/\d+$/);
      expect(fake().started[0]).toMatchObject({ agent: "Grilling-partner", repository: 1 });
    },
  );
});

describe("an open chat", () => {
  it.skipIf(usesRealServer)("changes the model while it waits", async () => {
    await open("/chat");
    const id = fake().seed({ title: "Hi", events: [["said", { message: "Hi" }]] });
    await page.goto(`${server.url}/chat/${id}`);
    await page.getByRole("button", { name: /^Model: / }).click();
    await page.getByRole("menuitemradio", { name: "fake-small" }).click();
    await page.getByRole("button", { name: "Model: fake-small" }).waitFor();
    expect(fake().chats[0]?.model).toBe("fake-small");
  });

  it.skipIf(usesRealServer)(
    "stops after asking, and writes to an ended chat in a new one",
    async () => {
      await open("/chat");
      fake().autoAnswer = false;
      await say("Long job");
      await page.waitForURL(/\/chat\/\d+$/);
      const run = fake().chats[0]?.id as number;
      fake().agentStart(run);
      await page.getByRole("button", { name: "Stop" }).click();
      await page.getByText("Stop this chat?").waitFor();
      await page.getByRole("button", { name: "Keep going" }).click();
      await page.getByRole("button", { name: "Stop" }).click();
      await page.getByRole("button", { name: "Stop chat" }).click();
      await page.getByTestId("chat-title").waitFor();
      await page.getByText("This chat has ended").waitFor();
      expect(fake().chats[0]?.status).toBe("stopped");
    },
  );
});

describe("the floating chat", () => {
  it.skipIf(usesRealServer)(
    "opens over another page and tells the agent where the person is",
    async () => {
      await open("/inbox");
      await page.getByRole("button", { name: "Open the floating chat" }).click();
      const window = page.getByRole("dialog", { name: "Floating chat" });
      await window.waitFor();
      await window.getByRole("textbox", { name: "Message" }).fill("What is this page?");
      await window.getByRole("textbox", { name: "Message" }).press("Enter");
      await window.getByText("You said: What is this page?").first().waitFor();

      expect(fake().started).toHaveLength(1);
      expect(fake().started[0]).toMatchObject({
        agent: "Moebius",
        message: "What is this page?",
        origin: "assistant",
        context: "Page: Inbox",
      });

      // The chat stays across pages, and the next message goes to the same chat.
      await page
        .getByRole("navigation", { name: "Pages" })
        .getByRole("link", { name: "Retro" })
        .click();
      await page.waitForURL("**/retro");
      // The address changes before the page is drawn; a person writes once the Retro is on the screen.
      await page.getByRole("heading", { name: "Retro", level: 1 }).waitFor();
      await window.getByText("You said: What is this page?").first().waitFor();
      await window.getByRole("textbox", { name: "Message" }).fill("And this one?");
      await window.getByRole("textbox", { name: "Message" }).press("Enter");
      await window.getByText("You said: And this one?").first().waitFor();
      expect(fake().started).toHaveLength(1);
      expect(fake().said[0]?.body).toMatchObject({
        message: "And this one?",
        context: "Page: Improve > Retro",
      });

      // Minimised, it is the button again; the Chat page does not list this chat.
      await window.getByRole("button", { name: "Minimise the floating chat" }).click();
      await window.waitFor({ state: "detached" });
      await page.goto(`${server.url}/chat`);
      await page.getByRole("button", { name: "Chats (0)" }).waitFor();
    },
  );

  it.skipIf(usesRealServer)("starts in the repository of the board it is opened on", async () => {
    await open("/board/1");
    await page.getByRole("button", { name: "Open the floating chat" }).click();
    const window = page.getByRole("dialog", { name: "Floating chat" });
    await window.getByRole("textbox", { name: "Message" }).fill("Look at this board");
    await window.getByRole("textbox", { name: "Message" }).press("Enter");
    await window.getByText("You said: Look at this board").first().waitFor();
    expect(fake().started[0]).toMatchObject({ repository: 1 });
    expect(String(fake().started[0]?.context)).toContain("Repository: demo (id 1)");
  });

  it("opens in the real server too, with the page's text", async () => {
    await open("/inbox");
    await page.getByRole("button", { name: "Open the floating chat" }).click();
    await page.getByRole("dialog", { name: "Floating chat" }).waitFor();
    await page.getByText("Ask the agent about this page.").waitFor();
  });
});

describe("a message with a picture", () => {
  /** A PNG one pixel square, the smallest thing a model could be shown. */
  const PNG = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==",
    "base64",
  );
  const clip = () => page.getByRole("button", { name: "Attach a picture" });
  const held = () => page.getByLabel("Pictures with this message");

  it.skipIf(usesRealServer)(
    "sends a picked picture with the message and shows it in the thread",
    async () => {
      await open("/chat");
      fake().autoAnswer = false;
      const id = fake().seed({
        title: "Look",
        agent: "Coder",
        model: "fake-coder",
        events: [["said", { message: "Look at this" }]],
      });
      await page.goto(`${server.url}/chat/${id}`);
      await clip().waitFor();
      await page
        .locator("input[type=file]")
        .setInputFiles({ name: "screen.png", mimeType: "image/png", buffer: PNG });
      await held().getByRole("img", { name: "screen.png" }).waitFor();

      await say("What is in this picture?");
      await expect.poll(() => fake().said.length).toBe(1);
      // The server is told the name, the kind and the bytes, which is what it keeps and serves again.
      expect(fake().said[0]?.body.images).toEqual([
        { name: "screen.png", media_type: "image/png", data: PNG.toString("base64") },
      ]);
      // The message is shown with the picture the chat's own directory serves.
      const shown = page.getByRole("img", { name: "screen.png" });
      await expect.poll(async () => shown.count()).toBe(1);
      expect(await shown.first().getAttribute("src")).toContain("/images/");
    },
  );

  it.skipIf(usesRealServer)("keeps a picture pasted into the field", async () => {
    await open("/chat");
    fake().autoAnswer = false;
    const id = fake().seed({ title: "Paste", events: [["said", { message: "Paste" }]] });
    await page.goto(`${server.url}/chat/${id}`);
    await clip().waitFor();
    await page.evaluate(async (bytes: number[]) => {
      const file = new File([new Uint8Array(bytes)], "pasted.png", { type: "image/png" });
      const carried = new DataTransfer();
      carried.items.add(file);
      const area = document.querySelector<HTMLTextAreaElement>('textarea[aria-label="Message"]');
      area?.dispatchEvent(
        new ClipboardEvent("paste", { clipboardData: carried, bubbles: true, cancelable: true }),
      );
    }, Array.from(PNG));
    await held().getByRole("img", { name: "pasted.png" }).waitFor();
    // The paste did not reach the field as text.
    await expect.poll(() => field().inputValue()).toBe("");

    await say("And in this one?");
    await expect.poll(() => fake().said.length).toBe(1);
    expect(fake().said[0]?.body.images).toEqual([
      { name: "pasted.png", media_type: "image/png", data: PNG.toString("base64") },
    ]);
  });

  it.skipIf(usesRealServer)("takes a picture off again before sending", async () => {
    await open("/chat");
    fake().autoAnswer = false;
    const id = fake().seed({ title: "Off", events: [["said", { message: "Off" }]] });
    await page.goto(`${server.url}/chat/${id}`);
    await page
      .locator("input[type=file]")
      .setInputFiles({ name: "oops.png", mimeType: "image/png", buffer: PNG });
    await held().getByRole("img", { name: "oops.png" }).waitFor();
    await held().getByRole("button", { name: "Take oops.png off the message" }).click();
    await held().waitFor({ state: "detached" });

    await say("Words only");
    await expect.poll(() => fake().said.length).toBe(1);
    expect(fake().said[0]?.body.images).toBeUndefined();
  });

  it.skipIf(usesRealServer)(
    "says why a model that answers in words only is offered no picture",
    async () => {
      await open("/chat");
      fake().autoAnswer = false;
      const id = fake().seed({
        title: "Words",
        agent: "Coder",
        model: "fake-small",
        events: [["said", { message: "Words" }]],
      });
      await page.goto(`${server.url}/chat/${id}`);
      await page.getByTestId("person-message").waitFor();
      await page.getByText("answers in words only").waitFor();
      expect(await clip().count()).toBe(0);
    },
  );
});
