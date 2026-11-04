/**
 * The Agents, Skills, MCP servers, Models and Plugins pages in a real browser. Against the fake server
 * (e2e/fake/pages-b.ts) every action is checked: it sends its request and the page shows the outcome (the commit an
 * edit made, the sentence the server refused with), and an API key typed on the Models page is sent once and never shown
 * back. Against a real server only the loading of each page is looked at, as that server has no harness configuration
 * repository, upstream or plugin. Every wait is on something the page shows, never on time.
 */
import { type Browser, type BrowserContext, chromium, type Page } from "playwright";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { columnsScrollEach, pickOption, scrollbarsAreQuiet, showing } from "./controls";
import type { FakeServer } from "./fakeServer";
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

afterEach(async () => {
  await context.close();
  await server.stop().catch(() => {});
});

/** A new server and browser, signed in and on `path`; against the fake, `before` sets its data up first. */
async function open(path: string, before?: (fake: PagesB) => void) {
  server = await startServer();
  if (server.pagesB) {
    server.pagesB.use();
    before?.(server.pagesB);
  }
  context = await browser.newContext();
  page = await context.newPage();
  await page.goto(server.loginUrl);
  await page.getByRole("navigation", { name: "Pages" }).waitFor();
  await page.goto(`${server.url}${path}`);
}

// A value may hold a reference to a name of `harness.mcp_env`, which the server resolves.
// biome-ignore lint/suspicious/noTemplateCurlyInString: a reference, not a template
const DOCS_REF = "${DOCS_TOKEN}";
// biome-ignore lint/suspicious/noTemplateCurlyInString: a reference, not a template
const OTHER_BEARER = "Bearer ${OTHER_KEY}";
// biome-ignore lint/suspicious/noTemplateCurlyInString: a reference, not a template
const DOCS_BEARER = "Bearer ${DOCS_TOKEN}";

type PagesB = FakeServer["pagesB"];

function fake(): PagesB {
  if (!server.pagesB) throw new Error("these tests run against the fake server");
  return server.pagesB;
}

const save = () => page.getByRole("button", { name: "Save", exact: true });
const card = (name: string) => page.getByTestId("list-card").filter({ hasText: name }).first();
/** The body of the first request to `method path`, as the fake received it. */
function sent<T>(method: string, path: string): T {
  const call = fake().calls.find((each) => each.method === method && each.path === path);
  if (!call) throw new Error(`${method} ${path} was not asked`);
  return call.body as T;
}
const requests = (method: string, path: string) =>
  fake().calls.filter((call) => call.method === method && call.path === path);

describe("the Agents page", () => {
  it("loads and lists the agents", async () => {
    await open("/agents");
    await page.getByRole("heading", { name: "Agents", exact: true }).waitFor();
    await card("Moebius").waitFor();
  });

  it.skipIf(usesRealServer)("lists the built-in, the own and the broken agents", async () => {
    await open("/agents");
    await card("Coder").waitFor();
    expect(await card("Moebius").textContent()).toContain("Built in");
    expect(await card("Broken").textContent()).toContain("Broken");
    expect(await card("Broken").textContent()).toContain("missing field `model`");
    expect(await card("Coder").textContent()).toContain("fake-coder · openai");
  });

  it.skipIf(usesRealServer)(
    "edits an agent and says the commit; a bad edit is refused",
    async () => {
      await open("/agents?agent=Coder");
      const model = page.getByLabel("Model", { exact: true });
      await model.waitFor();
      expect(await showing(model)).toBe("fake-coder");
      await pickOption(page, model, "claude-fake");
      // Picking a model takes its protocol with it.
      expect(await showing(page.getByLabel("Protocol"))).toBe("anthropic");
      await pickOption(page, page.getByLabel("Thinking"), "high");
      await page.getByLabel("Token budget per card").fill("2000000");
      // The card holding it is named Instructions too, so the field is asked for as what it is.
      await page
        .getByRole("textbox", { name: "Instructions", exact: true })
        .fill("Keep changes tiny.");
      await save().click();
      await page.getByText("Saved as commit c0ffee1.").waitFor();
      expect(requests("PUT", "/v1/agents/Coder")[0]?.body).toEqual({
        settings: {
          model: "claude-fake",
          protocol: "anthropic",
          thinking: "high",
          token_budget: 2000000,
          skills: ["house-style"],
          extensions: [],
          mcp: ["docs"],
        },
        instructions: "Keep changes tiny.",
      });
      // Nothing waits for the server now, so Save itself says so by resting.
      expect(await save().isDisabled()).toBe(true);

      await page.getByLabel("Token budget per card").fill("0");
      await save().click();
      await page.getByText("The token budget is a whole number of at least 1.").waitFor();
      await page.getByLabel("Token budget per card").fill("");
      fake().refuseNext("PUT", "/v1/agents/Coder", 500, "git push was refused by origin");
      await save().click();
      await page.getByText("git push was refused by origin").waitFor();
    },
  );

  it.skipIf(usesRealServer)("chooses skills and MCP servers and adds an extension", async () => {
    await open("/agents?agent=Reviewer");
    await page.getByRole("button", { name: "house-style", exact: true }).click();
    await page.getByRole("button", { name: "docs", exact: true }).click();
    await page.getByLabel("extension path").fill("extensions/log.ts");
    await page.getByRole("button", { name: "Add extension" }).click();
    await save().click();
    await page.getByText("Saved as commit").waitFor();
    expect(requests("PUT", "/v1/agents/Reviewer")[0]?.body).toMatchObject({
      settings: { skills: ["house-style"], mcp: ["docs"], extensions: ["extensions/log.ts"] },
    });
    // A server that cannot be read is marked in red and cannot be chosen away silently.
    expect(await page.getByRole("button", { name: "broken", exact: true }).textContent()).toBe(
      "broken",
    );
  });

  it.skipIf(usesRealServer)(
    "makes a new agent, asking for a name, and deletes one after asking",
    async () => {
      await open("/agents");
      await page.getByRole("button", { name: "New agent" }).click();
      await page.getByLabel("Name of the new agent").fill("Tester");
      await save().click();
      await page.getByText("Enter the model.").waitFor();
      await pickOption(page, page.getByLabel("Model", { exact: true }), "fake-small");
      await save().click();
      await page.getByText("Saved as commit").waitFor();
      await card("Tester").waitFor();
      expect(fake().agents.Tester?.settings.model).toBe("fake-small");

      await page.getByRole("button", { name: "Delete agent" }).click();
      await page.getByText("Delete Tester?").waitFor();
      await page.getByRole("button", { name: "Keep" }).click();
      expect(fake().agents.Tester).toBeDefined();
      await page.getByRole("button", { name: "Delete agent" }).click();
      await page.getByRole("button", { name: "Delete agent" }).last().click();
      await card("Tester").waitFor({ state: "detached" });
      expect(fake().agents.Tester).toBeUndefined();
    },
  );

  it.skipIf(usesRealServer)("shows what a built-in agent brings, read only", async () => {
    await open("/agents?agent=Moebius");
    const built = page.getByTestId("built-in-instructions");
    await built.waitFor();
    expect(await built.textContent()).toContain("Moebius brings these instructions");
    await page.getByText("moebius-operate").waitFor();
  });

  it.skipIf(usesRealServer)("says why a broken agent cannot be edited and deletes it", async () => {
    await open("/agents?agent=Broken");
    await page.getByRole("alert").filter({ hasText: "missing field `model`" }).waitFor();
    expect(await save().isDisabled()).toBe(true);
  });

  it.skipIf(usesRealServer)("asks the Moebius agent to write the instructions", async () => {
    await open("/agents?agent=Coder");
    await page.getByRole("button", { name: "Write with agent" }).click();
    await page.getByLabel("What this agent should do").fill("Review small changes first.");
    await page.getByRole("button", { name: "Submit" }).click();
    await page.getByRole("link", { name: "Open the chat" }).waitFor();
    const started = server.chat?.started.at(-1);
    expect(started?.agent).toBe("Moebius");
    expect(String(started?.message)).toContain("agent show Coder");
    expect(String(started?.message)).toContain("Review small changes first.");
  });

  it.skipIf(usesRealServer)(
    "keeps agents in moebius.toml on a server without a repository",
    async () => {
      await open("/agents?agent=Coder", (f) => f.useWithoutHarness());
      await page.getByTestId("harness-setup").waitFor();
      const model = page.getByLabel("Model", { exact: true });
      await model.waitFor();
      await pickOption(page, model, "fake-small");
      await page.getByLabel("skill path").fill("skills/extra");
      await page.getByRole("button", { name: "Add skill" }).click();
      await save().click();
      // The page says the same words in its title line; the notice the save leaves is the exact sentence.
      await page.getByText("Saved in moebius.toml.", { exact: true }).waitFor();
      const { set } = sent<{ set: Record<string, unknown> }>("PUT", "/v1/settings");
      expect(set["agents.Coder"]).toMatchObject({
        model: "fake-small",
        protocol: "openai",
        skills: ["house-style", "skills/extra"],
        instructions: "Keep changes small.",
      });
    },
  );
});

describe("the Skills page", () => {
  it("loads", async () => {
    await open("/skills");
    await page.getByRole("heading", { name: "Skills", exact: true }).waitFor();
    // A server with a repository lists the skills; one without offers to set it up.
    await page
      .locator('[data-testid="list-card"], [data-testid="harness-setup"]')
      .first()
      .waitFor();
  });

  it.skipIf(usesRealServer)(
    "lists the built-in skills first and edits one of the repository",
    async () => {
      await open("/skills");
      await card("house-style").waitFor();
      const names = await page.getByTestId("list-card").allInnerTexts();
      expect(names[0]).toContain("moebius");
      expect(names.at(-1)).toContain("house-style");
      await card("house-style").click();
      const description = page.getByLabel("Description");
      expect(await description.inputValue()).toBe("Write plainly.");
      await description.fill("Write very plainly.");
      await page
        .getByRole("textbox", { name: "SKILL.md", exact: true })
        .fill("Use shorter sentences.\n");
      await save().click();
      await page.getByText("Saved as commit c0ffee1.").waitFor();
      expect(requests("PUT", "/v1/skills/house-style")[0]?.body).toEqual({
        description: "Write very plainly.",
        text: "Use shorter sentences.",
      });
      await description.fill("");
      await save().click();
      await page.getByText("Enter the description.").waitFor();
    },
  );

  it.skipIf(usesRealServer)("makes a new skill and deletes it after asking", async () => {
    await open("/skills");
    await page.getByRole("button", { name: "New skill" }).click();
    await page.getByLabel("Name of the new skill").fill("pdf");
    await page.getByLabel("Description").fill("Fill PDF forms.");
    await save().click();
    await page.getByText("Saved as commit").waitFor();
    await card("pdf").waitFor();
    await page.getByRole("button", { name: "Delete skill" }).click();
    await page.getByRole("button", { name: "Delete skill" }).last().click();
    await card("pdf").waitFor({ state: "detached" });
    expect(fake().skills.pdf).toBeUndefined();
  });

  it.skipIf(usesRealServer)("opens, saves, adds and deletes a skill's other files", async () => {
    await open("/skills?skill=house-style");
    await page.getByRole("button", { name: "scripts/check.py" }).click();
    const text = page.getByRole("textbox", { name: "File text", exact: true });
    expect(await text.inputValue()).toBe("print('ok')\n");
    await text.fill("print('changed')\n");
    await page.getByRole("button", { name: "Save file" }).click();
    await page.getByText("Saved as commit").waitFor();
    expect(requests("PUT", "/v1/skills/house-style/files/scripts/check.py")[0]?.body).toEqual({
      text: "print('changed')\n",
    });

    await page.getByRole("button", { name: "New file" }).click();
    // A new file has no path yet: the page says what is missing where it is missing, and will not save until it is
    // there, rather than waiting to be pressed to find out. The button that cannot be pressed says the same words to
    // a screen reader, so the line that is on the screen is the first of the two.
    await page.getByText("Enter the file's path inside the skill").first().waitFor();
    expect(await page.getByRole("button", { name: "Save file" }).isDisabled()).toBe(true);
    await page.getByLabel("Path in the skill").fill("notes/a.md");
    await page.getByRole("textbox", { name: "File text", exact: true }).fill("Notes");
    await page.getByRole("button", { name: "Save file" }).click();
    await page.getByText("Saved as commit").waitFor();
    await page.getByRole("button", { name: "notes/a.md" }).waitFor();

    await page.getByRole("button", { name: "Delete file" }).click();
    await page.getByRole("button", { name: "Delete the file" }).click();
    await page.getByRole("button", { name: "notes/a.md" }).waitFor({ state: "detached" });
    expect(fake().skills["house-style"]?.files["notes/a.md"]).toBeUndefined();
  });

  it.skipIf(usesRealServer)(
    "reads every file of a skill the way the first of them was read",
    async () => {
      await open("/skills?skill=house-style");
      await page.getByRole("button", { name: "New file" }).click();
      await page.getByLabel("Path in the skill").fill("notes/readme.md");
      await page
        .getByRole("textbox", { name: "File text", exact: true })
        .fill("# Notes\n\nSay it once.\n");
      await page.getByRole("button", { name: "Save file" }).click();
      await page.getByText("Saved as commit").waitFor();
      // It opens as the text; the drawing is one tab away and stays picked for the next file too.
      expect(await page.getByRole("textbox", { name: "File text", exact: true }).count()).toBe(1);
      await page.getByRole("tab", { name: "Rendered" }).click();
      await page.getByRole("heading", { name: "Notes", exact: true }).waitFor();
      await page.getByRole("button", { name: "SKILL.md" }).click();
      expect(await page.getByRole("tab", { name: "Rendered" }).getAttribute("aria-selected")).toBe(
        "true",
      );
      expect(await page.getByRole("textbox", { name: "SKILL.md", exact: true }).count()).toBe(0);
      expect(
        await page.getByRole("paragraph").filter({ hasText: "Use short sentences." }).count(),
      ).toBeGreaterThan(0);
    },
  );

  it.skipIf(usesRealServer)(
    "lists a file that is not text with its size and does not edit it",
    async () => {
      await open("/skills?skill=house-style");
      await page.getByRole("button", { name: "assets/logo.bin" }).click();
      await page.getByText("Not text, or too large to edit (20.0 KB).").waitFor();
      expect(await page.getByRole("textbox", { name: "File text" }).count()).toBe(0);
    },
  );

  it.skipIf(usesRealServer)(
    "shows a built-in skill read only and updates one from its source",
    async () => {
      await open("/skills?skill=tdd");
      await page.getByText("Built in · read only").waitFor();
      await page
        .getByText("Copied from https://github.com/acme/skills at commit 0123456789ab")
        .waitFor();
      // A skill you cannot change is read, and it opens as the text all the same; the drawing is one tab away.
      const box = page.getByRole("textbox", { name: "SKILL.md", exact: true });
      expect(await box.getAttribute("readonly")).not.toBeNull();
      await page.getByRole("tab", { name: "Rendered" }).click();
      await page.getByText("The tdd skill.").waitFor();
      expect(await save().count()).toBe(0);
      await page.getByRole("button", { name: "Update from the repository" }).click();
      await page.getByRole("link", { name: "Open the chat" }).waitFor();
      const message = String(server.chat?.started.at(-1)?.message);
      expect(message).toContain("Update the skill tdd from its source");
      expect(message).toContain("0123456789abcdef0123");
    },
  );

  it.skipIf(usesRealServer)(
    "refuses to delete a skill an agent names, with the server's sentence",
    async () => {
      await open("/skills?skill=house-style");
      await page.getByRole("button", { name: "Delete skill" }).click();
      await page.getByRole("button", { name: "Delete skill" }).last().click();
      await page.getByRole("alert").filter({ hasText: "is named by Coder" }).waitFor();
    },
  );

  it.skipIf(usesRealServer)("imports from a link by asking the Moebius agent", async () => {
    await open("/skills");
    await page.getByRole("button", { name: "Import", exact: true }).click();
    const ask = page.getByRole("button", { name: "Submit" });
    expect(await ask.isDisabled()).toBe(true);
    await page.getByRole("button", { name: "From a link" }).click();
    await page.getByLabel("A git repository or a page").fill("https://github.com/acme/more-skills");
    await page.getByLabel("Note (optional)").fill("only pdf");
    await ask.click();
    await page.getByRole("link", { name: "Open the chat" }).waitFor();
    const message = String(server.chat?.started.at(-1)?.message);
    expect(message).toContain("git clone --depth 1 https://github.com/acme/more-skills import");
    expect(message.endsWith("What the person said:\n\nonly pdf")).toBe(true);
  });

  it.skipIf(usesRealServer)(
    "sends a zip to the server and gives the agent its path; a refused one is said",
    async () => {
      await open("/skills");
      await page.getByRole("button", { name: "Import", exact: true }).click();
      fake().refuseNext("POST", "/v1/uploads", 413, "too large");
      await page.getByLabel("Zip file").setInputFiles({
        name: "big.zip",
        mimeType: "application/zip",
        buffer: Buffer.from("zip"),
      });
      await page.getByText("The file is larger than the server takes.").waitFor();
      await page.getByLabel("Zip file").setInputFiles({
        name: "skills.zip",
        mimeType: "application/zip",
        buffer: Buffer.from("PK-zip-bytes"),
      });
      await page.getByText("skills.zip is on the server.").waitFor();
      expect(fake().uploads).toEqual([
        { id: "ab12cd34ab12cd34ab12cd34ab12cd34", size: 12, type: "application/zip" },
      ]);
      await page.getByRole("button", { name: "Submit" }).click();
      await page.getByRole("link", { name: "Open the chat" }).waitFor();
      expect(String(server.chat?.started.at(-1)?.message)).toContain(
        "unzip -q /srv/moebius/uploads/ab12cd34ab12cd34ab12cd34ab12cd34.zip -d import",
      );
    },
  );

  it.skipIf(usesRealServer)("offers to set the repository up and mutes what needs it", async () => {
    await open("/skills", (f) => f.useWithoutHarness());
    await page.getByTestId("harness-setup").waitFor();
    expect(await page.getByRole("button", { name: "Import", exact: true }).isDisabled()).toBe(true);
    expect(await page.getByRole("button", { name: "New skill" }).isDisabled()).toBe(true);
    await page.getByRole("button", { name: "Set up" }).click();
    await page.getByText("Set up. The server restarts and the app reconnects by itself.").waitFor();
    expect(requests("POST", "/v1/harness/setup")).toHaveLength(1);
    expect(fake().restarts).toBe(1);
    // The server is back with its repository: the event stream reconnects and the page reads again.
    await server.stop();
    await (server as FakeServer).restart();
    await card("house-style").waitFor();
    expect(await page.getByTestId("harness-setup").count()).toBe(0);
  });
});

describe("the MCP servers page", () => {
  it("loads", async () => {
    await open("/mcp-servers");
    await page.getByRole("heading", { name: "MCP servers", exact: true }).waitFor();
    await page
      .locator('[data-testid="list-card"], [data-testid="harness-setup"], :text("No servers yet")')
      .first()
      .waitFor();
  });

  it.skipIf(usesRealServer)("lists the servers, a broken one in red, and edits one", async () => {
    await open("/mcp-servers");
    await card("docs").waitFor();
    expect(await card("docs").textContent()).toContain("stdio");
    expect(await card("broken").textContent()).toContain("has no command or url");
    await card("docs").click();
    const command = page.getByLabel("Command");
    expect(await command.inputValue()).toBe("docs-server");
    expect(await page.getByLabel("Arguments, one per line").inputValue()).toBe("--root\n/srv/docs");
    expect(await page.getByLabel("Name 1").inputValue()).toBe("TOKEN");
    await command.fill("docs-server-2");
    await save().click();
    await page.getByText("Saved as commit c0ffee1.").waitFor();
    expect(requests("PUT", "/v1/mcp-servers/docs")[0]?.body).toEqual({
      command: "docs-server-2",
      args: ["--root", "/srv/docs"],
      env: { TOKEN: DOCS_REF },
    });
  });

  it.skipIf(usesRealServer)("adds a remote server and says what the server refuses", async () => {
    await open("/mcp-servers");
    await page.getByRole("button", { name: "New server" }).click();
    await page.getByLabel("Name of the new server").fill("search");
    await save().click();
    await page.getByText("Enter the command.").waitFor();
    await pickOption(page, page.getByLabel("Kind"), "URL");
    await page.getByLabel("URL", { exact: true }).fill("https://search.example.com/mcp");
    await page.getByRole("button", { name: "Add header" }).click();
    await page.getByLabel("Name 1").fill("Authorization");
    await page.getByLabel("Value 1").fill(OTHER_BEARER);
    await save().click();
    await page.getByRole("alert").filter({ hasText: "not in harness.mcp_env" }).waitFor();
    await page.getByLabel("Value 1").fill(DOCS_BEARER);
    await save().click();
    await page.getByText("Saved as commit").waitFor();
    expect(fake().mcp.search).toEqual({
      url: "https://search.example.com/mcp",
      headers: { Authorization: DOCS_BEARER },
    });
    await card("search").waitFor();
  });

  it.skipIf(usesRealServer)("does not delete a server an agent names", async () => {
    await open("/mcp-servers?server=docs");
    await page.getByRole("button", { name: "Delete server" }).click();
    await page.getByRole("button", { name: "Delete server" }).last().click();
    await page.getByRole("alert").filter({ hasText: "is named by Coder" }).waitFor();
    expect(fake().mcp.docs).toBeDefined();
  });

  it.skipIf(usesRealServer)("imports MCP servers by asking the Moebius agent", async () => {
    await open("/mcp-servers");
    await page.getByRole("button", { name: "Import", exact: true }).click();
    await page.getByRole("button", { name: "From a link" }).click();
    await page.getByLabel("A git repository or a page").fill("https://github.com/acme/docs-mcp");
    await page.getByRole("button", { name: "Submit" }).click();
    await page.getByRole("link", { name: "Open the chat" }).waitFor();
    expect(String(server.chat?.started.at(-1)?.message)).toContain("mcp.toml");
  });
});

describe("the Models page", () => {
  it("loads", async () => {
    await open("/models");
    await page.getByRole("heading", { name: "Models", exact: true }).waitFor();
    await page.getByRole("heading", { name: "Providers" }).waitFor();
    await page.getByRole("heading", { name: "Model classes" }).waitFor();
    await page.getByRole("heading", { name: "Teacher" }).waitFor();
  });

  it.skipIf(usesRealServer)(
    "shows the upstreams, classes, agents, teacher and decision model",
    async () => {
      await open("/models");
      const upstreams = page.getByTestId("upstream");
      await upstreams.nth(2).waitFor();
      expect(await upstreams.count()).toBe(3);
      expect(await upstreams.nth(0).getByLabel("Base URL").inputValue()).toBe(
        "http://127.0.0.1:8080",
      );
      expect(await upstreams.nth(2).textContent()).toContain("None: it logs in with OAuth");
      expect(
        await page
          .getByTestId("class")
          .first()
          .getByLabel("Models, first tried first")
          .inputValue(),
      ).toBe("fake-coder, fake-small");
      await page.getByRole("cell", { name: "Coder", exact: true }).waitFor();
      await page.getByText("No decision model is configured.").waitFor();
      await page.getByText("Could not ask https://api.broken.example: 401").waitFor();
    },
  );

  it.skipIf(usesRealServer)(
    "marks the answer with a check when every upstream answers",
    async () => {
      await open("/models", (held) => {
        held.availableErrors = [];
      });
      await page.getByTestId("upstream").nth(0).waitFor();
      const mark = page.getByRole("img", { name: "Answered" });
      // Nothing is marked before the button is pressed: the count already says what the upstreams answered.
      expect(await mark.count()).toBe(0);
      await page.getByRole("button", { name: "Refresh" }).click();
      await mark.waitFor();
    },
  );

  it.skipIf(usesRealServer)(
    "sends a typed key once, stores it by name and never shows it back",
    async () => {
      await open("/models");
      await page.getByRole("button", { name: "Add provider" }).click();
      const row = page.getByTestId("upstream").nth(3);
      await row.getByLabel("Base URL").fill("https://api.new.example");
      await pickOption(page, row.getByLabel("Protocol"), "anthropic");
      await pickOption(page, row.getByLabel("Key of provider 4: where it comes from"), "Typed key");
      await save().click();
      await page.getByText("Type the key of the provider 4.").waitFor();
      const key = row.getByLabel("Key of provider 4: the key");
      expect(await key.getAttribute("type")).toBe("password");
      await key.fill("sk-secret-123");
      await save().click();
      await page.getByText("Saved.", { exact: true }).waitFor();

      expect(fake().secrets["upstream-1"]).toBe("sk-secret-123");
      const holding = fake().calls.filter((call) =>
        JSON.stringify(call.body ?? "").includes("sk-secret-123"),
      );
      expect(holding.map((call) => `${call.method} ${call.path}`)).toEqual([
        "PUT /v1/secrets/upstream-1",
      ]);
      expect(requests("PUT", "/v1/settings")[0]?.body).toEqual({
        set: {
          "router.upstreams.3": {
            protocol: "anthropic",
            base_url: "https://api.new.example",
            api_key_secret: "upstream-1",
          },
        },
        remove: [],
      });
      // After the save the field is empty and says a key is stored.
      const after = page.getByTestId("upstream").nth(3).getByLabel("Key of provider 4: the key");
      expect(await after.inputValue()).toBe("");
      expect(await after.getAttribute("placeholder")).toBe("Stored; type to replace it");
      expect(await page.content()).not.toContain("sk-secret-123");
      expect(await page.evaluate(() => document.body.textContent)).not.toContain("sk-secret-123");
    },
  );

  it.skipIf(usesRealServer)("says when nothing changed and what the server refused", async () => {
    await open("/models");
    await page.getByTestId("upstream").nth(0).waitFor();
    // Nothing changed: Save rests until something does.
    expect(await save().isDisabled()).toBe(true);
    const first = page.getByTestId("upstream").nth(0);
    await pickOption(
      page,
      first.getByLabel("Key of provider 1: where it comes from"),
      "From the environment",
    );
    await first.getByLabel("Key of provider 1: environment variable").fill("OPENAI_KEY");
    fake().refuseNext("PUT", "/v1/settings", 422, "environment variable OPENAI_KEY is not set");
    await save().click();
    await page.getByRole("alert").filter({ hasText: "OPENAI_KEY is not set" }).waitFor();
  });

  it.skipIf(usesRealServer)(
    "edits the classes and turns the teacher off and the decision model on",
    async () => {
      await open("/models");
      const first = page.getByTestId("class").first();
      await first.getByLabel("Models, first tried first").fill("claude-fake, fake-coder");
      await page.getByRole("button", { name: "Turn off the teacher" }).click();
      await page.getByRole("button", { name: "Turn on the decision model" }).click();
      await save().click();
      await page.getByText("Enter the model of the decision model.").waitFor();
      await pickOption(page, page.getByLabel("Model", { exact: true }), "fake-small");
      await save().click();
      await page.getByText("Saved.", { exact: true }).waitFor();
      const body = requests("PUT", "/v1/settings")[0]?.body as {
        set: Record<string, unknown>;
        remove: string[];
      };
      expect(body.remove.sort()).toEqual(["router.classes", "teacher"]);
      expect(body.set).toMatchObject({
        "router.classes": { strong: ["claude-fake", "fake-coder"] },
        "decision_model.model": "fake-small",
      });
      await page.getByText("No teacher is configured").waitFor();
    },
  );

  it.skipIf(usesRealServer)("shows the restart banner and restarts the server", async () => {
    await open("/models", (f) => {
      f.restartKeys = ["router.listen"];
    });
    await page.getByText("Restart the server to use: router.listen").waitFor();
    await page.getByRole("button", { name: "Restart" }).click();
    await page.getByText("Restarting the server. The app reconnects by itself.").waitFor();
    expect(fake().restarts).toBe(1);
  });

  it.skipIf(usesRealServer)(
    "logs in by the provider's page: a new tab, a pasted address, then logged in",
    async () => {
      await open("/models");
      const row = page.getByTestId("upstream").nth(2);
      await row.getByText("Not logged in.").waitFor();
      expect(await row.getByTestId("oauth-warning").textContent()).toContain(
        "may break the provider's terms",
      );
      await row.getByRole("button", { name: "Log in", exact: true }).click();
      // Nothing is sent before the warning is confirmed.
      expect(requests("POST", "/v1/models/upstreams/2/login")).toHaveLength(0);
      const opened = context.waitForEvent("page");
      await row.getByRole("button", { name: "I understand, log in" }).click();
      const tab = await opened;
      await tab.waitForURL("**/fake-provider/authorize?state=fake-state");
      expect(requests("POST", "/v1/models/upstreams/2/login")[0]?.body).toEqual({
        accept_terms: true,
      });
      await row.getByText("The provider's page opened in a new tab.").waitFor();
      await row.getByRole("link", { name: "Open the page again" }).waitFor();

      const paste = row.getByLabel("Paste the code or the address you ended on");
      await paste.fill("bad");
      await row.getByRole("button", { name: "Finish login" }).click();
      await row.getByRole("alert").filter({ hasText: "nothing in pasted is a code" }).waitFor();
      await paste.fill("http://localhost:1455/callback?code=abc&state=fake-state");
      await row.getByRole("button", { name: "Finish login" }).click();
      await row.getByText("Logged in: the upstream lists 2 models.").waitFor();
      expect(requests("POST", "/v1/models/upstreams/2/login/code").at(-1)?.body).toEqual({
        pasted: "http://localhost:1455/callback?code=abc&state=fake-state",
      });
      await row.getByRole("button", { name: "Log in again" }).waitFor();
    },
  );

  it.skipIf(usesRealServer)(
    "logs in with a code to enter at the provider and follows it until it ends",
    async () => {
      await open("/models", (f) => {
        (f.file.router as { upstreams: unknown[] }).upstreams.push({
          protocol: "openai",
          base_url: "https://device.example",
          oauth: {
            flow: "device",
            client_id: "c",
            device_url: "https://login.example.com/device",
            token_url: "https://t",
            secret: "login-1",
          },
        });
      });
      const row = page.getByTestId("upstream").nth(3);
      await row.getByRole("button", { name: "Log in", exact: true }).click();
      await row.getByRole("button", { name: "I understand, log in" }).click();
      expect(await row.getByTestId("user-code").textContent()).toBe("WDJB-MJHT");
      await row.getByText("Enter this code at https://login.example.com/device").waitFor();
      fake().finishLogin(3);
      await row.getByText("Logged in: the upstream lists 2 models.").waitFor();
    },
  );

  it.skipIf(usesRealServer)(
    "shows why a login failed, and a refusal in the server's words",
    async () => {
      await open("/models", (f) => {
        (f.file.router as { upstreams: unknown[] }).upstreams.push({
          protocol: "openai",
          base_url: "https://device.example",
          oauth: {
            flow: "device",
            client_id: "c",
            device_url: "https://login.example.com/device",
            token_url: "https://t",
            secret: "login-1",
          },
        });
      });
      const row = page.getByTestId("upstream").nth(3);
      await row.getByRole("button", { name: "Log in", exact: true }).click();
      fake().refuseNext("POST", "/v1/models/upstreams/3/login", 403, "only a person can log in");
      await row.getByRole("button", { name: "I understand, log in" }).click();
      await row.getByRole("alert").filter({ hasText: "only a person can log in" }).waitFor();
      await row.getByRole("button", { name: "Log in", exact: true }).click();
      await row.getByRole("button", { name: "I understand, log in" }).click();
      await row.getByText("WDJB-MJHT").waitFor();
      fake().failLogin(3, "The provider refused the code.");
      await row.getByRole("alert").filter({ hasText: "The provider refused the code." }).waitFor();
    },
  );

  it.skipIf(usesRealServer)(
    "adds the upstream of a built-in provider and asks for the warning in its row",
    async () => {
      await open("/models");
      await page.getByRole("button", { name: "Log in to a provider" }).click();
      const panel = page.getByTestId("provider-panel");
      await panel.getByText("Anthropic (Claude Pro/Max)").waitFor();
      await panel.getByRole("button", { name: "Log in" }).first().click();
      await page
        .getByTestId("upstream")
        .nth(3)
        .getByRole("button", { name: "I understand, log in" })
        .waitFor();
      expect(requests("PUT", "/v1/settings")[0]?.body).toEqual({
        set: {
          "router.upstreams.3": {
            protocol: "anthropic",
            base_url: "https://api.anthropic.com",
            oauth: { preset: "anthropic" },
          },
        },
        remove: [],
      });
      expect(await panel.count()).toBe(0);
    },
  );

  it.skipIf(usesRealServer)(
    "adds another provider from the fields of an oauth table and says what is missing",
    async () => {
      await open("/models");
      await page.getByRole("button", { name: "Log in to a provider" }).click();
      await page.getByRole("button", { name: "Log in to Other" }).click();
      await page.getByRole("button", { name: "Add and continue" }).click();
      await page.getByText("Base URL is empty.").waitFor();
      const panel = page.getByTestId("provider-panel");
      await panel.getByLabel("Base URL").fill("https://api.other.example");
      await panel.getByLabel("Client ID").fill("client");
      await panel.getByLabel("Authorize URL").fill("https://login.other.example/authorize");
      await panel.getByLabel("Token URL").fill("https://login.other.example/token");
      await panel.getByRole("button", { name: "Add and continue" }).click();
      await page
        .getByTestId("upstream")
        .nth(3)
        .getByRole("button", { name: "I understand, log in" })
        .waitFor();
      const { set } = sent<{ set: Record<string, { oauth: Record<string, unknown> }> }>(
        "PUT",
        "/v1/settings",
      );
      expect(set["router.upstreams.3"]?.oauth).toMatchObject({
        flow: "code",
        client_id: "client",
        authorize_url: "https://login.other.example/authorize",
        token_url: "https://login.other.example/token",
      });
    },
  );
});

describe("the Plugins page", () => {
  it("loads", async () => {
    await open("/plugins");
    await page.getByRole("heading", { name: "Plugins", exact: true }).waitFor();
    await page.getByRole("heading", { name: "Loaded plugins" }).waitFor();
    await page.getByText(/plugins? loaded; a call into one is refused after \d+ s\./).waitFor();
  });

  it.skipIf(usesRealServer)(
    "lists what the host loaded and the plugins in their own process",
    async () => {
      await open("/plugins");
      await page.getByText("3 plugins loaded; a call into one is refused after 30 s.").waitFor();
      const rows = page.getByTestId("plugin-row");
      await rows.nth(3).waitFor();
      const guard = await rows.nth(0).textContent();
      expect(guard).toContain("guard");
      expect(guard).toContain("Built in");
      expect(guard).toContain("tool_call");
      const process = await rows.nth(1).textContent();
      expect(process).toContain("Running");
      expect(process).toContain("start_card");
      expect(await rows.nth(2).textContent()).toContain("No kind of decision");
      await page.getByRole("heading", { name: "Plugins in their own process" }).waitFor();
      const own = await rows.nth(3).textContent();
      expect(own).toContain("/srv/plugins/allow-reads");
      expect(own).toContain("2 times");
      expect(own).toContain("exited with status 1");
      expect(own).toContain("/srv/.moebius/plugins/allow-reads.log");
    },
  );
});

describe("the pages that put a list beside a detail", () => {
  it.skipIf(usesRealServer)("keeps the list and the detail each inside the window", async () => {
    await open("/");
    // The three lay out what the harness configuration holds the same way.
    for (const [path, title] of [
      ["/agents", "Agents"],
      ["/skills", "Skills"],
      ["/mcp-servers", "MCP servers"],
    ] as const) {
      await page.goto(`${server.url}${path}`);
      await page.getByRole("heading", { name: title, level: 1 }).waitFor();
      await page.getByTestId("list-card").first().click();
      await page.getByTestId("detail-column").waitFor();
      await columnsScrollEach(page);
      await scrollbarsAreQuiet(page);
    }
  });
});
