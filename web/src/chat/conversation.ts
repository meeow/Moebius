/**
 * A chat's run events folded into what the chat shows: the agent's text and thinking, its tool calls with the check's
 * decision, the person's messages and the errors, in order. It is the logic of the desktop app's `Conversation`
 * (`crates/moebius-desktop/src/widgets/conversation.rs`), with the errors of the model added: a provider that refuses
 * the key, runs out of quota or answers an HTTP error ends Pi's assistant message with `stopReason: "error"`, which the
 * server passes on as a `message_end` event (a Claude Code run ends with an `agent_end` that has `is_error`).
 *
 * No React here. Events go in one by one (the stream) or as a page, in any order, and an event seen twice counts once,
 * so both ways give the same conversation. An entry that changes is replaced by a new object, so a view can skip the
 * entries that kept theirs.
 */
import type { RunEvent } from "../api/types/chat";
import { fileName } from "../ui/lib/format";

/** What a tool call works on, shown as one line — a file by its name, with `inputPath` for the whole path. */
export type ToolInput =
  | { type: "unknown" }
  | { type: "read"; path: string }
  | { type: "edit"; path: string; changes: Array<{ oldText: string; newText: string }> }
  | { type: "write"; path: string; content: string }
  | { type: "command"; command: string }
  | { type: "todos"; todos: Array<{ content: string; status: string }> }
  | { type: "other"; json: string };

/** The answer of the tool-call check (or of the MCP gateway). */
export interface Check {
  allowed: boolean;
  reason: string;
}

export interface McpPart {
  server: string;
  tool: string;
  check: Check;
}

export type Execution = { state: "running" } | { state: "finished"; failed: boolean };

export interface ToolCall {
  id: string;
  tool: string;
  input: ToolInput;
  check: Check | null;
  mcp: McpPart | null;
  /** Null while only the check has been seen. */
  execution: Execution | null;
  /** What the tool printed: so far while it runs, all of it when it ended. */
  output: string;
}

export type Said = "said" | "steered";

/** A picture the person sent with their message. The server keeps its bytes and serves them again, so the picture is
 * still there when the chat is read back. */
export interface SaidPicture {
  /** The name the server keeps the bytes under, which is the same picture twice in one message only once. */
  key: string;
  name: string | null;
  mediaType: string;
  src: string;
}

/** Where an error comes from: the model's provider (refused key, quota, an HTTP error), or Moebius, which ended the run. */
export type ErrorSource = "model" | "run";

/**
 * One thing the chat shows. `at` is when it arrived and `seq` the event that started it: a page that reads older
 * messages puts them above the ones drawn, so nothing may be named by its place in the list, or every turn would
 * change its name whenever something was added above it.
 */
export type Entry = { at: number; seq: number } & (
  | { kind: "text"; text: string }
  | { kind: "thinking"; text: string }
  | { kind: "tool"; call: ToolCall }
  | { kind: "person"; said: Said; message: string; pictures: SaidPicture[] }
  | { kind: "error"; source: ErrorSource; message: string }
  | { kind: "mcpError"; server: string; message: string }
);

type Json = Record<string, unknown>;

const object = (value: unknown): Json =>
  typeof value === "object" && value !== null && !Array.isArray(value) ? (value as Json) : {};
const string = (value: unknown): string => (typeof value === "string" ? value : "");

/** The pictures a `said` event carries. The server keeps each one under the chat's own directory and serves it from
 * there, so the message is shown with the same picture it was written with. */
export function pictureParts(raw: Json, runId: number): SaidPicture[] {
  const list = Array.isArray(raw.images) ? (raw.images as unknown[]) : [];
  const pictures: SaidPicture[] = [];
  for (const value of list) {
    const image = object(value);
    // The server names each picture after its bytes, so the same picture sent twice is one picture shown.
    const file = string(image.file).split("/").pop() ?? "";
    if (file === "" || pictures.some((each) => each.key === file)) continue;
    pictures.push({
      key: file,
      name: typeof image.name === "string" ? image.name : null,
      mediaType: string(image.media_type) || "an image",
      // The name holds no directory, so the address cannot be led out of the chat's own images.
      src: `/v1/chats/${runId}/images/${file}`,
    });
  }
  return pictures;
}

/** The input of a tool call from its arguments; Pi's tool names are lower case, Claude Code's are capitalised. */
export function toolInput(tool: string, args: unknown): ToolInput {
  const a = object(args);
  const path = string(a.path) || string(a.file_path);
  switch (tool.toLowerCase()) {
    case "read":
      return { type: "read", path };
    case "write":
      return { type: "write", path, content: string(a.content) };
    case "bash":
      return { type: "command", command: string(a.command) };
    case "edit": {
      // Pi's edit takes `edits`, and still the single `oldText` and `newText` of its older form; Claude Code's takes
      // `old_string` and `new_string`.
      const changes = Array.isArray(a.edits)
        ? a.edits.map((edit) => ({
            oldText: string(object(edit).oldText),
            newText: string(object(edit).newText),
          }))
        : [];
      if (typeof a.oldText === "string") {
        changes.push({ oldText: a.oldText, newText: string(a.newText) });
      }
      if (typeof a.old_string === "string") {
        changes.push({ oldText: a.old_string, newText: string(a.new_string) });
      }
      return { type: "edit", path, changes };
    }
    case "todowrite": {
      if (!Array.isArray(a.todos)) return { type: "other", json: JSON.stringify(a) };
      return {
        type: "todos",
        todos: a.todos.map((todo) => ({
          content: string(object(todo).content),
          status: string(object(todo).status),
        })),
      };
    }
    default:
      return args === null || args === undefined
        ? { type: "unknown" }
        : { type: "other", json: JSON.stringify(args) };
  }
}

/** The input in one line, as the pill of a tool call shows it. */
export function inputSummary(input: ToolInput): string {
  switch (input.type) {
    case "unknown":
      return "";
    case "read":
    case "edit":
    case "write":
      return fileName(input.path);
    case "command":
      return input.command.split("\n")[0] ?? "";
    case "todos":
      return `${input.todos.length} items`;
    case "other":
      return input.json;
  }
}

/**
 * The whole path a file call names, for the tooltip of a row that shows only the file's name; null for the calls
 * that are not about one file.
 */
export function inputPath(input: ToolInput): string | null {
  switch (input.type) {
    case "read":
    case "edit":
    case "write":
      return input.path;
    default:
      return null;
  }
}

/** The decision the call is shown with: a refusal of the check or the gateway, else the check's answer, else the gateway's. */
export function verdict(call: ToolCall): Check | null {
  const checks = [call.check, call.mcp?.check ?? null].filter((check) => check !== null);
  return checks.find((check) => !check.allowed) ?? checks[0] ?? null;
}

export function refused(call: ToolCall): boolean {
  return verdict(call)?.allowed === false;
}

/** The name the call is shown under: `<server> · <tool>` for a call of an MCP server, else the tool's. */
export function toolTitle(call: ToolCall): string {
  return call.mcp ? `${call.mcp.server} · ${call.mcp.tool}` : call.tool;
}

/** The text of a tool's result: the text blocks of its content, joined. */
function resultText(result: unknown): string {
  const content = object(result).content;
  if (!Array.isArray(content)) return "";
  return content
    .map((block) => object(block).text)
    .filter((text): text is string => typeof text === "string")
    .join("\n");
}

/** The text of the last assistant message of an `agent_end`, which says why a Claude Code run ended in error. */
function endText(data: Json): string {
  const messages = Array.isArray(data.messages) ? data.messages : [];
  const content = object(messages.at(-1)).content;
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .map((block) => string(object(block).text))
    .filter((text) => text !== "")
    .join("\n");
}

const MODEL_ERROR = "The model returned an error.";

/**
 * What a model's error says, as a person reads it. A gateway wraps its own sentence in JSON — `404: {"message":"no
 * route for POST …","type":"not_found_error",…}` — and the only part worth reading is the sentence, so it comes out
 * with whatever stood in front of the braces. Anything that is not that shape is left exactly as it came.
 */
export function readableError(text: string): string {
  const start = text.indexOf("{");
  if (start === -1 || !text.trimEnd().endsWith("}")) return text;
  let body: unknown;
  try {
    body = JSON.parse(text.slice(start));
  } catch {
    return text;
  }
  const inner = object(object(body).error).message;
  const sentence = string(object(body).message) || string(inner);
  if (sentence === "") return text;
  const lead = text.slice(0, start).trim();
  return lead === "" ? sentence : `${lead} ${sentence}`;
}

/** The error of an assistant message that the provider ended with an error; null for any other message. */
function modelErrorOf(message: Json): string | null {
  if (message.role !== "assistant" || message.stopReason !== "error") return null;
  return string(message.errorMessage) || MODEL_ERROR;
}

export class Conversation {
  private events = new Map<number, RunEvent>();
  private list: Entry[] = [];
  private calls = new Map<string, number>();
  private textOpen = false;
  private thinkingOpen = false;
  private now = 0;
  /** The `seq` of the event being folded; what an entry drawn from it is named by. */
  private seq = 0;
  private newest = 0;
  private oldest = 0;
  /** The error of the model that the turn showed already, so that `agent_end`, which repeats it, adds none. */
  private modelError: string | null = null;

  /** The chat whose events are folded: its pictures are served from its own directory. */
  private readonly run: number;

  constructor(run = 0) {
    this.run = run;
  }

  /** The highest `seq` seen; 0 when none. */
  get lastSeq(): number {
    return this.newest;
  }

  /** The lowest `seq` held; 0 when none. What a page of older events is asked to stop below. */
  get oldestSeq(): number {
    return this.oldest;
  }

  /** The time of the newest event, Unix seconds; 0 when none. */
  get lastAt(): number {
    let at = 0;
    for (const event of this.events.values()) at = Math.max(at, event.at);
    return at;
  }

  /**
   * The entries, in order. The array is new on every call. An entry is the same object as before until an event lands
   * among those already folded and the log is folded again; the entries are then new objects, equal to the old ones
   * where nothing changed, and nothing may rely on their identity.
   */
  entries(): Entry[] {
    return this.list.slice();
  }

  /**
   * Adds one event as it arrives on the stream. One seen before is ignored; one older than the newest seen sits
   * between entries already drawn, so the fold is done again and it takes its place. That is the only time a single
   * event costs more than itself, and it is rare: what arrives while a chat is open is nearly always the next one.
   */
  push(event: RunEvent): void {
    if (this.events.has(event.seq)) return;
    this.hold(event);
    if (event.seq <= this.newest) {
      this.rebuild();
      return;
    }
    this.apply(event);
    this.newest = event.seq;
  }

  /**
   * Adds the events read from the server, which come as a page of them at a time. Reading what was missed and being
   * sent what happens now are two different paths that meet in one conversation, so a page often arrives holding
   * events older than the newest one the stream already brought. Each of those is put in its place once, for the
   * whole page: folding the log again per event would cost the log again per event, which is seconds on a long chat
   * that is still answering.
   *
   * The page also carries the server's time on events the stream already brought with this browser's clock on them,
   * and a duration is worked out between two `at`s, so the stored time is the one kept.
   */
  backfill(events: readonly RunEvent[]): void {
    const fresh: RunEvent[] = [];
    let again = false;
    for (const event of events) {
      const held = this.events.get(event.seq);
      if (held) {
        if (held.at !== event.at) {
          this.events.set(event.seq, event);
          again = true;
        }
        continue;
      }
      this.hold(event);
      fresh.push(event);
      if (event.seq <= this.newest) again = true;
    }
    if (again) {
      this.rebuild();
      return;
    }
    // Nothing landed out of order and no time needed correcting, so the page is simply more of the chat: its events
    // follow the last one drawn and are applied in their own order.
    for (const event of fresh.sort((a, b) => a.seq - b.seq)) {
      this.apply(event);
      this.newest = event.seq;
    }
  }

  /** Keeps a new event, and the lowest `seq` with it, so that asking for it costs nothing. */
  private hold(event: RunEvent): void {
    this.events.set(event.seq, event);
    if (this.oldest === 0 || event.seq < this.oldest) this.oldest = event.seq;
  }

  /** Draws every event held again, in the order their `seq`s give them. */
  private rebuild(): void {
    const sorted = [...this.events.values()].sort((a, b) => a.seq - b.seq);
    this.list = [];
    this.calls = new Map();
    this.textOpen = this.thinkingOpen = false;
    this.modelError = null;
    this.newest = 0;
    for (const event of sorted) {
      this.apply(event);
      this.newest = event.seq;
    }
  }

  private addModelError(raw: string): void {
    const message = readableError(raw);
    if (this.modelError === message) return;
    this.modelError = message;
    this.add({ at: this.now, seq: this.seq, kind: "error", source: "model", message });
  }

  private add(entry: Entry): number {
    this.list.push(entry);
    return this.list.length - 1;
  }

  private close(): void {
    this.textOpen = this.thinkingOpen = false;
  }

  /** The tool call with this id, made when it is first seen; calls without an id are always new. */
  private call(id: string, tool: string): [number, ToolCall] {
    let index = id === "" ? undefined : this.calls.get(id);
    if (index === undefined) {
      index = this.add({
        at: this.now,
        seq: this.seq,
        kind: "tool",
        call: {
          id,
          tool,
          input: { type: "unknown" },
          check: null,
          mcp: null,
          execution: null,
          output: "",
        },
      });
      this.calls.set(id, index);
    }
    this.close();
    return [index, (this.list[index] as Extract<Entry, { kind: "tool" }>).call];
  }

  private setCall(index: number, call: ToolCall): void {
    const entry = this.list[index] as Extract<Entry, { kind: "tool" }>;
    this.list[index] = { ...entry, call };
  }

  /** Appends `delta` to the open entry of `kind`, or starts one. */
  private stream(kind: "text" | "thinking", delta: string): void {
    const last = this.list.at(-1);
    const open = kind === "text" ? this.textOpen : this.thinkingOpen;
    if (last && last.kind === kind && open) {
      this.list[this.list.length - 1] = { ...last, text: last.text + delta };
    } else {
      this.add({ at: this.now, seq: this.seq, kind, text: delta });
    }
    this.close();
    if (kind === "text") this.textOpen = true;
    else this.thinkingOpen = true;
  }

  private apply(event: RunEvent): void {
    this.now = event.at;
    this.seq = event.seq;
    const data = event.data;
    const id = string(data.toolCallId);
    const tool = string(data.toolName);
    switch (event.kind) {
      case "message_update": {
        const update = object(data.assistantMessageEvent);
        if (update.type === "text_delta" && typeof update.delta === "string") {
          this.stream("text", update.delta);
        }
        if (update.type === "thinking_delta" && typeof update.delta === "string") {
          this.stream("thinking", update.delta);
        }
        break;
      }
      case "message_start":
        this.close();
        break;
      case "message_end": {
        this.close();
        const error = modelErrorOf(object(data.message));
        if (error !== null) this.addModelError(error);
        break;
      }
      case "agent_start":
        this.modelError = null;
        break;
      case "agent_end": {
        this.close();
        // Pi's `agent_end` holds the messages of the turn, the failed one last; a chat's terminal records it alone.
        const messages = Array.isArray(data.messages) ? data.messages : [];
        const error = modelErrorOf(object(messages.at(-1)));
        if (error !== null) {
          this.addModelError(error);
        } else if (data.is_error === true) {
          // A Claude Code run ends with `is_error` and the text of its last message.
          this.addModelError(endText(data) || MODEL_ERROR);
        }
        break;
      }
      case "tool_execution_start": {
        const [index, call] = this.call(id, tool);
        this.setCall(index, {
          ...call,
          tool,
          input: toolInput(tool, data.args),
          execution: { state: "running" },
        });
        break;
      }
      case "tool_execution_update": {
        const [index, call] = this.call(id, tool);
        this.setCall(index, { ...call, output: resultText(data.partialResult) });
        break;
      }
      case "tool_execution_end": {
        const [index, call] = this.call(id, tool);
        this.setCall(index, {
          ...call,
          output: resultText(data.result),
          execution: { state: "finished", failed: data.isError === true },
        });
        break;
      }
      case "tool_check": {
        const [index, call] = this.call(string(data.tool_call_id), string(data.tool));
        this.setCall(index, {
          ...call,
          check: { allowed: data.decision === "allow", reason: string(data.reason) },
        });
        break;
      }
      case "mcp_call":
        this.mcpCall(data);
        break;
      case "mcp_error":
        this.add({
          at: this.now,
          seq: this.seq,
          kind: "mcpError",
          server: string(data.server),
          message: string(data.message),
        });
        this.close();
        break;
      case "steered":
      case "said":
        this.modelError = null;
        this.add({
          at: this.now,
          seq: this.seq,
          kind: "person",
          said: event.kind === "steered" ? "steered" : "said",
          message: string(data.message),
          pictures: pictureParts(data, this.run),
        });
        this.close();
        break;
      case "moebius_error":
        this.add({
          at: this.now,
          seq: this.seq,
          kind: "error",
          source: "run",
          message: string(data.message),
        });
        this.close();
        break;
    }
  }

  /**
   * A call through the MCP gateway. With Pi-Bolt the harness has the call under its own name, `mcp__<server>__<tool>`:
   * the gateway's part goes into that call (the one with the same id, else the latest of that name without a
   * gateway part yet) so that it is drawn once; else it is a call of its own.
   */
  private mcpCall(data: Json): void {
    const server = string(data.server);
    const tool = string(data.tool);
    const id = string(data.tool_call_id);
    const ownName = `__${server}__${tool}`;
    const isTool = (entry: Entry): entry is Extract<Entry, { kind: "tool" }> =>
      entry.kind === "tool";
    let index: number | undefined;
    const known = id === "" ? undefined : this.calls.get(id);
    if (known !== undefined) {
      const entry = this.list[known];
      if (entry && isTool(entry) && entry.call.mcp === null) index = known;
    }
    if (index === undefined) {
      for (let i = this.list.length - 1; i >= 0; i--) {
        const entry = this.list[i];
        if (
          entry &&
          isTool(entry) &&
          entry.call.mcp === null &&
          entry.call.tool.startsWith("mcp__") &&
          entry.call.tool.endsWith(ownName)
        ) {
          index = i;
          break;
        }
      }
    }
    if (index === undefined) {
      index = this.add({
        at: this.now,
        seq: this.seq,
        kind: "tool",
        call: {
          id,
          tool: `mcp__${server}__${tool}`,
          input:
            data.input === null || data.input === undefined
              ? { type: "unknown" }
              : { type: "other", json: JSON.stringify(data.input) },
          check: null,
          mcp: null,
          execution: null,
          output: "",
        },
      });
      if (id !== "") this.calls.set(id, index);
    }
    this.close();
    const call = (this.list[index] as Extract<Entry, { kind: "tool" }>).call;
    const result = resultText(data.result);
    this.setCall(index, {
      ...call,
      execution: call.execution ?? {
        state: "finished",
        failed: object(data.result).isError === true,
      },
      output: call.output === "" ? result : call.output,
      mcp: {
        server,
        tool,
        check: { allowed: data.decision === "allow", reason: string(data.reason) },
      },
    });
  }
}
